import 'package:flutter_test/flutter_test.dart';
import 'package:patient/domain/models/erreur_api.dart';
import 'package:patient/domain/models/profil_patient.dart';

/// Exemple fictif de docs/api.md §2.2.6.
Map<String, dynamic> _exemple() => {
      'compte_id': '0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55',
      'nip': '1000000000000008',
      'nom': 'OUEDRAOGO',
      'prenom': 'Awa',
      'telephone': '+22670000000',
      'date_naissance': '1995-04-12',
      'lieu_naissance': 'Koudougou',
    };

ProfilPatient _profil([Map<String, dynamic>? json]) =>
    ProfilPatient.depuisJson(statut: 200, json: json ?? _exemple());

ProfilPatient _avecPrenom(String prenom) => _profil(_exemple()..['prenom'] = prenom);

Matcher _illisible(int statut) => throwsA(isA<ErreurApi>()
    .having((e) => e.code, 'code', CodesErreur.reponseIllisible)
    .having((e) => e.statut, 'statut', statut));

void main() {
  test('1. réponse complète : tous les champs lus', () {
    final profil = _profil();
    expect(profil.compteId, '0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55');
    expect(profil.nip, '1000000000000008');
    expect(profil.nom, 'OUEDRAOGO');
    expect(profil.prenom, 'Awa');
    expect(profil.telephone, '+22670000000');
    expect(profil.dateNaissance, DateTime.utc(1995, 4, 12));
    expect(profil.lieuNaissance, 'Koudougou');
  });

  test('2. champ absent, autre type, compte_id ou nip vide : '
      'reponse_illisible', () {
    for (final cle in _exemple().keys) {
      expect(() => _profil(_exemple()..remove(cle)), _illisible(200),
          reason: 'sans $cle');
      expect(() => _profil(_exemple()..[cle] = 42), _illisible(200),
          reason: '$cle = 42');
    }
    for (final cle in ['compte_id', 'nip']) {
      expect(() => _profil(_exemple()..[cle] = ''), _illisible(200),
          reason: '$cle vide');
    }
    expect(() => ProfilPatient.depuisJson(statut: 200, json: null),
        _illisible(200));
  });

  test('3. date : AAAA-MM-JJ réelle seulement', () {
    for (final date in ['1995-4-12', '12/04/1995', '2026-02-30', '']) {
      expect(() => _profil(_exemple()..['date_naissance'] = date),
          _illisible(200),
          reason: date);
    }
  });

  test('4. premier prénom : coupé aux espaces Unicode seulement', () {
    for (final (prenom, attendu) in [
      ('Awa', 'Awa'),
      ('Awa Marie', 'Awa'),
      ('Jean-Marie Paul', 'Jean-Marie'),
      ('awa', 'awa'),
      ('Awa\u{A0}Marie', 'Awa'),
      ('Awa\u{202F}Marie', 'Awa'),
      ('   Awa Marie', 'Awa'),
      ('', ''),
      (' \u{A0}\u{3000} ', ''),
    ]) {
      expect(_avecPrenom(prenom).premierPrenom, attendu, reason: '"$prenom"');
    }
  });

  test('5. NIP en groupes de 4 ; tel quel sans 16 chiffres', () {
    expect(_profil().nipGroupe, '1000 0000 0000 0008');
    final court = _profil(_exemple()..['nip'] = '100000000000008');
    expect(court.nipGroupe, '100000000000008');
  });

  test('6. toString ne montre aucune donnée personnelle', () {
    final texte = _profil().toString();
    for (final donnee in [
      'OUEDRAOGO',
      'Awa',
      '1000000000000008',
      '+22670000000',
      'Koudougou',
      '1995',
    ]) {
      expect(texte, isNot(contains(donnee)), reason: donnee);
    }
  });
}
