/// Chemins de l'application.
abstract final class Routes {
  /// Démarrage, pendant la lecture de la session. La doc de go_router exige
  /// une route « / » : « It must not be empty and must contain an [GoRoute]
  /// to match `/` » (GoRouter.routes, go_router 18.0.2).
  static const demarrage = '/';

  static const connexion = '/connexion';

  /// Numéros d'urgence : accessibles sans compte.
  static const urgences = '/urgences';

  static const inscription = '/inscription';
  static const motDePasseOublie = '/mot-de-passe-oublie';
  static const accueil = '/accueil';

  /// Pages ouvertes sans session.
  static const sansCompte = {connexion, urgences, inscription, motDePasseOublie};

  /// Pages qui n'ont plus de sens une fois connecté : renvoi vers l'accueil.
  static const avantConnexion = {
    demarrage,
    connexion,
    inscription,
    motDePasseOublie,
  };
}
