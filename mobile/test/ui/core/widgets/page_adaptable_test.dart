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
import 'package:patient/l10n/app_localizations.dart';
import 'package:patient/ui/connexion/connexion_ecran.dart';
import 'package:patient/ui/connexion/connexion_view_model.dart';
import 'package:patient/ui/core/theme/theme_laaficare.dart';
import 'package:patient/ui/core/widgets/ecran_a_venir.dart';
import 'package:patient/ui/demarrage/demarrage_ecran.dart';

/// Tailles de fenêtre (points) et agrandissements du texte testés. 200 % :
/// WCAG 2.2, critère 1.4.4 « Resize Text ». Flutter fait échouer un test dès
/// qu'un débordement survient (« A RenderFlex overflowed »).
const _cas = <(String, Size, List<double>)>[
  ('petit téléphone', Size(320, 568), [1.0, 2.0]),
  ('téléphone du porteur', Size(328, 728), [1.0, 1.3, 2.0]),
  ('téléphone en paysage', Size(728, 328), [1.0, 2.0]),
  ('tablette en portrait', Size(800, 1280), [1.0]),
  ('tablette en paysage', Size(1280, 800), [1.0]),
];

Future<void> _fenetre(WidgetTester tester, Size taille, double texte) async {
  tester.view.devicePixelRatio = 1.0;
  tester.view.physicalSize = taille;
  tester.platformDispatcher.textScaleFactorTestValue = texte;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
}

Widget _app(Widget ecran) => MaterialApp(
      theme: ThemeLaafiCare.clair,
      localizationsDelegates: Textes.localizationsDelegates,
      supportedLocales: Textes.supportedLocales,
      home: ecran,
    );

ConnexionEcran _connexion() {
  final client = ClientApi(
    configuration:
        Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false),
    client: MockClient((_) async => http.Response('', 500)),
  );
  return ConnexionEcran(
    vm: ConnexionViewModel(
      auth: AuthRepository(
        client: client,
        session:
            GestionnaireSession(client: client, stockage: StockageJeton()),
      ),
    ),
    surMotDePasseOublie: () {},
    surCreerCompte: () {},
    surUrgences: () {},
  );
}

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  for (final (nom, taille, textes) in _cas) {
    for (final texte in textes) {
      final titre = '$nom ${taille.width.toInt()} × ${taille.height.toInt()}, '
          'texte ${(texte * 100).round()} %';

      testWidgets('connexion, $titre : aucun débordement, carte accessible',
          (tester) async {
        await _fenetre(tester, taille, texte);
        await tester.pumpWidget(_app(_connexion()));
        await tester.pumpAndSettle();
        final carte = find.text("Numéros d'urgence");
        await tester.ensureVisible(carte);
        await tester.pumpAndSettle();
        expect(carte.hitTestable(), findsOneWidget);
      });

      testWidgets('écran à venir et démarrage, $titre : aucun débordement',
          (tester) async {
        await _fenetre(tester, taille, texte);
        await tester.pumpWidget(_app(EcranAVenir(
          titre: 'Accueil',
          action: FilledButton(onPressed: () {}, child: const Text('Action')),
        )));
        await tester.pumpAndSettle();
        await tester.pumpWidget(_app(const DemarrageEcran()));
        await tester.pumpAndSettle();
      });
    }
  }

  testWidgets('place suffisante : carte des urgences collée en bas',
      (tester) async {
    await _fenetre(tester, const Size(328, 1000), 1.0);
    await tester.pumpWidget(_app(_connexion()));
    await tester.pumpAndSettle();
    final carte = find.ancestor(
        of: find.text("Numéros d'urgence"), matching: find.byType(Card));
    // Bas de l'écran moins la marge de 16.
    expect(tester.getBottomLeft(carte).dy, closeTo(1000 - 16, 1));
  });

  testWidgets('tablette : formulaire de 480 au plus, centré', (tester) async {
    await _fenetre(tester, const Size(1280, 800), 1.0);
    await tester.pumpWidget(_app(_connexion()));
    await tester.pumpAndSettle();
    final bouton = find.widgetWithText(FilledButton, 'Se connecter');
    expect(tester.getSize(bouton).width,
        lessThanOrEqualTo(Espacements.largeurMaxContenu));
    expect(tester.getCenter(bouton).dx, closeTo(1280 / 2, 1));
  });
}
