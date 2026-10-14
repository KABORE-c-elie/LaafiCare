import 'dart:async';

import '../../domain/models/erreur_api.dart';
import '../../domain/models/paire_jetons.dart';
import 'client_api.dart';
import 'stockage_jeton.dart';

/// La session est fermée (renouvellement refusé, ou requête rejouée encore
/// refusée) : la personne doit se reconnecter. Les jetons sont déjà effacés.
class SessionFermee implements Exception {
  const SessionFermee();
}

/// Session patient sur ce téléphone (docs/api.md §1.7) : ouverture,
/// requêtes protégées avec renouvellement, fermeture, déconnexions.
class GestionnaireSession {
  // Paramètres nommés privés (Dart ≥ 3.12) : l'appelant écrit client:,
  // stockage:, delaiRenouvellement:. https://dart.dev/language/constructors
  GestionnaireSession({
    required this._client,
    required this._stockage,
    this._delaiRenouvellement = delaiRenouvellementParDefaut,
  });

  final ClientApi _client;
  final StockageJeton _stockage;
  final Duration _delaiRenouvellement;

  /// Délai d'une tentative de renouvellement : choix du porteur (C2), pas
  /// une valeur officielle. Il doit rester bien sous 30 s : docs/api.md
  /// §1.7, « L'application doit relancer sa demande de renouvellement en
  /// moins de 30 secondes […] Son délai d'attente réseau doit être réglé en
  /// conséquence ». Avec 10 s, la seconde tentative part au plus tard vers
  /// 10 s.
  static const delaiRenouvellementParDefaut = Duration(seconds: 10);

  static const _cheminRenouveler = '/api/sessions/renouveler';
  static const _cheminDeconnexion = '/api/sessions/deconnexion';
  static const _cheminDeconnexionPartout = '/api/sessions/deconnexion-partout';

  /// Réponses d'un intermédiaire réseau : le serveur a peut-être déjà changé
  /// le jeton, la réponse est traitée comme perdue (choix du porteur, C9).
  static const _statutsIntermediaire = {502, 503, 504};

  /// Prévenu à chaque fermeture de la session (AuthRepository passe alors
  /// à l'état « déconnecté »).
  void Function()? surSessionFermee;

  /// Renouvellement en cours, partagé par toutes les requêtes qui reçoivent
  /// un 401 pendant ce temps (docs/api.md §1.7 : « ne jamais lancer deux
  /// renouvellements en même temps avec le même jeton »).
  Future<void>? _renouvellementEnCours;

  /// Augmenté à chaque fermeture (C10) : un renouvellement qui se termine
  /// après une fermeture n'enregistre pas sa paire, ou l'efface aussitôt.
  var _generation = 0;

  /// Vrai si un jeton d'accès est enregistré sur le téléphone.
  Future<bool> estOuverte() async => await _stockage.lireJetonAcces() != null;

  /// Connexion ou inscription réussie : la paire est enregistrée avant
  /// toute autre requête.
  Future<void> ouvrir(PaireJetons paire) => _stockage.enregistrerPaire(
        jetonAcces: paire.jetonAcces,
        jetonRenouvellement: paire.jetonRenouvellement,
      );

  /// Requête protégée. [envoyer] reçoit le jeton d'accès à présenter.
  ///
  /// Sur un 401 : un seul renouvellement (partagé), puis **un seul** rejeu
  /// (docs/api.md §1.7 : « renouveler une fois, puis rejouer la demande »).
  /// Si le rejeu est encore refusé, la session est fermée : jamais de
  /// boucle.
  Future<ReponseApi> requete(
      Future<ReponseApi> Function(String jetonAcces) envoyer) async {
    final jeton = await _stockage.lireJetonAcces();
    if (jeton == null) {
      await _fermer();
      throw const SessionFermee();
    }
    try {
      return await envoyer(jeton);
    } on ErreurApi catch (erreur) {
      if (!_jetonRefuse(erreur)) rethrow;
    }
    await _renouvelerSiNecessaire(jetonUtilise: jeton);
    final nouveau = await _stockage.lireJetonAcces();
    if (nouveau == null) throw const SessionFermee();
    try {
      return await envoyer(nouveau);
    } on ErreurApi catch (erreur) {
      if (!_jetonRefuse(erreur)) rethrow;
      await _fermer();
      throw const SessionFermee();
    }
  }

  /// Déconnexion de cet appareil : jetons effacés d'abord (C6), puis le
  /// serveur est prévenu sans attendre son succès (la route répond « toujours
  /// 204 », docs/api.md §1.7).
  Future<void> deconnecter() async {
    final jetonRenouvellement = await _stockage.lireJetonRenouvellement();
    await _fermer();
    if (jetonRenouvellement == null) return;
    try {
      await _client.post(_cheminDeconnexion,
          corps: {'jeton_renouvellement': jetonRenouvellement},
          delaiAttente: _delaiRenouvellement);
    } on ErreurApi {
      // Ignorée : les jetons sont déjà effacés de ce téléphone.
    } on ErreurReseau {
      // Idem : sans réseau, la session expirera d'elle-même côté serveur.
    }
  }

  /// Déconnexion de tous les appareils. Les jetons de ce téléphone sont
  /// effacés dans tous les cas ; si la requête échoue, l'erreur remonte
  /// pour que l'écran dise que les autres appareils ne sont peut-être pas
  /// déconnectés (C6).
  Future<void> deconnecterPartout() async {
    try {
      await requete(
          (jeton) => _client.post(_cheminDeconnexionPartout, jeton: jeton));
    } finally {
      await _fermer();
    }
  }

  bool _jetonRefuse(ErreurApi erreur) =>
      erreur.statut == 401 &&
      (erreur.code == CodesErreur.jetonInvalide ||
          erreur.code == CodesErreur.authentificationRequise);

  Future<void> _renouvelerSiNecessaire({required String jetonUtilise}) async {
    final enCours = _renouvellementEnCours;
    if (enCours != null) return enCours;
    // Requête envoyée avec un jeton déjà remplacé : rejouer sans renouveler.
    final actuel = await _stockage.lireJetonAcces();
    if (actuel != null && actuel != jetonUtilise) return;
    // Un autre appel a pu lancer le renouvellement pendant la lecture.
    final lanceEntreTemps = _renouvellementEnCours;
    if (lanceEntreTemps != null) return lanceEntreTemps;
    final renouvellement = _renouveler();
    _renouvellementEnCours = renouvellement;
    try {
      await renouvellement;
    } finally {
      _renouvellementEnCours = null;
    }
  }

  Future<void> _renouveler() async {
    final generation = _generation;
    final jetonRenouvellement = await _stockage.lireJetonRenouvellement();
    if (jetonRenouvellement == null) {
      await _fermer();
      throw const SessionFermee();
    }
    ReponseApi reponse;
    try {
      reponse = await _tenterRenouvellement(jetonRenouvellement);
    } on _ReponsePerdue {
      // Seconde et dernière tentative, avec le même jeton (C2). Le serveur
      // n'accepte qu'une seule réutilisation de l'ancien jeton, dans les
      // 30 s (docs/api.md §1.7) : jamais de troisième tentative.
      // Conséquence assumée : si l'une des deux tentatives a atteint le
      // serveur et que la personne réessaie après 30 s, le serveur voit un
      // rejeu et ferme la session (docs/api.md §1.7). Il faut se reconnecter,
      // et le serveur enregistre une fausse alerte de vol.
      try {
        reponse = await _tenterRenouvellement(jetonRenouvellement);
      } on _ReponsePerdue catch (perdue) {
        throw perdue.erreur; // jetons gardés, erreur remontée
      }
    }
    final paire = PaireJetons.depuisRenouvellement(
        statut: reponse.statut, json: reponse.json);
    if (generation != _generation) return; // fermée entre-temps (C10)
    await _stockage.enregistrerPaire(
      jetonAcces: paire.jetonAcces,
      jetonRenouvellement: paire.jetonRenouvellement,
    );
    // L'écriture se fait en deux étapes : une fermeture pendant l'écriture
    // laisserait un jeton seul après l'effacement. On efface de nouveau,
    // sans prévenir une seconde fois (correction du porteur, C10).
    if (generation != _generation) await _stockage.effacer();
  }

  /// Une tentative : sans en-tête Authorization (docs/api.md §1.7 : « sans
  /// en-tête Authorization (le jeton d'accès peut être expiré) »), avec le
  /// délai court. Réponse perdue = erreur réseau ou 502/503/504 (C9).
  /// 401 = session fermée par le serveur.
  Future<ReponseApi> _tenterRenouvellement(String jetonRenouvellement) async {
    try {
      return await _client.post(_cheminRenouveler,
          corps: {'jeton_renouvellement': jetonRenouvellement},
          delaiAttente: _delaiRenouvellement);
    } on ErreurReseau catch (erreur) {
      throw _ReponsePerdue(erreur);
    } on ErreurApi catch (erreur) {
      if (_statutsIntermediaire.contains(erreur.statut)) {
        throw _ReponsePerdue(erreur);
      }
      if (erreur.statut == 401) {
        await _fermer();
        throw const SessionFermee();
      }
      rethrow;
    }
  }

  Future<void> _fermer() async {
    _generation++;
    await _stockage.effacer();
    surSessionFermee?.call();
  }
}

/// Réponse d'un renouvellement perdue (usage interne).
class _ReponsePerdue implements Exception {
  const _ReponsePerdue(this.erreur);
  final Exception erreur;
}
