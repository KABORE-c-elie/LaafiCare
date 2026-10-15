import 'package:flutter/foundation.dart';

import '../../domain/models/erreur_api.dart';
import '../../domain/models/paire_jetons.dart';
import '../services/client_api.dart';
import '../services/gestionnaire_session.dart';

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
/// l'inscription : nouveau code, puis nouvel envoi du formulaire gardé, avec
/// `relance: true` (docs/api.md §2.2.2).
class InscriptionNonCreee implements Exception {
  const InscriptionNonCreee();
}

/// La réponse de l'inscription s'est perdue, et la connexion de contrôle n'a
/// pas pu joindre le serveur après 4 essais : on ne sait pas si le compte
/// existe. Réessayer plus tard avec [AuthRepository.confirmerInscription],
/// **jamais** en renvoyant l'inscription (docs/api.md §2.2.2). Le formulaire
/// reste gardé par l'écran.
class InscriptionNonConfirmee implements Exception {
  const InscriptionNonConfirmee();
}

/// La relance de l'inscription a répondu 409, puis la connexion de contrôle
/// encore 401 : le numéro appartient à un autre compte, ou ce compte a un
/// autre mot de passe. Le parcours s'arrête ; l'écran affiche le message
/// normal « ce numéro a déjà un compte », avec la connexion et « Mot de
/// passe oublié ? » (docs/api.md §2.2.2, « Sortie de boucle »).
class NumeroDejaInscrit implements Exception {
  const NumeroDejaInscrit();
}

/// Authentification patient : les routes de docs/api.md §2.2, la session
/// (§1.7, par le gestionnaire de session) et l'état de la session pour la
/// navigation.
class AuthRepository extends ChangeNotifier {
  // Paramètres nommés privés (Dart ≥ 3.12) : l'appelant écrit client:,
  // session:, attendre:. https://dart.dev/language/constructors
  AuthRepository({
    required this._client,
    required this._session,
    this._attendre = _attendreParDefaut,
  }) {
    // Une session fermée ailleurs (renouvellement refusé, rejeu refusé)
    // ramène la navigation à la connexion.
    _session.surSessionFermee = () => _changerEtat(EtatSession.deconnecte);
  }

  final ClientApi _client;
  final GestionnaireSession _session;

  /// Attente entre deux connexions de contrôle ; remplaçable dans les tests
  /// (sinon 22 s réelles).
  final Future<void> Function(Duration) _attendre;

  static Future<void> _attendreParDefaut(Duration duree) =>
      Future<void>.delayed(duree);

  static const _cheminOtp = '/api/patients/otp';
  static const _cheminInscription = '/api/patients/inscription';
  static const _cheminConnexion = '/api/patients/connexion';
  static const _cheminReinitialisation =
      '/api/patients/mot-de-passe/reinitialiser';

  /// Réponses d'un intermédiaire réseau (passerelle, proxy, répartiteur) :
  /// la demande a peut-être été traitée, la réponse est traitée comme
  /// perdue (docs/api.md §2.2.2).
  static const _statutsIntermediaire = {502, 503, 504};

  /// Attentes entre les essais de la connexion de contrôle : 1 essai, puis
  /// au plus 3 nouveaux essais (docs/api.md §2.2.2 : « au plus 3 fois,
  /// espacées (par exemple après 2 s, 5 s, puis 15 s) »). Valeurs reprises
  /// de l'exemple, choix du porteur.
  static const _attentesControle = [
    Duration(seconds: 2),
    Duration(seconds: 5),
    Duration(seconds: 15),
  ];

  EtatSession _etat = EtatSession.inconnu;
  EtatSession get etat => _etat;

  /// Au démarrage : une session existe si un jeton d'accès est enregistré.
  /// Le jeton n'est pas lu ni vérifié ici (docs/api.md §1.2 : l'application
  /// ne lit pas son contenu) ; s'il a expiré, la première route protégée
  /// répondra 401, et le gestionnaire de session renouvellera.
  ///
  /// Une erreur imprévue pendant cette lecture mène à l'état « déconnecté »
  /// (décision du porteur : l'application ne doit jamais rester bloquée sur
  /// l'écran de démarrage). L'erreur n'est pas cachée : elle est signalée par
  /// FlutterError.reportError, comme une erreur du framework.
  Future<void> initialiser() async {
    try {
      _changerEtat(await _session.estOuverte()
          ? EtatSession.connecte
          : EtatSession.deconnecte);
    } catch (erreur, pile) {
      FlutterError.reportError(FlutterErrorDetails(
        exception: erreur,
        stack: pile,
        library: 'LaafiCare',
        context: ErrorDescription('pendant la lecture de la session'),
      ));
      _changerEtat(EtatSession.deconnecte);
    }
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

  /// Inscription (docs/api.md §2.2.2). [relance] : vrai quand l'écran
  /// renvoie le formulaire après [InscriptionNonCreee].
  ///
  /// - Réponse perdue (erreur réseau, délai, 502/503/504) : jamais de second
  ///   envoi, connexion de contrôle ([confirmerInscription]).
  /// - Relance qui répond 409 `telephone_deja_utilise` : la première
  ///   inscription avait abouti ; connexion de contrôle sans erreur
  ///   affichée, et [NumeroDejaInscrit] si elle répond encore 401.
  /// - Première inscription qui répond 409 : l'erreur remonte telle quelle.
  Future<void> inscrire(
    DonneesInscription donnees, {
    bool relance = false,
  }) async {
    final ReponseApi reponse;
    try {
      reponse =
          await _client.post(_cheminInscription, corps: donnees.versJson());
    } on ErreurReseau {
      return confirmerInscription(
          telephone: donnees.telephone, motDePasse: donnees.motDePasse);
    } on ErreurApi catch (erreur) {
      if (_statutsIntermediaire.contains(erreur.statut)) {
        return confirmerInscription(
            telephone: donnees.telephone, motDePasse: donnees.motDePasse);
      }
      if (relance && erreur.code == CodesErreur.telephoneDejaUtilise) {
        try {
          return await confirmerInscription(
              telephone: donnees.telephone, motDePasse: donnees.motDePasse);
        } on InscriptionNonCreee {
          throw const NumeroDejaInscrit();
        }
      }
      rethrow;
    }
    await _ouvrirSession(reponse);
  }

  /// Connexion de contrôle (tableau de docs/api.md §2.2.2) :
  /// - 200 : le compte existe, la session est ouverte ;
  /// - 401 identifiants_invalides : [InscriptionNonCreee] ;
  /// - erreur réseau ou 502/503/504 : nouvel essai après 2 s, 5 s, puis
  ///   15 s ; après le 4ᵉ échec, [InscriptionNonConfirmee] (le formulaire
  ///   reste gardé par l'écran, qui pourra rappeler cette méthode).
  ///
  /// Une [EtapeConnexionNonGeree] remonte telle quelle : le compte existe,
  /// mais demande une étape que l'application ne gère pas encore.
  Future<void> confirmerInscription({
    required String telephone,
    required String motDePasse,
  }) async {
    for (var essai = 0;; essai++) {
      try {
        await connecter(telephone: telephone, motDePasse: motDePasse);
        return;
      } on ErreurApi catch (erreur) {
        if (erreur.code == CodesErreur.identifiantsInvalides) {
          throw const InscriptionNonCreee();
        }
        if (!_statutsIntermediaire.contains(erreur.statut)) rethrow;
      } on ErreurReseau {
        // Nouvel essai ci-dessous.
      }
      if (essai == _attentesControle.length) {
        throw const InscriptionNonConfirmee();
      }
      await _attendre(_attentesControle[essai]);
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

  /// Déconnexion de cet appareil (docs/api.md §1.7) : jetons effacés même
  /// sans réseau ; l'état passe à « déconnecté » par le rappel du
  /// gestionnaire.
  Future<void> deconnecter() => _session.deconnecter();

  /// Déconnexion de tous les appareils : jetons effacés dans tous les cas ;
  /// une erreur remonte si les autres appareils n'ont peut-être pas été
  /// déconnectés.
  Future<void> deconnecterPartout() => _session.deconnecterPartout();

  /// **Seul endroit** qui lit la réponse d'ouverture de session (connexion
  /// et inscription), par PaireJetons.depuisOuverture : `etape` testée
  /// (EtapeConnexionNonGeree sinon), paire enregistrée avant tout le reste.
  Future<void> _ouvrirSession(ReponseApi reponse) async {
    final paire =
        PaireJetons.depuisOuverture(statut: reponse.statut, json: reponse.json);
    await _session.ouvrir(paire);
    _changerEtat(EtatSession.connecte);
  }

  void _changerEtat(EtatSession etat) {
    if (_etat == etat) return;
    _etat = etat;
    notifyListeners();
  }
}
