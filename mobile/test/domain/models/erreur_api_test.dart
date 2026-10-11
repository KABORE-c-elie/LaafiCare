import 'package:flutter_test/flutter_test.dart';
import 'package:patient/domain/models/erreur_api.dart';

void main() {
  group('ErreurApi.depuisReponse', () {
    test('lit le code (connexion refusée, docs/api.md §2.2.3)', () {
      final erreur = ErreurApi.depuisReponse(
        statut: 401,
        corps: '{"erreur": "Numéro ou mot de passe incorrect.", '
            '"code": "identifiants_invalides"}',
        entetes: const {},
      );
      expect(erreur.statut, 401);
      expect(erreur.code, CodesErreur.identifiantsInvalides);
      expect(erreur.reglesNonRespectees, isEmpty);
      expect(erreur.attente, isNull);
    });

    test('lit les règles non respectées (docs/api.md §1.5)', () {
      final erreur = ErreurApi.depuisReponse(
        statut: 422,
        corps: '{"erreur": "Mot de passe non conforme.", '
            '"code": "mot_de_passe_non_conforme", '
            '"regles_non_respectees": ["chiffre", "caractere_special"]}',
        entetes: const {},
      );
      expect(erreur.code, CodesErreur.motDePasseNonConforme);
      expect(erreur.reglesNonRespectees,
          [ReglesMotDePasse.chiffre, ReglesMotDePasse.caractereSpecial]);
    });

    test('lit Retry-After en secondes (429 sur la demande de code)', () {
      final erreur = ErreurApi.depuisReponse(
        statut: 429,
        corps: '{"erreur": "Trop de demandes.", "code": "trop_de_demandes"}',
        entetes: const {'retry-after': '45'},
      );
      expect(erreur.code, CodesErreur.tropDeDemandes);
      expect(erreur.attente, const Duration(seconds: 45));
    });

    test('429 sans Retry-After (inscription, dernier essai) : délai inconnu',
        () {
      final erreur = ErreurApi.depuisReponse(
        statut: 429,
        corps: '{"erreur": "Trop d\'essais.", "code": "trop_de_demandes"}',
        entetes: const {},
      );
      expect(erreur.attente, isNull);
    });

    test('Retry-After en date ou invalide : délai inconnu', () {
      for (final valeur in ['Fri, 31 Dec 1999 23:59:59 GMT', '-5', '1.5', '']) {
        final erreur = ErreurApi.depuisReponse(
          statut: 429,
          corps: '{"erreur": "x", "code": "trop_de_demandes"}',
          entetes: {'retry-after': valeur},
        );
        expect(erreur.attente, isNull, reason: valeur);
      }
    });

    test('corps vide, HTML ou sans code : reponse_illisible, statut gardé', () {
      for (final corps in [
        '',
        '<html>Bad Gateway</html>',
        '{"erreur": "sans code"}',
        '{"code": 42}',
        '[]',
      ]) {
        final erreur = ErreurApi.depuisReponse(
          statut: 502,
          corps: corps,
          entetes: const {},
        );
        expect(erreur.code, CodesErreur.reponseIllisible, reason: corps);
        expect(erreur.statut, 502);
      }
    });

    test('toString ne recopie que le statut et le code', () {
      final erreur = ErreurApi.depuisReponse(
        statut: 422,
        corps: '{"erreur": "donnée saisie", "code": "donnees_invalides"}',
        entetes: const {},
      );
      expect(erreur.toString(), 'ErreurApi(422, donnees_invalides)');
    });
  });
}
