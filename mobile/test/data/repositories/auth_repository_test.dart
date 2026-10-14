import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/repositories/auth_repository.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/data/services/gestionnaire_session.dart';
import 'package:patient/data/services/stockage_jeton.dart';
import 'package:patient/domain/models/erreur_api.dart';
import 'package:patient/domain/models/paire_jetons.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

/// Valeurs fictives des exemples de docs/api.md.
const _telephone = '70 00 00 00';
const _motDePasse = 'Exemple#2026';

const _otp = '/api/patients/otp';
const _inscription = '/api/patients/inscription';
const _connexion = '/api/patients/connexion';
const _reinitialisation = '/api/patients/mot-de-passe/reinitialiser';

final _formulaire = DonneesInscription(
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

/// Réponse d'ouverture de session (docs/api.md §1.7).
http.Response _ouverture(int statut) => _json(statut, {
      'etape': 'complete',
      'jeton_acces': 'acces-1',
      'expire_dans': 900,
      'jeton_renouvellement': 'renouv-1',
    });

http.Response _perdue(http.Request r) =>
    throw http.ClientException('coupure', r.url);

/// Dépôt branché sur un serveur simulé ; les attentes entre connexions de
/// contrôle sont notées sans être réellement attendues.
class _Banc {
  _Banc(Future<http.Response> Function(http.Request requete) serveur) {
    client = ClientApi(
      configuration: _configuration,
      client: MockClient((requete) {
        requetes.add(requete);
        return serveur(requete);
      }),
    );
    session = GestionnaireSession(client: client, stockage: StockageJeton());
    depot = AuthRepository(
      client: client,
      session: session,
      attendre: (duree) async => attentes.add(duree),
    );
    depot.addListener(() => notifications++);
  }

  final requetes = <http.Request>[];
  final attentes = <Duration>[];
  late final ClientApi client;
  late final GestionnaireSession session;
  late final AuthRepository depot;
  var notifications = 0;

  List<String> get chemins => [for (final r in requetes) r.url.path];
}

Future<String?> _acces() => StockageJeton().lireJetonAcces();
Future<String?> _renouvellement() => StockageJeton().lireJetonRenouvellement();

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('1. démarrage sans jetons : déconnecté', () async {
    final banc = _Banc((_) async => throw StateError('aucune requête'));
    expect(banc.depot.etat, EtatSession.inconnu);
    await banc.depot.initialiser();
    expect(banc.depot.etat, EtatSession.deconnecte);
    expect(banc.requetes, isEmpty);
  });

  test('2. démarrage avec jetons : connecté', () async {
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc((_) async => throw StateError('aucune requête'));
    await banc.depot.initialiser();
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('3. demanderCode : chemin et corps', () async {
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.depot.demanderCode(_telephone);
    expect(banc.chemins, [_otp]);
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

  test('5. connexion 200 : les deux jetons enregistrés, connecté', () async {
    final banc = _Banc((_) async => _ouverture(200));
    await banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse);
    expect(banc.chemins, [_connexion]);
    expect(jsonDecode(banc.requetes.single.body),
        {'telephone': _telephone, 'mot_de_passe': _motDePasse});
    expect(await _acces(), 'acces-1');
    expect(await _renouvellement(), 'renouv-1');
    expect(banc.depot.etat, EtatSession.connecte);
    expect(banc.notifications, 1);
  });

  test('6. connexion 401 : rien enregistré', () async {
    final banc = _Banc((_) async => _erreur(401, 'identifiants_invalides'));
    await banc.depot.initialiser();
    await expectLater(
      banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.identifiantsInvalides)),
    );
    expect(await _acces(), isNull);
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test('7. connexion second_facteur : EtapeConnexionNonGeree, rien '
      'enregistré', () async {
    final banc = _Banc((_) async => _json(200, {
          'etape': 'second_facteur',
          'jeton_intermediaire': 'jeton-intermediaire-exemple',
        }));
    await banc.depot.initialiser();
    await expectLater(
      banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse),
      throwsA(isA<EtapeConnexionNonGeree>()),
    );
    expect(await _acces(), isNull);
    expect(await _renouvellement(), isNull);
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test('8. connexion 200 mal formée : reponse_illisible, rien enregistré',
      () async {
    final banc = _Banc((_) async => _json(200, {'jeton': 'ancienne-forme'}));
    await expectLater(
      banc.depot.connecter(telephone: _telephone, motDePasse: _motDePasse),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.reponseIllisible)),
    );
    expect(await _acces(), isNull);
  });

  test('9. inscription 201 : corps complet, paire enregistrée, connecté',
      () async {
    final banc = _Banc((_) async => _ouverture(201));
    await banc.depot.inscrire(_formulaire);
    expect(banc.chemins, [_inscription]);
    expect(jsonDecode(banc.requetes.single.body), {
      'telephone': _telephone,
      'code_otp': '123456',
      'nom': 'OUEDRAOGO',
      'prenom': 'Awa',
      'date_naissance': '1995-04-12',
      'lieu_naissance': 'Koudougou',
      'mot_de_passe': _motDePasse,
    });
    expect(await _renouvellement(), 'renouv-1');
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('10. inscription, mot de passe non conforme : aucun contrôle',
      () async {
    final banc = _Banc((_) async => _json(422, {
          'erreur': 'x',
          'code': 'mot_de_passe_non_conforme',
          'regles_non_respectees': ['chiffre'],
        }));
    await expectLater(
      banc.depot.inscrire(_formulaire),
      throwsA(isA<ErreurApi>().having(
          (e) => e.reglesNonRespectees, 'regles', [ReglesMotDePasse.chiffre])),
    );
    expect(banc.chemins, [_inscription]);
  });

  test("11. réponse perdue, contrôle 200 : connecté, l'inscription n'est "
      'pas renvoyée', () async {
    final banc = _Banc((r) async =>
        r.url.path == _inscription ? _perdue(r) : _ouverture(200));
    await banc.depot.inscrire(_formulaire);
    expect(banc.chemins, [_inscription, _connexion]);
    expect(jsonDecode(banc.requetes.last.body),
        {'telephone': _telephone, 'mot_de_passe': _motDePasse});
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('12. réponse perdue, contrôle 401 : InscriptionNonCreee', () async {
    final banc = _Banc((r) async => r.url.path == _inscription
        ? _perdue(r)
        : _erreur(401, 'identifiants_invalides'));
    await expectLater(
        banc.depot.inscrire(_formulaire), throwsA(isA<InscriptionNonCreee>()));
    expect(await _acces(), isNull);
  });

  test('13. inscription 502, 503 ou 504 : connexion de contrôle', () async {
    for (final statut in [502, 503, 504]) {
      FlutterSecureStorage.setMockInitialValues({});
      final banc = _Banc((r) async => r.url.path == _inscription
          ? http.Response('<html>Bad Gateway</html>', statut)
          : _ouverture(200));
      await banc.depot.inscrire(_formulaire);
      expect(banc.chemins, [_inscription, _connexion], reason: '$statut');
      expect(banc.depot.etat, EtatSession.connecte, reason: '$statut');
    }
  });

  test('14. quatre contrôles échoués : attentes 2 s, 5 s, 15 s, puis '
      'InscriptionNonConfirmee ; confirmation plus tard réussie', () async {
    var reseauRevenu = false;
    var controles = 0;
    final banc = _Banc((r) async {
      if (r.url.path == _inscription) return _perdue(r);
      if (reseauRevenu) return _ouverture(200);
      // Réseau et 503 mêlés.
      controles++;
      return controles.isEven
          ? http.Response('<html>Service Unavailable</html>', 503)
          : _perdue(r);
    });
    await expectLater(banc.depot.inscrire(_formulaire),
        throwsA(isA<InscriptionNonConfirmee>()));
    expect(banc.chemins.where((c) => c == _connexion), hasLength(4));
    expect(banc.attentes, const [
      Duration(seconds: 2),
      Duration(seconds: 5),
      Duration(seconds: 15),
    ]);
    expect(banc.chemins.where((c) => c == _inscription), hasLength(1));

    reseauRevenu = true;
    await banc.depot
        .confirmerInscription(telephone: _telephone, motDePasse: _motDePasse);
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('15. deux contrôles échoués puis réussite : attentes 2 s, 5 s',
      () async {
    var controles = 0;
    final banc = _Banc((r) async {
      if (r.url.path == _inscription) return _perdue(r);
      return ++controles <= 2 ? _perdue(r) : _ouverture(200);
    });
    await banc.depot.inscrire(_formulaire);
    expect(banc.attentes,
        const [Duration(seconds: 2), Duration(seconds: 5)]);
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('16. relance avec 409, puis contrôle 200 : connecté sans erreur',
      () async {
    final banc = _Banc((r) async => r.url.path == _inscription
        ? _erreur(409, 'telephone_deja_utilise')
        : _ouverture(200));
    await banc.depot.inscrire(_formulaire, relance: true);
    expect(banc.chemins, [_inscription, _connexion]);
    expect(banc.depot.etat, EtatSession.connecte);
  });

  test('17. relance avec 409, puis contrôle 401 : NumeroDejaInscrit',
      () async {
    final banc = _Banc((r) async => r.url.path == _inscription
        ? _erreur(409, 'telephone_deja_utilise')
        : _erreur(401, 'identifiants_invalides'));
    await expectLater(banc.depot.inscrire(_formulaire, relance: true),
        throwsA(isA<NumeroDejaInscrit>()));
  });

  test('18. première inscription avec 409 : erreur remontée, aucun contrôle',
      () async {
    final banc = _Banc((_) async => _erreur(409, 'telephone_deja_utilise'));
    await expectLater(
      banc.depot.inscrire(_formulaire),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.telephoneDejaUtilise)),
    );
    expect(banc.chemins, [_inscription]);
  });

  test('19. réinitialisation 204 : corps envoyé, état inchangé', () async {
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.depot.initialiser();
    await banc.depot.reinitialiserMotDePasse(
      telephone: _telephone,
      codeOtp: '123456',
      nouveauMotDePasse: 'Nouveau#2026',
    );
    expect(banc.chemins, [_reinitialisation]);
    expect(jsonDecode(banc.requetes.single.body), {
      'telephone': _telephone,
      'code_otp': '123456',
      'nouveau_mot_de_passe': 'Nouveau#2026',
    });
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test('20. déconnexion : route appelée, jetons effacés, déconnecté',
      () async {
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.depot.initialiser();
    await banc.depot.deconnecter();
    expect(banc.chemins, ['/api/sessions/deconnexion']);
    expect(await _acces(), isNull);
    expect(await _renouvellement(), isNull);
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test('21. déconnexion partout sans réseau : jetons effacés, déconnecté, '
      'erreur remontée', () async {
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc((r) async => _perdue(r));
    await banc.depot.initialiser();
    await expectLater(
        banc.depot.deconnecterPartout(), throwsA(isA<ErreurReseau>()));
    expect(await _acces(), isNull);
    expect(banc.depot.etat, EtatSession.deconnecte);
  });

  test('22. session fermée par un renouvellement refusé : déconnecté',
      () async {
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc((_) async => _erreur(401, 'jeton_invalide'));
    await banc.depot.initialiser();
    expect(banc.depot.etat, EtatSession.connecte);
    await expectLater(
      banc.session.requete(
          (jeton) => banc.client.get('/api/patients/moi', jeton: jeton)),
      throwsA(isA<SessionFermee>()),
    );
    expect(banc.depot.etat, EtatSession.deconnecte);
  });
}
