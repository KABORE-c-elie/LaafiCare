import 'dart:async';
import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/data/services/gestionnaire_session.dart';
import 'package:patient/data/services/stockage_jeton.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

const _moi = '/api/patients/moi';
const _renouveler = '/api/sessions/renouveler';
const _deconnexion = '/api/sessions/deconnexion';
const _partout = '/api/sessions/deconnexion-partout';

http.Response _json(int statut, Object corps) => http.Response.bytes(
    utf8.encode(jsonEncode(corps)), statut,
    headers: {'content-type': 'application/json'});

http.Response _refus() =>
    _json(401, {'erreur': 'x', 'code': 'jeton_invalide'});

http.Response _paire(int n) => _json(200, {
      'jeton_acces': 'acces-$n',
      'expire_dans': 900,
      'jeton_renouvellement': 'renouv-$n',
    });

String? _bearer(http.BaseRequest r) => r.headers['Authorization'];

String? _renouvellementEnvoye(http.Request r) =>
    (jsonDecode(r.body) as Map)['jeton_renouvellement'] as String?;

/// Gestionnaire branché sur un serveur simulé ; [requetes] garde chaque
/// requête reçue.
class _Banc {
  _Banc(
    Future<http.Response> Function(http.Request requete) serveur, {
    StockageJeton? stockage,
    Duration delaiRenouvellement =
        GestionnaireSession.delaiRenouvellementParDefaut,
  }) : stockage = stockage ?? StockageJeton() {
    client = ClientApi(
      configuration: _configuration,
      client: MockClient((requete) {
        requetes.add(requete);
        return serveur(requete);
      }),
    );
    session = GestionnaireSession(
      client: client,
      stockage: this.stockage,
      delaiRenouvellement: delaiRenouvellement,
    )..surSessionFermee = () => fermetures++;
  }

  final StockageJeton stockage;
  final requetes = <http.Request>[];
  late final ClientApi client;
  late final GestionnaireSession session;
  var fermetures = 0;

  List<String> get chemins => [for (final r in requetes) r.url.path];

  Future<ReponseApi> moi() =>
      session.requete((jeton) => client.get(_moi, jeton: jeton));
}

/// Stockage en mémoire dont l'écriture du jeton d'accès peut être retenue
/// (test 18).
class _StockageRetenu extends Fake implements FlutterSecureStorage {
  _StockageRetenu(this.valeurs);

  final Map<String, String> valeurs;
  Completer<void>? retenueAcces;
  final ecritureAccesCommencee = Completer<void>();

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async =>
      valeurs[key];

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (key == 'jeton_acces' && retenueAcces != null) {
      ecritureAccesCommencee.complete();
      await retenueAcces!.future;
    }
    valeurs[key] = value!;
  }

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async =>
      valeurs.remove(key);
}

void main() {
  // Session ouverte avec la paire n° 1 (valeurs fictives).
  setUp(() => FlutterSecureStorage.setMockInitialValues(
      {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'}));

  test('1. 401, renouvellement, rejeu réussi avec le nouveau jeton',
      () async {
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) return _paire(2);
      return _bearer(r) == 'Bearer acces-2' ? _json(200, {}) : _refus();
    });
    final reponse = await banc.moi();
    expect(reponse.statut, 200);
    expect(banc.chemins, [_moi, _renouveler, _moi]);
    expect(_bearer(banc.requetes.last), 'Bearer acces-2');
  });

  test('2. la paire est enregistrée avant le rejeu', () async {
    String? renouvellementAuRejeu;
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) return _paire(2);
      if (_bearer(r) == 'Bearer acces-2') {
        renouvellementAuRejeu =
            await StockageJeton().lireJetonRenouvellement();
        return _json(200, {});
      }
      return _refus();
    });
    await banc.moi();
    expect(renouvellementAuRejeu, 'renouv-2');
  });

  test('3. trois requêtes simultanées en 401 : un seul renouvellement',
      () async {
    final tousRefuses = Completer<void>();
    var refus = 0;
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) {
        await tousRefuses.future;
        return _paire(2);
      }
      if (_bearer(r) == 'Bearer acces-2') return _json(200, {});
      if (++refus == 3) tousRefuses.complete();
      return _refus();
    });
    final reponses = await Future.wait([banc.moi(), banc.moi(), banc.moi()]);
    expect(reponses.map((r) => r.statut), [200, 200, 200]);
    expect(banc.chemins.where((c) => c == _renouveler), hasLength(1));
    expect(banc.chemins.where((c) => c == _moi), hasLength(6));
  });

  test('4. rejeu encore en 401 : session fermée, aucune autre requête',
      () async {
    final banc = _Banc((r) async =>
        r.url.path == _renouveler ? _paire(2) : _refus());
    await expectLater(banc.moi(), throwsA(isA<SessionFermee>()));
    expect(banc.chemins, [_moi, _renouveler, _moi]);
    expect(await banc.session.estOuverte(), isFalse);
    expect(banc.fermetures, 1);
  });

  test('5. renouvellement en 401 : session fermée, rappel appelé', () async {
    final banc = _Banc((_) async => _refus());
    await expectLater(banc.moi(), throwsA(isA<SessionFermee>()));
    expect(banc.chemins, [_moi, _renouveler]);
    expect(await banc.stockage.lireJetonAcces(), isNull);
    expect(await banc.stockage.lireJetonRenouvellement(), isNull);
    expect(banc.fermetures, 1);
  });

  test('6. 1re tentative perdue (réseau), 2e réussie avec le même jeton',
      () async {
    var tentatives = 0;
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) {
        if (++tentatives == 1) throw http.ClientException('coupure', r.url);
        return _paire(2);
      }
      return _bearer(r) == 'Bearer acces-2' ? _json(200, {}) : _refus();
    });
    await banc.moi();
    final envoyes = [
      for (final r in banc.requetes)
        if (r.url.path == _renouveler) _renouvellementEnvoye(r)
    ];
    expect(envoyes, ['renouv-1', 'renouv-1']);
  });

  test('7. 1re tentative en 503, 2e réussie (C9)', () async {
    var tentatives = 0;
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) {
        return ++tentatives == 1
            ? http.Response('<html>Service Unavailable</html>', 503)
            : _paire(2);
      }
      return _bearer(r) == 'Bearer acces-2' ? _json(200, {}) : _refus();
    });
    expect((await banc.moi()).statut, 200);
    expect(tentatives, 2);
  });

  test('8. deux tentatives perdues : erreur réseau, jetons gardés, '
      'pas de troisième', () async {
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) {
        throw http.ClientException('coupure', r.url);
      }
      return _refus();
    });
    await expectLater(banc.moi(), throwsA(isA<ErreurReseau>()));
    expect(banc.chemins.where((c) => c == _renouveler), hasLength(2));
    expect(await banc.stockage.lireJetonAcces(), 'acces-1');
    expect(await banc.stockage.lireJetonRenouvellement(), 'renouv-1');
    expect(banc.fermetures, 0);
  });

  test("9. l'ancien jeton de renouvellement n'est jamais renvoyé", () async {
    var accesValide = 'acces-2';
    var prochain = 2;
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) return _paire(prochain++);
      return _bearer(r) == 'Bearer $accesValide' ? _json(200, {}) : _refus();
    });
    await banc.moi(); // renouvelle avec renouv-1, reçoit la paire 2
    accesValide = 'acces-3'; // le jeton acces-2 expire
    await banc.moi(); // renouvelle avec renouv-2, reçoit la paire 3
    final envoyes = [
      for (final r in banc.requetes)
        if (r.url.path == _renouveler) _renouvellementEnvoye(r)
    ];
    expect(envoyes, ['renouv-1', 'renouv-2']);
  });

  test('10. requête envoyée avec un jeton déjà remplacé : rejeu sans '
      'renouvellement', () async {
    final retenueA = Completer<void>();
    var appelsAcces1 = 0;
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) return _paire(2);
      if (_bearer(r) == 'Bearer acces-2') return _json(200, {});
      if (++appelsAcces1 == 1) await retenueA.future; // requête A retenue
      return _refus();
    });
    final a = banc.moi();
    await Future<void>.delayed(Duration.zero);
    await banc.moi(); // B : 401, renouvellement, rejeu
    retenueA.complete();
    expect((await a).statut, 200);
    expect(banc.chemins.where((c) => c == _renouveler), hasLength(1));
  });

  test('11. renouvellement sans Authorization, avec le délai court',
      () async {
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) return _paire(2);
      return _bearer(r) == 'Bearer acces-2' ? _json(200, {}) : _refus();
    });
    await banc.moi();
    final renouvellement =
        banc.requetes.firstWhere((r) => r.url.path == _renouveler);
    expect(renouvellement.headers.containsKey('Authorization'), isFalse);

    // Délai court : 10 ms pour le renouvellement, 30 s pour le reste.
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final client = ClientApi(
      configuration: _configuration,
      client: MockClient.streaming((r, _) async {
        if (r.url.path == _renouveler) {
          await (r as http.Abortable).abortTrigger;
          throw http.RequestAbortedException(r.url);
        }
        return http.StreamedResponse(
            Stream.value(utf8.encode(
                jsonEncode({'erreur': 'x', 'code': 'jeton_invalide'}))),
            401);
      }),
    );
    final session = GestionnaireSession(
      client: client,
      stockage: StockageJeton(),
      delaiRenouvellement: const Duration(milliseconds: 10),
    );
    await expectLater(
      session.requete((jeton) => client.get(_moi, jeton: jeton)),
      throwsA(isA<ErreurReseau>()
          .having((e) => e.delaiDepasse, 'delaiDepasse', isTrue)),
    );
  });

  test('12. déconnexion avec réseau : jetons effacés, route appelée',
      () async {
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.session.deconnecter();
    expect(await banc.session.estOuverte(), isFalse);
    expect(banc.chemins, [_deconnexion]);
    expect(_renouvellementEnvoye(banc.requetes.single), 'renouv-1');
    expect(banc.fermetures, 1);
  });

  test('13. déconnexion sans réseau : jetons effacés, aucune erreur',
      () async {
    final banc =
        _Banc((r) async => throw http.ClientException('coupure', r.url));
    await banc.session.deconnecter();
    expect(await banc.session.estOuverte(), isFalse);
    expect(await banc.stockage.lireJetonRenouvellement(), isNull);
  });

  test('14. déconnexion partout réussie : jeton d\'accès envoyé, '
      'jetons effacés', () async {
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.session.deconnecterPartout();
    expect(banc.chemins, [_partout]);
    expect(_bearer(banc.requetes.single), 'Bearer acces-1');
    expect(await banc.session.estOuverte(), isFalse);
  });

  test('15. déconnexion partout sans réseau : jetons effacés et erreur '
      'remontée', () async {
    final banc =
        _Banc((r) async => throw http.ClientException('coupure', r.url));
    await expectLater(
        banc.session.deconnecterPartout(), throwsA(isA<ErreurReseau>()));
    expect(await banc.session.estOuverte(), isFalse);
  });

  test('16. déconnexion pendant un renouvellement : paire non enregistrée',
      () async {
    final retenue = Completer<void>();
    final renouvellementRecu = Completer<void>();
    final banc = _Banc((r) async {
      if (r.url.path == _renouveler) {
        renouvellementRecu.complete();
        await retenue.future;
        return _paire(2);
      }
      if (r.url.path == _deconnexion) return http.Response('', 204);
      return _refus();
    });
    final requete = banc.moi();
    await renouvellementRecu.future;
    await banc.session.deconnecter();
    retenue.complete();
    await expectLater(requete, throwsA(isA<SessionFermee>()));
    expect(await banc.stockage.lireJetonAcces(), isNull);
    expect(await banc.stockage.lireJetonRenouvellement(), isNull);
  });

  test('17. aucun jeton enregistré : session fermée, sans requête',
      () async {
    FlutterSecureStorage.setMockInitialValues({});
    final banc = _Banc((_) async => throw StateError('aucune requête'));
    await expectLater(banc.moi(), throwsA(isA<SessionFermee>()));
    expect(banc.requetes, isEmpty);
  });

  test("18. déconnexion pendant l'écriture de la paire : estOuverte faux",
      () async {
    final memoire = _StockageRetenu(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc(
      (r) async {
        if (r.url.path == _renouveler) return _paire(2);
        if (r.url.path == _deconnexion) return http.Response('', 204);
        return _refus();
      },
      stockage: StockageJeton(stockage: memoire),
    );
    memoire.retenueAcces = Completer<void>();
    final requete = banc.moi();
    // Le jeton de renouvellement est écrit, l'écriture du jeton d'accès est
    // retenue : la déconnexion arrive entre les deux.
    await memoire.ecritureAccesCommencee.future;
    await banc.session.deconnecter();
    final fermeturesApresDeconnexion = banc.fermetures;
    memoire.retenueAcces!.complete();
    await requete.then((_) {}, onError: (_) {});
    expect(await banc.session.estOuverte(), isFalse);
    expect(memoire.valeurs, isEmpty);
    expect(banc.fermetures, fermeturesApresDeconnexion);
  });
}
