import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patient/ui/core/theme/theme_laaficare.dart';
import 'package:patient/ui/core/widgets/logo_laafi.dart';
import 'package:patient/ui/core/widgets/message_erreur.dart';

Widget _dansApp(Widget enfant) => MaterialApp(
      theme: ThemeLaafiCare.clair,
      home: Scaffold(body: enfant),
    );

const _texte = 'Numéro ou mot de passe incorrect.';

void main() {
  testWidgets('1. MessageErreur affiche le texte et l\'icône info',
      (tester) async {
    await tester.pumpWidget(_dansApp(const MessageErreur(_texte)));
    expect(find.text(_texte), findsOneWidget);
    expect(find.byIcon(Icons.info_outline), findsOneWidget);
  });

  testWidgets('2. fond errorContainer, texte onErrorContainer (neutre)',
      (tester) async {
    await tester.pumpWidget(_dansApp(const MessageErreur(_texte)));
    final schema = ThemeLaafiCare.clair.colorScheme;
    final bloc = tester.widget<Container>(find.descendant(
        of: find.byType(MessageErreur), matching: find.byType(Container)));
    expect((bloc.decoration! as BoxDecoration).color, schema.errorContainer);
    expect(tester.widget<Text>(find.text(_texte)).style!.color,
        schema.onErrorContainer);
    expect(tester.widget<Icon>(find.byIcon(Icons.info_outline)).color,
        schema.onErrorContainer);
  });

  testWidgets("3. annoncé au lecteur d'écran, icône non lue", (tester) async {
    await tester.pumpWidget(_dansApp(const MessageErreur(_texte)));
    expect(
      find.byWidgetPredicate(
          (w) => w is Semantics && w.properties.liveRegion == true),
      findsOneWidget,
    );
    expect(
      find.descendant(
          of: find.byType(ExcludeSemantics),
          matching: find.byIcon(Icons.info_outline)),
      findsOneWidget,
    );
  });

  testWidgets('4. LogoLaafi : SVG de la variante, à la hauteur demandée',
      (tester) async {
    await tester.pumpWidget(_dansApp(const LogoLaafi(
        variante: VarianteLogo.verticalSurVert,
        hauteur: 64,
        libelle: 'LaafiCare')));
    final svg = tester.widget<SvgPicture>(find.byType(SvgPicture));
    expect(svg.height, 64);
    expect((svg.bytesLoader as SvgAssetLoader).assetName,
        'assets/branding/logo_vertical_sur_vert.svg');
  });

  testWidgets("5. logo lu par le lecteur d'écran sous le nom donné",
      (tester) async {
    final semantique = tester.ensureSemantics();
    await tester.pumpWidget(_dansApp(const LogoLaafi(libelle: 'LaafiCare')));
    expect(find.bySemanticsLabel('LaafiCare'), findsOneWidget);
    semantique.dispose();
  });

  test('6. chaque variante pointe vers un fichier déclaré', () async {
    TestWidgetsFlutterBinding.ensureInitialized();
    for (final variante in VarianteLogo.values) {
      final donnees = await rootBundle.load(variante.chemin);
      expect(donnees.lengthInBytes, greaterThan(0), reason: variante.name);
    }
  });
}
