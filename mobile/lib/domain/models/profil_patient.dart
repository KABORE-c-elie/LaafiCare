import 'erreur_api.dart';

/// Profil du patient connecté, lu par GET /api/patients/moi (docs/api.md
/// §2.2.6). Gardé en mémoire seulement (décision du porteur) : jamais écrit
/// sur le téléphone.
///
/// Données personnelles : [toString] n'en montre aucune.
class ProfilPatient {
  const ProfilPatient({
    required this.compteId,
    required this.nip,
    required this.nom,
    required this.prenom,
    required this.telephone,
    required this.dateNaissance,
    required this.lieuNaissance,
  });

  /// Identifiant du compte, « ne change jamais » (§2.2.6) : servira à
  /// rattacher ce que l'application garde sur le téléphone (rappels).
  final String compteId;
  final String nip;
  final String nom;
  final String prenom;

  /// Forme normalisée (`+226…`).
  final String telephone;
  final DateTime dateNaissance;
  final String lieuNaissance;

  /// Lit la réponse 200 de /moi. Un champ absent, d'un autre type, ou une
  /// date qui n'est pas AAAA-MM-JJ valide : reponse_illisible.
  static ProfilPatient depuisJson({
    required int statut,
    required Map<String, dynamic>? json,
  }) {
    String texte(String cle) {
      final valeur = json?[cle];
      if (valeur is! String) _illisible(statut);
      return valeur;
    }

    final compteId = texte('compte_id');
    final nip = texte('nip');
    if (compteId.isEmpty || nip.isEmpty) _illisible(statut);
    final date = _lireDate(texte('date_naissance'));
    if (date == null) _illisible(statut);
    return ProfilPatient(
      compteId: compteId,
      nip: nip,
      nom: texte('nom'),
      prenom: texte('prenom'),
      telephone: texte('telephone'),
      dateNaissance: date,
      lieuNaissance: texte('lieu_naissance'),
    );
  }

  /// Premier prénom : coupé aux espaces seulement (un prénom composé avec un
  /// trait d'union reste entier), majuscules inchangées ; vide si le prénom
  /// n'a que des espaces (décision du porteur). Toutes les espaces Unicode,
  /// et non `\s` de Dart (sémantique JavaScript : il ajoute U+FEFF et oublie
  /// U+0085) : un prénom saisi sur un autre clavier peut contenir une espace
  /// insécable.
  String get premierPrenom {
    final mots = prenom.split(_espaceUnicode).where((mot) => mot.isNotEmpty);
    return mots.isEmpty ? '' : mots.first;
  }

  /// NIP en 4 groupes de 4 chiffres (« 1000 0000 0000 0008 ») ; affiché tel
  /// quel s'il n'a pas 16 chiffres (ne devrait pas arriver : CLAUDE.md §6,
  /// 16 chiffres avec clé de Luhn).
  String get nipGroupe {
    if (!RegExp(r'^[0-9]{16}$').hasMatch(nip)) return nip;
    return [
      for (var i = 0; i < 16; i += 4) nip.substring(i, i + 4),
    ].join(' ');
  }

  @override
  String toString() => 'ProfilPatient(…)';

  /// Les 25 caractères de la propriété Unicode White_Space, d'après
  /// PropList-18.0.0.txt (Unicode, 2026-08-07, « Total code points: 25 ») :
  /// 0009..000D, 0020, 0085, 00A0, 1680, 2000..200A, 2028, 2029, 202F,
  /// 205F, 3000. https://www.unicode.org/Public/UCD/latest/ucd/PropList.txt
  static final _espaceUnicode = RegExp(
    '[\u{9}-\u{D}\u{20}\u{85}\u{A0}\u{1680}\u{2000}-\u{200A}'
    '\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}]',
    unicode: true,
  );

  /// AAAA-MM-JJ (docs/api.md §1.1), date réelle : « 2026-02-30 » refusée.
  static DateTime? _lireDate(String texte) {
    final parties =
        RegExp(r'^([0-9]{4})-([0-9]{2})-([0-9]{2})$').firstMatch(texte);
    if (parties == null) return null;
    final a = int.parse(parties[1]!);
    final m = int.parse(parties[2]!);
    final j = int.parse(parties[3]!);
    final date = DateTime.utc(a, m, j);
    return date.year == a && date.month == m && date.day == j ? date : null;
  }

  static Never _illisible(int statut) =>
      throw ErreurApi(statut: statut, code: CodesErreur.reponseIllisible);
}
