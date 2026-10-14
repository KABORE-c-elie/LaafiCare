import 'package:flutter_test/flutter_test.dart';
import 'package:patient/domain/models/erreur_api.dart';
import 'package:patient/domain/models/paire_jetons.dart';

/// Valeurs fictives : jamais de vrai jeton dans un test.
const _acces = 'jeton-acces-exemple';
const _renouvellement = 'jeton-renouvellement-exemple';

Map<String, dynamic> _ouverture({Object? etape = 'complete'}) => {
      'etape': etape,
      'jeton_acces': _acces,
      'expire_dans': 900,
      'jeton_renouvellement': _renouvellement,
    };

Matcher _illisible(int statut) => throwsA(isA<ErreurApi>()
    .having((e) => e.code, 'code', CodesErreur.reponseIllisible)
    .having((e) => e.statut, 'statut', statut));

void main() {
  test('1. ouverture complete : les trois valeurs sont lues', () {
    final paire = PaireJetons.depuisOuverture(statut: 200, json: _ouverture());
    expect(paire.jetonAcces, _acces);
    expect(paire.jetonRenouvellement, _renouvellement);
    expect(paire.expireDans, const Duration(seconds: 900));
  });

  test('2. ouverture second_facteur : EtapeConnexionNonGeree', () {
    expect(
      () => PaireJetons.depuisOuverture(statut: 200, json: {
        'etape': 'second_facteur',
        'jeton_intermediaire': 'jeton-intermediaire-exemple',
      }),
      throwsA(isA<EtapeConnexionNonGeree>()
          .having((e) => e.etape, 'etape', 'second_facteur')),
    );
  });

  test('3. ouverture avec une étape inconnue : EtapeConnexionNonGeree', () {
    expect(
      () => PaireJetons.depuisOuverture(
          statut: 200, json: _ouverture(etape: 'autre')),
      throwsA(isA<EtapeConnexionNonGeree>()),
    );
  });

  test('4. ouverture sans etape, ou etape non textuelle : reponse_illisible',
      () {
    for (final json in [
      _ouverture()..remove('etape'),
      _ouverture(etape: 1),
      _ouverture(etape: null),
    ]) {
      expect(() => PaireJetons.depuisOuverture(statut: 201, json: json),
          _illisible(201),
          reason: '$json');
    }
  });

  test('5. champ manquant, vide ou de mauvais type : reponse_illisible', () {
    for (final (cle, valeur) in [
      ('jeton_acces', null),
      ('jeton_acces', ''),
      ('jeton_acces', 42),
      ('jeton_renouvellement', null),
      ('jeton_renouvellement', ''),
      ('expire_dans', null),
      ('expire_dans', '900'),
      ('expire_dans', 0),
      ('expire_dans', -1),
    ]) {
      final json = _ouverture()..[cle] = valeur;
      expect(() => PaireJetons.depuisOuverture(statut: 200, json: json),
          _illisible(200),
          reason: '$cle = $valeur');
    }
  });

  test('6. renouvellement sans etape : accepté', () {
    final paire = PaireJetons.depuisRenouvellement(
        statut: 200, json: _ouverture()..remove('etape'));
    expect(paire.jetonAcces, _acces);
    expect(paire.jetonRenouvellement, _renouvellement);
  });

  test('7. corps absent : reponse_illisible dans les deux cas', () {
    expect(() => PaireJetons.depuisOuverture(statut: 200, json: null),
        _illisible(200));
    expect(() => PaireJetons.depuisRenouvellement(statut: 200, json: null),
        _illisible(200));
  });

  test('8. toString ne contient aucun jeton', () {
    final paire = PaireJetons.depuisOuverture(statut: 200, json: _ouverture());
    expect(paire.toString(), isNot(contains(_acces)));
    expect(paire.toString(), isNot(contains(_renouvellement)));
    const etape = EtapeConnexionNonGeree('second_facteur');
    expect(etape.toString(), 'EtapeConnexionNonGeree(second_facteur)');
  });
}
