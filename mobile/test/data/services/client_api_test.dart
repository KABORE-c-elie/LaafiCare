import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/domain/models/erreur_api.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

ClientApi _client(
  http.Client client, {
  Duration delai = const Duration(seconds: 30),
  void Function()? surSessionExpiree,
}) =>
    ClientApi(
      configuration: _configuration,
      client: client,
      delaiAttente: delai,
      surSessionExpiree: surSessionExpiree,
    );

/// Réponse JSON encodée en UTF-8, sans charset dans Content-Type.
http.Response _json(int statut, Object corps) => http.Response.bytes(
    utf8.encode(jsonEncode(corps)), statut,
    headers: {'content-type': 'application/json'});

void main() {
  test('POST : chemin, corps JSON, Content-Type, sans Authorization ; 204',
      () async {
    late http.Request recue;
    final client = _client(MockClient((requete) async {
      recue = requete;
      return http.Response('', 204);
    }));

    final reponse = await client.post('/api/patients/otp',
        corps: {'telephone': '70 00 00 00'});

    expect(recue.method, 'POST');
    expect(recue.url.toString(), 'http://127.0.0.1:8080/api/patients/otp');
    expect(recue.headers['Content-Type'], startsWith('application/json'));
    expect(recue.headers.containsKey('Authorization'), isFalse);
    expect(jsonDecode(recue.body), {'telephone': '70 00 00 00'});
    expect(reponse.statut, 204);
    expect(reponse.json, isNull);
  });

  test('200 : corps JSON décodé en UTF-8 (accents intacts)', () async {
    final client = _client(MockClient(
        (_) async => _json(200, {'jeton': 'j', 'lieu': 'Koudougou é'})));
    final reponse = await client.post('/api/patients/connexion', corps: {});
    expect(reponse.json, {'jeton': 'j', 'lieu': 'Koudougou é'});
  });

  test('GET avec jeton : en-tête Authorization: Bearer', () async {
    late http.Request recue;
    final client = _client(MockClient((requete) async {
      recue = requete;
      return _json(200, {});
    }));
    await client.get('/api/patients/moi', jeton: 'abc');
    expect(recue.headers['Authorization'], 'Bearer abc');
  });

  test("erreur de l'API : ErreurApi avec le code", () async {
    final client = _client(MockClient((_) async =>
        _json(422, {'erreur': 'x', 'code': 'telephone_invalide'})));
    await expectLater(
      client.post('/api/patients/otp', corps: {}),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.telephoneInvalide)),
    );
  });

  test('401 jeton_invalide avec jeton : session expirée signalée', () async {
    var appels = 0;
    final client = _client(
      MockClient((_) async =>
          _json(401, {'erreur': 'x', 'code': 'jeton_invalide'})),
      surSessionExpiree: () => appels++,
    );
    await expectLater(
      client.get('/api/patients/moi', jeton: 'abc'),
      throwsA(isA<ErreurApi>()),
    );
    expect(appels, 1);
  });

  test('401 identifiants_invalides sans jeton : pas de session expirée',
      () async {
    var appels = 0;
    final client = _client(
      MockClient((_) async =>
          _json(401, {'erreur': 'x', 'code': 'identifiants_invalides'})),
      surSessionExpiree: () => appels++,
    );
    await expectLater(
      client.post('/api/patients/connexion', corps: {}),
      throwsA(isA<ErreurApi>()),
    );
    expect(appels, 0);
  });

  test('délai dépassé : requête interrompue, ErreurReseau(delaiDepasse)',
      () async {
    // MockClient.streaming reçoit la requête d'origine (AbortableRequest) ;
    // MockClient simple en recrée une sans abortTrigger. Et MockClient
    // n'interrompt pas lui-même : « it is the handler's responsibility to
    // throw [RequestAbortedException] » (doc de MockClient).
    final client = _client(
      MockClient.streaming((requete, _) async {
        await (requete as http.Abortable).abortTrigger;
        throw http.RequestAbortedException(requete.url);
      }),
      delai: const Duration(milliseconds: 10),
    );
    await expectLater(
      client.post('/api/patients/otp', corps: {}),
      throwsA(isA<ErreurReseau>()
          .having((e) => e.delaiDepasse, 'delaiDepasse', isTrue)),
    );
  });

  test('panne réseau (ClientException) : ErreurReseau sans délai dépassé',
      () async {
    final client = _client(MockClient((requete) async =>
        throw http.ClientException('injoignable', requete.url)));
    await expectLater(
      client.post('/api/patients/otp', corps: {}),
      throwsA(isA<ErreurReseau>()
          .having((e) => e.delaiDepasse, 'delaiDepasse', isFalse)),
    );
  });

  test('échec TLS ou socket non converti par IOClient : ErreurReseau',
      () async {
    for (final exception in <IOException>[
      const HandshakeException('poignée de main TLS refusée'),
      const CertificateException('certificat invalide'),
      const TlsException('erreur TLS'),
      const SocketException('connexion coupée pendant le corps'),
    ]) {
      final client = _client(MockClient((_) async => throw exception));
      await expectLater(
        client.post('/api/patients/otp', corps: {}),
        throwsA(isA<ErreurReseau>()
            .having((e) => e.delaiDepasse, 'delaiDepasse', isFalse)),
        reason: exception.runtimeType.toString(),
      );
    }
  });

  test("une erreur de programmation n'est pas déguisée en ErreurReseau",
      () async {
    final client =
        _client(MockClient((_) async => throw StateError('bogue')));
    await expectLater(
      client.post('/api/patients/otp', corps: {}),
      throwsA(isA<StateError>()),
    );
  });

  test("200 dont le corps n'est pas un objet JSON : reponse_illisible",
      () async {
    final client =
        _client(MockClient((_) async => http.Response('<html></html>', 200)));
    await expectLater(
      client.get('/api/patients/moi', jeton: 'abc'),
      throwsA(isA<ErreurApi>()
          .having((e) => e.code, 'code', CodesErreur.reponseIllisible)),
    );
  });
}
