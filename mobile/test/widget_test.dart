import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/repositories/auth_repository.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/data/services/gestionnaire_session.dart';
import 'package:patient/data/services/stockage_jeton.dart';
import 'package:patient/main.dart';
import 'package:patient/ui/connexion/connexion_ecran.dart';
import 'package:patient/ui/demarrage/demarrage_ecran.dart';

/// Stockage dont la lecture lève une erreur imprévue (pas une
/// PlatformException, déjà traitée par StockageJeton).
class _StockageEnPanne extends Fake implements FlutterSecureStorage {
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
      throw StateError('panne imprévue du stockage');
}

AuthRepository _auth({StockageJeton? stockage}) {
  final client = ClientApi(
    configuration:
        Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false),
    client: MockClient((_) async => http.Response('', 500)),
  );
  return AuthRepository(
    client: client,
    session: GestionnaireSession(
        client: client, stockage: stockage ?? StockageJeton()),
  );
}

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  testWidgets('démarrage, puis connexion sans jeton ; pas de bandeau DEBUG',
      (tester) async {
    final auth = _auth();
    await tester.pumpWidget(ApplicationLaafiCare(auth: auth));
    expect(find.byType(DemarrageEcran), findsOneWidget);
    await auth.initialiser();
    await tester.pumpAndSettle();
    expect(find.byType(ConnexionEcran), findsOneWidget);
    expect(
        tester.widget<MaterialApp>(find.byType(MaterialApp))
            .debugShowCheckedModeBanner,
        isFalse);
  });

  testWidgets('stockage en panne à la lecture : écran de connexion, erreur '
      'signalée', (tester) async {
    final auth =
        _auth(stockage: StockageJeton(stockage: _StockageEnPanne()));
    await tester.pumpWidget(ApplicationLaafiCare(auth: auth));
    await auth.initialiser();
    await tester.pumpAndSettle();
    expect(auth.etat, EtatSession.deconnecte);
    expect(find.byType(ConnexionEcran), findsOneWidget);
    // L'erreur n'est pas cachée : signalée par FlutterError.reportError.
    expect(tester.takeException(), isA<StateError>());
  });
}
