import 'package:flutter/foundation.dart';

import '../../data/repositories/auth_repository.dart';
import '../../data/services/client_api.dart';
import '../../domain/models/erreur_api.dart';
import '../../domain/models/paire_jetons.dart';

/// Erreurs possibles de l'écran de connexion. L'écran choisit le texte
/// (Textes) ; chaque valeur correspond à un code de docs/api.md §2.2.3, ou
/// à un cas propre à l'application.
enum ErreurConnexion {
  /// Contrôles locaux, avant tout envoi.
  telephoneVide,
  motDePasseVide,

  /// 401 identifiants_invalides : tous les refus, compte verrouillé compris.
  identifiants,

  /// 422 telephone_invalide.
  telephoneInvalide,

  /// Pas de réponse (ErreurReseau).
  reseau,

  /// 500 erreur_interne, réponse illisible, ou tout autre code.
  serveur,

  /// Étape de connexion inconnue (EtapeConnexionNonGeree, docs/api.md §1.7).
  etapeNonGeree,
}

/// État de l'écran de connexion (guide d'architecture Flutter : un view
/// model par écran, https://docs.flutter.dev/app-architecture).
///
/// Ne garde jamais le mot de passe : il est lu dans le champ au moment de
/// l'envoi, puis oublié.
class ConnexionViewModel extends ChangeNotifier {
  // Paramètre nommé privé (Dart ≥ 3.12) : l'appelant écrit auth:.
  // https://dart.dev/language/constructors
  ConnexionViewModel({required this._auth});

  final AuthRepository _auth;

  /// Indicatif du Burkina Faso, seul en V1 (décision du porteur), affiché
  /// devant le champ et ajouté à l'envoi. Une donnée, pas un texte à
  /// traduire : il n'est pas dans le .arb. Le numéro envoyé est l'indicatif
  /// suivi de la saisie, telle quelle : le serveur normalise (docs/api.md
  /// §1.6 : « il retire les espaces, les points et les tirets » ;
  /// « +226 70000000 » est accepté).
  static const indicatif = '+226';

  bool _enCours = false;
  ErreurConnexion? _erreur;
  bool _motDePasseVisible = false;
  bool _ferme = false;

  /// Envoi en cours : le bouton est bloqué.
  bool get enCours => _enCours;
  ErreurConnexion? get erreur => _erreur;
  bool get motDePasseVisible => _motDePasseVisible;

  void basculerMotDePasse() {
    _motDePasseVisible = !_motDePasseVisible;
    _prevenir();
  }

  /// Connexion. Renvoie vrai si la session est ouverte (la navigation suit
  /// l'état d'AuthRepository) ; faux sinon, avec [erreur] renseignée.
  /// Un second appel pendant un envoi est ignoré (double appui).
  Future<bool> connecter({
    required String saisieTelephone,
    required String motDePasse,
  }) async {
    if (_enCours) return false;
    // Mot de passe pris tel que saisi, espaces comprises (CLAUDE.md §14) :
    // seul le champ vide est refusé.
    final ErreurConnexion? locale = saisieTelephone.trim().isEmpty
        ? ErreurConnexion.telephoneVide
        : motDePasse.isEmpty
            ? ErreurConnexion.motDePasseVide
            : null;
    if (locale != null) {
      _erreur = locale;
      _prevenir();
      return false;
    }
    _enCours = true;
    _erreur = null;
    _prevenir();
    try {
      await _auth.connecter(
        telephone: '$indicatif$saisieTelephone',
        motDePasse: motDePasse,
      );
      return true;
    } on ErreurApi catch (erreur) {
      _erreur = switch (erreur.code) {
        CodesErreur.identifiantsInvalides => ErreurConnexion.identifiants,
        CodesErreur.telephoneInvalide => ErreurConnexion.telephoneInvalide,
        _ => ErreurConnexion.serveur,
      };
    } on ErreurReseau {
      _erreur = ErreurConnexion.reseau;
    } on EtapeConnexionNonGeree {
      _erreur = ErreurConnexion.etapeNonGeree;
    } finally {
      _enCours = false;
      _prevenir();
    }
    return false;
  }

  /// La personne modifie un champ : le message d'erreur disparaît.
  void effacerErreur() {
    if (_erreur == null) return;
    _erreur = null;
    _prevenir();
  }

  @override
  void dispose() {
    _ferme = true;
    super.dispose();
  }

  /// Après une connexion réussie, la navigation ferme l'écran pendant
  /// l'envoi : prévenir un ChangeNotifier fermé lèverait une erreur.
  void _prevenir() {
    if (!_ferme) notifyListeners();
  }
}
