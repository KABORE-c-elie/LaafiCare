import 'dart:convert';

/// Codes d'erreur de l'API utilisés par l'application patient
/// (docs/api.md §3). L'application décide toujours d'après ces codes,
/// jamais d'après le message (docs/api.md §1.3).
abstract final class CodesErreur {
  // Routes protégées (docs/api.md §1.2).
  static const authentificationRequise = 'authentification_requise';
  static const jetonInvalide = 'jeton_invalide';
  static const accesRefuse = 'acces_refuse';

  // Lecture de la requête (docs/api.md §1.4).
  static const jsonInvalide = 'json_invalide';
  static const donneesInvalides = 'donnees_invalides';
  static const contentTypeInvalide = 'content_type_invalide';
  static const corpsIllisible = 'corps_illisible';

  // Routes patient (docs/api.md §2.2).
  static const telephoneInvalide = 'telephone_invalide';
  static const codeInvalide = 'code_invalide';
  static const tropDeDemandes = 'trop_de_demandes';
  static const telephoneDejaUtilise = 'telephone_deja_utilise';
  static const telephoneInconnu = 'telephone_inconnu';
  static const motDePasseNonConforme = 'mot_de_passe_non_conforme';
  static const identifiantsInvalides = 'identifiants_invalides';

  static const erreurInterne = 'erreur_interne';

  /// Code propre à l'application, jamais envoyé par l'API : la réponse
  /// d'erreur n'a pas la forme {"erreur", "code"} (corps vide, page HTML
  /// d'un intermédiaire réseau…).
  static const reponseIllisible = 'reponse_illisible';
}

/// Valeurs de `regles_non_respectees` (docs/api.md §1.5).
///
/// Une valeur absente de cette liste (règle ajoutée plus tard par le
/// serveur) est gardée telle quelle par [ErreurApi] ; l'écran du mot de
/// passe affiche alors un texte général (« Le mot de passe ne respecte pas
/// une règle du serveur. »), jamais le code brut.
abstract final class ReglesMotDePasse {
  static const longueurMin = 'longueur_min';
  static const longueurMax = 'longueur_max';
  static const caractereDeControle = 'caractere_de_controle';
  static const lettre = 'lettre';
  static const chiffre = 'chiffre';
  static const caractereSpecial = 'caractere_special';
}

/// Réponse d'erreur de l'API, de forme {"erreur": "…", "code": "…"}
/// (docs/api.md §1.3).
///
/// Le champ `erreur` n'est volontairement pas gardé : l'application teste
/// `code`, jamais le message (docs/api.md §1.3), et ce texte peut reprendre
/// une donnée saisie (CLAUDE.md §12). Il ne doit donc finir ni à l'écran ni
/// dans un journal.
class ErreurApi implements Exception {
  const ErreurApi({
    required this.statut,
    required this.code,
    this.reglesNonRespectees = const [],
    this.attente,
  });

  /// Code HTTP de la réponse.
  final int statut;

  /// Code machine de l'erreur (voir [CodesErreur]).
  final String code;

  /// Règles du mot de passe non respectées, pour
  /// `mot_de_passe_non_conforme` (docs/api.md §1.5) ; vide sinon.
  final List<String> reglesNonRespectees;

  /// Délai à attendre avant une nouvelle demande, lu dans l'en-tête
  /// `Retry-After` d'un 429 `trop_de_demandes` (docs/api.md §2.2.5) ;
  /// null quand l'en-tête est absent ou n'est pas un nombre de secondes.
  final Duration? attente;

  /// Lit une réponse d'erreur de l'API.
  ///
  /// [corps] est le corps déjà décodé en UTF-8 par le client ; [entetes]
  /// a ses noms en minuscules, comme les donne le paquet http (« The header
  /// names are converted to lowercase », doc de BaseResponse.headers :
  /// https://pub.dev/documentation/http/latest/http/BaseResponse/headers.html).
  factory ErreurApi.depuisReponse({
    required int statut,
    required String corps,
    required Map<String, String> entetes,
  }) {
    final attente = _lireRetryAfter(entetes['retry-after']);
    final Object? json;
    try {
      json = jsonDecode(corps);
    } on FormatException {
      return ErreurApi(
        statut: statut,
        code: CodesErreur.reponseIllisible,
        attente: attente,
      );
    }
    if (json is! Map<String, dynamic> || json['code'] is! String) {
      return ErreurApi(
        statut: statut,
        code: CodesErreur.reponseIllisible,
        attente: attente,
      );
    }
    final regles = json['regles_non_respectees'];
    return ErreurApi(
      statut: statut,
      code: json['code'] as String,
      reglesNonRespectees: regles is List
          ? List.unmodifiable(regles.whereType<String>())
          : const [],
      attente: attente,
    );
  }

  /// RFC 9110 §10.2.3 : « Retry-After = HTTP-date / delay-seconds » ;
  /// « A delay-seconds value is a non-negative decimal integer, representing
  /// time in seconds. » docs/api.md §1.3 fixe la forme envoyée par le
  /// serveur : un nombre de secondes. Une date n'est donc pas lue, et donne
  /// null (délai inconnu) plutôt qu'une supposition.
  /// https://www.rfc-editor.org/rfc/rfc9110#section-10.2.3
  static Duration? _lireRetryAfter(String? valeur) {
    if (valeur == null) return null;
    final texte = valeur.trim();
    if (!RegExp(r'^[0-9]+$').hasMatch(texte)) return null;
    final secondes = int.tryParse(texte);
    return secondes == null ? null : Duration(seconds: secondes);
  }

  @override
  String toString() => 'ErreurApi($statut, $code)';
}
