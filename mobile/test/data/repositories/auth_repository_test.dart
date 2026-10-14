import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/repositories/auth_repository.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/data/services/stockage_jeton.dart';
import 'package:patient/domain/models/erreur_api.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

/// Valeurs fictives des exemples de docs/api.md.
const _telephone = '70 00 00 00';
const _motDePasse = 'Exemple#2026';

final _inscription = DonneesInscription(
  telephone: _telephone,
  codeOtp: '123456',
  nom: 'OUEDRAOGO',
  prenom: 'Awa',
  dateNaissance: DateTime(1995, 4, 12),
  lieuNaissance: 'Koudougou',
  motDePasse: _motDePasse,
);

http.Response _json(int statut, Object corps, {Map<String, String>? entetes}) =>
    http.Response.bytes(utf8.encode(jsonEncode(corps)), statut,
        headers: {'content-type': 'application/json', ...?entetes});

http.Response _erreur(int statut, String code) =>
    _json(statut, {'erreur': 'x', 'code': code});

/// Dépôt branché sur un serveur simulé ; [requetes] garde chaque requête
/// reçue, dans l'ordre.
class _Banc {
  _Banc(Future<http.Response> Function(http.Request requete) serveur) {
    depot = AuthRepository(
      client: ClientApi(
        configuration: _configuration,
        client: MockClient((requete) {
          requetes.add(requete);
          return serveur(requete);
        }),
      ),
      stockage: StockageJeton(),
    );
    depot.addListener(() => notifications++);
  }

  final requetes = <http.Request>[];
  late final AuthRepository depot;
  var notifications = 0;

  List<String> get chemins => [for (final r in requetes) r.url.path];
}

Future<String?> _jetonEnregistre() => StockageJeton().lireJetonAcces();

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('1. démarrage sans jeton : déconnecté', () async {
    final banc = _Banc((_) async => throw StateError('aucune requête'));
    expect(banc.depot.etat, EtatSession.inconnu);
    await banc.depot.initialiser();
    expect(banc.depot.etat, EtatSession.deconnecte);
    expect(banc.requetes, isEmpty);
  });

  test('2. démarrage avec jeton : connecté', () async {
    await StockageJeton().enregistrerJetonAcces('jeton-exemple');
    final banc = _Banc((_) async => throw StateError('aucune requête'));
    await banc.depot.initialiser();
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('3. demanderCode : chemin et corps', () async {
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.depot.demanderCode(_telephone);
    expect(banc.chemins, ['/api/patients/otp']);
    expect(jsonDecode(banc.requetes.single.body), {'telephone': _telephone});
  });

  test('4. demanderCode, 429 avec Retry-After : délai transmis', () async {
    final banc = _Banc((_) async => _json(
        429, {'erreur': 'x', 'code': 'trop_de_demandes'},
        entetes: {'retry-after': '45'}));
    await expectLater(
      banc.depot.demanderCode(_telephone),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.tropDeDemandes)
          .having((e) => e.attente, 'attente', const Duration(seconds: 45))),
    );
  });

  test('5. connecter, 200 : jeton enregistré, connecté, écouteurs prévenus',
      () async {
    final banc = _Banc((_) async => _json(200, {'jeton': 'jeton-exemple'}));
    await banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse);
    expect(banc.chemins, ['/api/patients/connexion']);
    expect(jsonDecode(banc.requetes.single.body),
        {'telephone': _telephone, 'mot_de_passe': _motDePasse});
    expect(await _jetonEnregistre(), 'jeton-exemple');
    expect(banc.depot.etat, EtatSession.connecte);
    expect(banc.notifications, 1);
  });

  test('6. connecter, 401 identifiants_invalides : rien enregistré',
      () async {
    final banc = _Banc((_) async => _erreur(401, 'identifiants_invalides'));
    await banc.depot.initialiser();
    await expectLater(
      banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.identifiantsInvalides)),
    );
    expect(await _jetonEnregistre(), isNull);
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test('7. connecter, 200 sans jeton : reponse_illisible, rien enregistré',
      () async {
    final banc = _Banc((_) async => _json(200, {'autre': 'x'}));
    await expectLater(
      banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.reponseIllisible)),
    );
    expect(await _jetonEnregistre(), isNull);
  });

  test('8. inscrire, 201 : corps complet, date AAAA-MM-JJ, connecté',
      () async {
    final banc = _Banc((_) async => _json(201, {'jeton': 'jeton-exemple'}));
    await banc.depot.inscrire(_inscription);
    expect(banc.chemins, ['/api/patients/inscription']);
    expect(jsonDecode(banc.requetes.single.body), {
      'telephone': _telephone,
      'code_otp': '123456',
      'nom': 'OUEDRAOGO',
      'prenom': 'Awa',
      'date_naissance': '1995-04-12',
      'lieu_naissance': 'Koudougou',
      'mot_de_passe': _motDePasse,
    });
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('9. inscrire, mot de passe non conforme : pas de connexion de contrôle',
      () async {
    final banc = _Banc((_) async => _json(422, {
          'erreur': 'x',
          'code': 'mot_de_passe_non_conforme',
          'regles_non_respectees': ['chiffre'],
        }));
    await expectLater(
      banc.depot.inscrire(_inscription),
      throwsA(isA<ErreurApi>().having(
          (e) => e.reglesNonRespectees, 'regles', [ReglesMotDePasse.chiffre])),
    );
    expect(banc.chemins, ['/api/patients/inscription']);
  });

  test('10. inscrire, réponse perdue puis connexion 200 : connecté, '
      "l'inscription n'est pas renvoyée", () async {
    final banc = _Banc((requete) async {
      if (requete.url.path == '/api/patients/inscription') {
        throw http.ClientException('coupure', requete.url);
      }
      return _json(200, {'jeton': 'jeton-exemple'});
    });
    await banc.depot.inscrire(_inscription);
    expect(banc.chemins,
        ['/api/patients/inscription', '/api/patients/connexion']);
    expect(jsonDecode(banc.requetes.last.body),
        {'telephone': _telephone, 'mot_de_passe': _motDePasse});
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('11. inscrire, réponse perdue puis connexion 401 : InscriptionNonCreee',
      () async {
    final banc = _Banc((requete) async {
      if (requete.url.path == '/api/patients/inscription') {
        throw http.ClientException('coupure', requete.url);
      }
      return _erreur(401, 'identifiants_invalides');
    });
    await expectLater(
      banc.depot.inscrire(_inscription),
      throwsA(isA<InscriptionNonCreee>()),
    );
    expect(await _jetonEnregistre(), isNull);
  });

  test('12. inscrire, réponse perdue puis réseau absent : '
      'InscriptionNonConfirmee, puis confirmation réussie', () async {
    var reseauRevenu = false;
    final banc = _Banc((requete) async {
      if (!reseauRevenu) throw http.ClientException('coupure', requete.url);
      return _json(200, {'jeton': 'jeton-exemple'});
    });
    await expectLater(
      banc.depot.inscrire(_inscription),
      throwsA(isA<InscriptionNonConfirmee>()),
    );
    reseauRevenu = true;
    await banc.depot
        .confirmerInscription(telephone: _telephone, motDePasse: _motDePasse);
    expect(banc.chemins, [
      '/api/patients/inscription',
      '/api/patients/connexion',
      '/api/patients/connexion',
    ]);
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('13. réinitialiser 204 : état inchangé ; puis déconnecter',
      () async {
    await StockageJeton().enregistrerJetonAcces('jeton-exemple');
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.depot.initialiser();
    await banc.depot.reinitialiserMotDePasse(
      telephone: _telephone,
      codeOtp: '123456',
      nouveauMotDePasse: 'Nouveau#2026',
    );
    expect(banc.chemins, ['/api/patients/mot-de-passe/reinitialiser']);
    expect(jsonDecode(banc.requetes.single.body), {
      'telephone': _telephone,
      'code_otp': '123456',
      'nouveau_mot_de_passe': 'Nouveau#2026',
    });
    expect(banc.depot.etat, EtatSession.connecte);

    await banc.depot.deconnecter();
    expect(await _jetonEnregistre(), isNull);
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test("14. inscrire, 502/503/504 d'un intermédiaire : connexion de contrôle "
      '(choix du porteur)', () async {
    for (final statut in [502, 503, 504]) {
      FlutterSecureStorage.setMockInitialValues({});
      final banc = _Banc((requete) async {
        if (requete.url.path == '/api/patients/inscription') {
          return http.Response('<html>Bad Gateway</html>', statut);
        }
        return _json(200, {'jeton': 'jeton-exemple'});
      });
      await banc.depot.inscrire(_inscription);
      expect(banc.chemins,
          ['/api/patients/inscription', '/api/patients/connexion'],
          reason: '$statut');
      expect(banc.depot.etat, EtatSession.connecte, reason: '$statut');
    }
  });
}
