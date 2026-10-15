import 'dart:async';
import 'dart:convert';

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
import 'package:patient/ui/core/widgets/logo_laafi.dart';
import 'package:patient/ui/core/widgets/message_erreur.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

const _texteIdentifiants = 'Numéro ou mot de passe incorrect. Après plusieurs '
    'essais, la connexion est bloquée quelques minutes : utilisez « Mot de '
    'passe oublié ? » si besoin.';

http.Response _json(int statut, Object corps) => http.Response.bytes(
    utf8.encode(jsonEncode(corps)), statut,
    headers: {'content-type': 'application/json'});

/// Écran branché sur un serveur simulé ; [actions] note les navigations.
class _Banc {
  _Banc(Future<http.Response> Function(http.Request r) serveur) {
    final client = ClientApi(
        configuration: _configuration, client: MockClient(serveur));
    vm = ConnexionViewModel(
      auth: AuthRepository(
        client: client,
        session:
            GestionnaireSession(client: client, stockage: StockageJeton()),
      ),
    );
  }

  late final ConnexionViewModel vm;
  final actions = <String>[];

  Widget get app => MaterialApp(
        theme: ThemeLaafiCare.clair,
        localizationsDelegates: Textes.localizationsDelegates,
        supportedLocales: Textes.supportedLocales,
        home: ConnexionEcran(
          vm: vm,
          surMotDePasseOublie: () => actions.add('oublie'),
          surCreerCompte: () => actions.add('creer'),
          surUrgences: () => actions.add('urgences'),
        ),
      );
}

Finder get _champTelephone => find.byType(TextField).at(0);
Finder get _champMotDePasse => find.byType(TextField).at(1);

Future<void> _appuyerSeConnecter(WidgetTester tester) async {
  await tester.ensureVisible(find.byType(FilledButton).first);
  await tester.tap(find.byType(FilledButton).first);
  await tester.pump();
}

InputDecoration _decoration(WidgetTester tester, Finder champ) => tester
    .widget<InputDecorator>(
        find.descendant(of: champ, matching: find.byType(InputDecorator)))
    .decoration;

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  testWidgets('1. textes de la maquette, logo, indicatif, carte d\'urgence',
      (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    for (final texte in [
      'Se connecter',
      'Avec votre numéro de téléphone et votre mot de passe.',
      'Numéro de téléphone',
      '70 00 00 00',
      'Mot de passe',
      'Mot de passe oublié ?',
      'Pas encore de compte ?',
      'Créer mon compte',
      "Numéros d'urgence",
      '15 SAMU, 18 pompiers, 17 police',
      '+226',
    ]) {
      expect(find.text(texte), findsWidgets, reason: texte);
    }
    expect(find.byType(LogoLaafi), findsOneWidget);
    expect(find.byIcon(Icons.phone), findsOneWidget);
  });

  testWidgets('2. bouton œil : masqué par défaut, puis affiché, puis masqué',
      (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    TextField mdp() => tester.widget<TextField>(_champMotDePasse);
    expect(mdp().obscureText, isTrue);
    expect(find.byTooltip('Afficher le mot de passe'), findsOneWidget);
    await tester.tap(find.byTooltip('Afficher le mot de passe'));
    await tester.pump();
    expect(mdp().obscureText, isFalse);
    expect(find.byTooltip('Masquer le mot de passe'), findsOneWidget);
    await tester.tap(find.byTooltip('Masquer le mot de passe'));
    await tester.pump();
    expect(mdp().obscureText, isTrue);
  });

  testWidgets('3. champs vides : message sous le téléphone, avec l\'icône',
      (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    await _appuyerSeConnecter(tester);
    expect(find.text('Saisissez votre numéro de téléphone.'), findsOneWidget);
    expect(find.byIcon(Icons.info_outline), findsOneWidget);
    // Le message est entre les deux champs.
    final yMessage = tester.getTopLeft(find.byType(MessageErreur)).dy;
    expect(yMessage, greaterThan(tester.getTopLeft(_champTelephone).dy));
    expect(yMessage, lessThan(tester.getTopLeft(_champMotDePasse).dy));
  });

  testWidgets('4. refus 401 : texte long sous le mot de passe, avec l\'icône',
      (tester) async {
    final banc = _Banc((_) async =>
        _json(401, {'erreur': 'x', 'code': 'identifiants_invalides'}));
    await tester.pumpWidget(banc.app);
    await tester.enterText(_champTelephone, '70 00 00 00');
    await tester.enterText(_champMotDePasse, 'Exemple#2026');
    await _appuyerSeConnecter(tester);
    await tester.pumpAndSettle();
    expect(find.text(_texteIdentifiants), findsOneWidget);
    expect(find.byIcon(Icons.info_outline), findsOneWidget);
    expect(tester.getTopLeft(find.byType(MessageErreur)).dy,
        greaterThan(tester.getTopLeft(_champMotDePasse).dy));
  });

  testWidgets('5. pendant l\'envoi : bouton désactivé, indicateur visible',
      (tester) async {
    final retenue = Completer<http.Response>();
    final banc = _Banc((_) => retenue.future);
    await tester.pumpWidget(banc.app);
    await tester.enterText(_champTelephone, '70 00 00 00');
    await tester.enterText(_champMotDePasse, 'Exemple#2026');
    await _appuyerSeConnecter(tester);
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byType(FilledButton).first)
        .onPressed, isNull);
    retenue.complete(http.Response('', 500));
    await tester.pumpAndSettle();
    expect(find.byType(CircularProgressIndicator), findsNothing);
  });

  testWidgets('6. une saisie efface le message d\'erreur', (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    await _appuyerSeConnecter(tester);
    expect(find.byType(MessageErreur), findsOneWidget);
    await tester.enterText(_champTelephone, '7');
    await tester.pump();
    expect(find.byType(MessageErreur), findsNothing);
  });

  testWidgets('7. les trois actions de navigation sont appelées',
      (tester) async {
    final banc = _Banc((_) async => http.Response('', 500));
    await tester.pumpWidget(banc.app);
    for (final texte in [
      'Mot de passe oublié ?',
      'Créer mon compte',
      "Numéros d'urgence",
    ]) {
      // Défilement jusqu'au texte, puis une image pour qu'il soit en place.
      await tester.ensureVisible(find.text(texte));
      await tester.pumpAndSettle();
      await tester.tap(find.text(texte));
    }
    expect(banc.actions, ['oublie', 'creer', 'urgences']);
  });

  testWidgets('8. lien « Mot de passe oublié ? » : au moins 48 × 48',
      (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    final taille = tester.getSize(find.ancestor(
        of: find.text('Mot de passe oublié ?'),
        matching: find.byType(TextButton)));
    expect(taille.width, greaterThanOrEqualTo(48));
    expect(taille.height, greaterThanOrEqualTo(48));
  });

  testWidgets('9. clavier téléphone', (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    expect(tester.widget<TextField>(_champTelephone).keyboardType,
        TextInputType.phone);
  });

  testWidgets('10. titre marqué comme en-tête', (tester) async {
    final semantique = tester.ensureSemantics();
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    final titre = find.descendant(
        of: find.byType(Semantics), matching: find.text('Se connecter'));
    expect(
      tester.getSemantics(titre.first),
      isSemantics(label: 'Se connecter', isHeader: true),
    );
    semantique.dispose();
  });

  testWidgets('11. chaque champ est lu avec son nom, le libellé une seule '
      'fois', (tester) async {
    final semantique = tester.ensureSemantics();
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    for (final (champ, nom) in [
      (_champTelephone, 'Numéro de téléphone'),
      (_champMotDePasse, 'Mot de passe'),
    ]) {
      final noeud = tester.getSemantics(champ);
      expect(noeud.label, contains(nom), reason: nom);
      expect(noeud.flagsCollection.isTextField, isTrue, reason: nom);
      // Un seul nœud porte ce nom, et c'est le champ : le libellé visible
      // n'est pas lu une seconde fois.
      // Nom exact sur une ligne du libellé (« Mot de passe oublié ? » ne
      // compte pas).
      final porteurs = find.bySemanticsLabel(
          RegExp('(^|\n)${RegExp.escape(nom)}(\n|\$)'));
      expect(porteurs, findsOneWidget, reason: nom);
      expect(tester.getSemantics(porteurs).flagsCollection.isTextField, isTrue,
          reason: nom);
    }
    semantique.dispose();
  });

  testWidgets('12. champ en erreur : bord errorBorder, sans texte Material, '
      'même hauteur', (tester) async {
    await tester.pumpWidget(_Banc((_) async => http.Response('', 500)).app);
    final hauteurAvant = tester.getSize(_champTelephone).height;
    await _appuyerSeConnecter(tester);
    final telephone = _decoration(tester, _champTelephone);
    expect(telephone.error, isNotNull);
    expect(telephone.errorText, isNull);
    expect(telephone.errorBorder,
        ThemeLaafiCare.clair.inputDecorationTheme.errorBorder);
    final bord = telephone.errorBorder! as OutlineInputBorder;
    expect(bord.borderSide.width, 2);
    expect(bord.borderSide.color, ThemeLaafiCare.schema.error);
    expect(_decoration(tester, _champMotDePasse).error, isNull);
    expect(tester.getSize(_champTelephone).height, hauteurAvant);
  });
}
