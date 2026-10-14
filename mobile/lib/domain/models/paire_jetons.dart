import 'erreur_api.dart';

/// Paire de jetons d'une session patient (docs/api.md §1.7) : jeton d'accès
/// (15 minutes) et jeton de renouvellement (change à chaque usage).
///
/// Jamais affichée ni écrite dans un journal : [toString] ne montre aucun
/// jeton.
class PaireJetons {
  const PaireJetons({
    required this.jetonAcces,
    required this.jetonRenouvellement,
    required this.expireDans,
  });

  final String jetonAcces;

  /// Texte non vide, sans contrôle de format : le serveur reste le seul juge
  /// du jeton (décision du porteur), même si docs/api.md §1.7 annonce
  /// 64 caractères hexadécimaux.
  final String jetonRenouvellement;

  /// Durée de vie du jeton d'accès (`expire_dans`, en secondes). Gardée
  /// pour un futur renouvellement anticipé ; aujourd'hui, l'application
  /// renouvelle seulement sur un 401 (docs/api.md §1.7).
  final Duration expireDans;

  /// Valeur d'`etape` qui ouvre la session (docs/api.md §1.7 : « aujourd'hui,
  /// etape vaut toujours complete. L'application doit quand même tester
  /// etape »).
  static const etapeComplete = 'complete';

  /// Réponse de la connexion (200) ou de l'inscription (201).
  ///
  /// `etape` est lue en premier : une réponse `second_facteur` n'a pas de
  /// `jeton_acces`, mais doit donner [EtapeConnexionNonGeree], pas une
  /// réponse illisible. Toute autre étape que `complete` lève
  /// [EtapeConnexionNonGeree] : rien n'est enregistré, l'écran affiche un
  /// message clair.
  static PaireJetons depuisOuverture({
    required int statut,
    required Map<String, dynamic>? json,
  }) {
    final etape = json?['etape'];
    if (etape is! String) _illisible(statut);
    if (etape != etapeComplete) throw EtapeConnexionNonGeree(etape);
    return _lirePaire(statut, json!);
  }

  /// Réponse du renouvellement (200) : « même forme […] sans etape »
  /// (docs/api.md §1.7). Une `etape` éventuelle est ignorée.
  static PaireJetons depuisRenouvellement({
    required int statut,
    required Map<String, dynamic>? json,
  }) {
    if (json == null) _illisible(statut);
    return _lirePaire(statut, json);
  }

  static PaireJetons _lirePaire(int statut, Map<String, dynamic> json) {
    final acces = json['jeton_acces'];
    final renouvellement = json['jeton_renouvellement'];
    final expireDans = json['expire_dans'];
    if (acces is! String ||
        acces.isEmpty ||
        renouvellement is! String ||
        renouvellement.isEmpty ||
        expireDans is! int ||
        expireDans <= 0) {
      _illisible(statut);
    }
    return PaireJetons(
      jetonAcces: acces,
      jetonRenouvellement: renouvellement,
      expireDans: Duration(seconds: expireDans),
    );
  }

  static Never _illisible(int statut) => throw ErreurApi(
        statut: statut,
        code: CodesErreur.reponseIllisible,
      );

  @override
  String toString() => 'PaireJetons(expireDans: $expireDans)';
}

/// La connexion a réussi, mais demande une étape que l'application ne gère
/// pas encore (ex. `second_facteur`, docs/api.md §1.7). Aucun jeton n'est
/// enregistré ; l'écran affiche un message clair, jamais un plantage.
class EtapeConnexionNonGeree implements Exception {
  const EtapeConnexionNonGeree(this.etape);

  /// Valeur reçue (un code court comme `second_facteur`, jamais un jeton).
  final String etape;

  @override
  String toString() => 'EtapeConnexionNonGeree($etape)';
}
