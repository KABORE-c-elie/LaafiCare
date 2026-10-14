import 'package:flutter/foundation.dart';

import '../../domain/models/erreur_api.dart';
import '../services/client_api.dart';
import '../services/stockage_jeton.dart';

/// État de la session patient.
enum EtatSession {
  /// Le stockage n'a pas encore été lu (démarrage).
  inconnu,
  connecte,
  deconnecte,
}

/// Formulaire d'inscription (docs/api.md §2.2.2).
///
/// C'est l'écran qui le garde, pas [AuthRepository] : il resservira si la
/// réponse se perd et que le compte n'a pas été créé. L'écran doit
/// l'effacer, mot de passe compris, dès que l'inscription est confirmée ou
/// abandonnée (décision du porteur).
class DonneesInscription {
  const DonneesInscription({
    required this.telephone,
    required this.codeOtp,
    required this.nom,
    required this.prenom,
    required this.dateNaissance,
    required this.lieuNaissance,
    required this.motDePasse,
  });

  final String telephone;
  final String codeOtp;
  final String nom;
  final String prenom;
  final DateTime dateNaissance;
  final String lieuNaissance;
  final String motDePasse;

  Map<String, Object?> versJson() => {
        'telephone': telephone,
        'code_otp': codeOtp,
        'nom': nom,
        'prenom': prenom,
        // docs/api.md §1.1 : dates au format AAAA-MM-JJ.
        'date_naissance': '${dateNaissance.year.toString().padLeft(4, '0')}-'
            '${dateNaissance.month.toString().padLeft(2, '0')}-'
            '${dateNaissance.day.toString().padLeft(2, '0')}',
        'lieu_naissance': lieuNaissance,
        'mot_de_passe': motDePasse,
      };
}

/// La réponse de l'inscription s'est perdue, et la connexion de contrôle
/// répond `identifiants_invalides` : le compte n'a pas été créé. Reprendre
/// l'inscription : nouveau code, puis nouvel envoi du formulaire gardé
/// (docs/api.md §2.2.2).
class InscriptionNonCreee implements Exception {
  const InscriptionNonCreee();
}

/// La réponse de l'inscription s'est perdue, et la connexion de contrôle n'a
/// pas pu joindre le serveur : on ne sait pas si le compte existe.
/// Réessayer plus tard avec [AuthRepository.confirmerInscription], **jamais**
/// en renvoyant l'inscription (docs/api.md §2.2.2).
class InscriptionNonConfirmee implements Exception {
  const InscriptionNonConfirmee();
}

/// Authentification patient : les quatre routes de docs/api.md §2.2, et
/// l'état de la session pour la navigation.
class AuthRepository extends ChangeNotifier {
  // Paramètres nommés privés (Dart ≥ 3.12) : l'appelant écrit client:,
  // stockage:. https://dart.dev/language/constructors
  AuthRepository({required this._client, required this._stockage});

  final ClientApi _client;
  final StockageJeton _stockage;

  static const _cheminOtp = '/api/patients/otp';
  static const _cheminInscription = '/api/patients/inscription';
  static const _cheminConnexion = '/api/patients/connexion';
  static const _cheminReinitialisation =
      '/api/patients/mot-de-passe/reinitialiser';

  /// Réponses d'un intermédiaire réseau (passerelle, répartiteur) à
  /// l'inscription : 502, 503, 504. Le serveur a peut-être créé le compte.
  /// Choix du porteur, en attendant que docs/api.md décrive ce cas : elles
  /// sont traitées comme une réponse perdue, et la connexion de contrôle est
  /// lancée. Sans risque : cette connexion ne crée rien, elle vérifie.
  static const _statutsIntermediaire = {502, 503, 504};

  EtatSession _etat = EtatSession.inconnu;
  EtatSession get etat => _etat;

  /// Au démarrage : une session existe si un jeton est enregistré. Le jeton
  /// n'est pas lu ni vérifié ici (docs/api.md §1.2 : l'application ne lit
  /// pas son contenu) ; s'il a expiré, la première route protégée répondra
  /// 401, et la session sera fermée.
  Future<void> initialiser() async {
    final jeton = await _stockage.lireJetonAcces();
    _changerEtat(jeton == null ? EtatSession.deconnecte : EtatSession.connecte);
  }

  /// Envoie un code par SMS (docs/api.md §2.2.1). Un 429 remonte en
  /// [ErreurApi] avec son délai `attente` (§2.2.5).
  Future<void> demanderCode(String telephone) async {
    await _client.post(_cheminOtp, corps: {'telephone': telephone});
  }

  /// Connexion (docs/api.md §2.2.3).
  Future<void> connecter({
    required String telephone,
    required String motDePasse,
  }) async {
    final reponse = await _client.post(_cheminConnexion,
        corps: {'telephone': telephone, 'mot_de_passe': motDePasse});
    await _ouvrirSession(reponse);
  }

  /// Inscription (docs/api.md §2.2.2). Si la réponse se perd (erreur
  /// réseau, délai dépassé, ou 502/503/504 d'un intermédiaire),
  /// l'inscription n'est jamais renvoyée : une connexion avec le même
  /// téléphone et le même mot de passe vérifie si le compte a été créé
  /// (voir [confirmerInscription]).
  Future<void> inscrire(DonneesInscription donnees) async {
    final ReponseApi reponse;
    try {
      reponse =
          await _client.post(_cheminInscription, corps: donnees.versJson());
    } on ErreurReseau {
      await confirmerInscription(
        telephone: donnees.telephone,
        motDePasse: donnees.motDePasse,
      );
      return;
    } on ErreurApi catch (erreur) {
      if (!_statutsIntermediaire.contains(erreur.statut)) rethrow;
      await confirmerInscription(
        telephone: donnees.telephone,
        motDePasse: donnees.motDePasse,
      );
      return;
    }
    await _ouvrirSession(reponse);
  }

  /// Connexion de contrôle après une inscription dont la réponse s'est
  /// perdue (tableau de docs/api.md §2.2.2) :
  /// - 200 : le compte existe, la session est ouverte ;
  /// - 401 identifiants_invalides : [InscriptionNonCreee] ;
  /// - erreur réseau : [InscriptionNonConfirmee], réessayer plus tard.
  Future<void> confirmerInscription({
    required String telephone,
    required String motDePasse,
  }) async {
    try {
      await connecter(telephone: telephone, motDePasse: motDePasse);
    } on ErreurApi catch (erreur) {
      if (erreur.code == CodesErreur.identifiantsInvalides) {
        throw const InscriptionNonCreee();
      }
      rethrow;
    } on ErreurReseau {
      throw const InscriptionNonConfirmee();
    }
  }

  /// Mot de passe oublié (docs/api.md §2.2.4) : aucun jeton n'est renvoyé,
  /// la personne se connecte ensuite avec son nouveau mot de passe.
  Future<void> reinitialiserMotDePasse({
    required String telephone,
    required String codeOtp,
    required String nouveauMotDePasse,
  }) async {
    await _client.post(_cheminReinitialisation, corps: {
      'telephone': telephone,
      'code_otp': codeOtp,
      'nouveau_mot_de_passe': nouveauMotDePasse,
    });
  }

  /// Déconnexion, ou fin de session après un 401 (rappel de [ClientApi]) :
  /// décision du porteur, le jeton est effacé sur le téléphone. Aucune
  /// route de déconnexion n'est encore en service (docs/api.md §1.7, en
  /// cours).
  Future<void> deconnecter() async {
    await _stockage.effacer();
    _changerEtat(EtatSession.deconnecte);
  }

  /// **Seul endroit** qui lit la réponse d'ouverture de session (connexion
  /// et inscription). Aujourd'hui : `{"jeton": "<JWT>"}` (docs/api.md §2.2.2,
  /// §2.2.3). Quand docs/api.md §1.7 sera en service, cette méthode lira
  /// {"etape", "jeton_acces", "expire_dans", "jeton_renouvellement"} ou
  /// {"etape": "second_facteur", …} ; rien d'autre ne changera dans
  /// l'application.
  Future<void> _ouvrirSession(ReponseApi reponse) async {
    final jeton = reponse.json?['jeton'];
    if (jeton is! String || jeton.isEmpty) {
      throw ErreurApi(
        statut: reponse.statut,
        code: CodesErreur.reponseIllisible,
      );
    }
    await _stockage.enregistrerJetonAcces(jeton);
    _changerEtat(EtatSession.connecte);
  }

  void _changerEtat(EtatSession etat) {
    if (_etat == etat) return;
    _etat = etat;
    notifyListeners();
  }
}
