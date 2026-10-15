import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/repositories/auth_repository.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/data/services/gestionnaire_session.dart';
import 'package:patient/data/services/stockage_jeton.dart';
import 'package:patient/l10n/app_localizations.dart';
import 'package:patient/routing/routes.dart';
import 'package:patient/routing/routeur.dart';
import 'package:patient/ui/connexion/connexion_ecran.dart';
import 'package:patient/ui/core/theme/theme_laaficare.dart';
import 'package:patient/ui/demarrage/demarrage_ecran.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

http.Response _json(int statut, Object corps) => http.Response.bytes(
    utf8.encode(jsonEncode(corps)), statut,
    headers: {'content-type': 'application/json'});

http.Response _ouverture() => _json(200, {
      'etape': 'complete',
      'jeton_acces': 'acces-1',
      'expire_dans': 900,
      'jeton_renouvellement': 'renouv-1',
    });

/// Application complète (routeur, thème, traductions) sur un serveur simulé.
class _Banc {
  _Banc(Future<http.Response> Function(http.Request r) serveur) {
    client = ClientApi(
        configuration: _configuration, client: MockClient(serveur));
    session = GestionnaireSession(client: client, stockage: StockageJeton());
    auth = AuthRepository(client: client, session: session);
    routeur = creerRouteur(auth);
  }

  late final ClientApi client;
  late final GestionnaireSession session;
  late final AuthRepository auth;
  late final GoRouter routeur;

  Widget get app => MaterialApp.router(
        routerConfig: routeur,
        theme: ThemeLaafiCare.clair,
        localizationsDelegates: Textes.localizationsDelegates,
        supportedLocales: Textes.supportedLocales,
      );

  /// Page du dessus. go_router 18.0.2, GoRouter.state : « The top
  /// [GoRouterState], the state of the route that was last used in either
  /// [GoRouter.go] or [GoRouter.push] ».
  String get chemin => routeur.state.matchedLocation;
}

Future<void> _toucher(WidgetTester tester, Finder cible) async {
  await tester.ensureVisible(cible);
  await tester.pumpAndSettle();
  await tester.tap(cible);
  await tester.pumpAndSettle();
}

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('1. redirection : état × chemin', () {
    const tous = [
      Routes.demarrage,
      Routes.connexion,
      Routes.urgences,
      Routes.inscription,
      Routes.motDePasseOublie,
      Routes.accueil,
    ];
    for (final chemin in tous) {
      expect(rediriger(EtatSession.inconnu, chemin),
          chemin == Routes.demarrage ? null : Routes.demarrage,
          reason: 'inconnu $chemin');
    }
    expect(rediriger(EtatSession.deconnecte, Routes.demarrage),
        Routes.connexion);
    expect(rediriger(EtatSession.deconnecte, Routes.accueil),
        Routes.connexion);
    for (final chemin in Routes.sansCompte) {
      expect(rediriger(EtatSession.deconnecte, chemin), isNull,
          reason: 'déconnecté $chemin');
    }
    for (final chemin in Routes.avantConnexion) {
      expect(rediriger(EtatSession.connecte, chemin), Routes.accueil,
          reason: 'connecté $chemin');
    }
    expect(rediriger(EtatSession.connecte, Routes.accueil), isNull);
    expect(rediriger(EtatSession.connecte, Routes.urgences), isNull);
  });

  testWidgets('2. état inconnu : écran de démarrage', (tester) async {
    final banc = _Banc((_) async => http.Response('', 500));
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    expect(find.byType(DemarrageEcran), findsOneWidget);
    expect(banc.chemin, Routes.demarrage);
  });

  testWidgets('3. déconnecté : /accueil mène à /connexion, /urgences reste '
      'ouvert', (tester) async {
    final banc = _Banc((_) async => http.Response('', 500));
    await banc.auth.initialiser();
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    expect(banc.chemin, Routes.connexion);
    banc.routeur.go(Routes.accueil);
    await tester.pumpAndSettle();
    expect(banc.chemin, Routes.connexion);
    banc.routeur.go(Routes.urgences);
    await tester.pumpAndSettle();
    expect(banc.chemin, Routes.urgences);
  });

  testWidgets('4. connexion réussie : passage automatique à /accueil',
      (tester) async {
    final banc = _Banc((_) async => _ouverture());
    await banc.auth.initialiser();
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).at(0), '70 00 00 00');
    await tester.enterText(find.byType(TextField).at(1), 'Exemple#2026');
    await _toucher(tester, find.widgetWithText(FilledButton, 'Se connecter'));
    expect(banc.chemin, Routes.accueil);
    expect(find.text('Accueil'), findsOneWidget);
  });

  testWidgets('5. session fermée par le serveur : retour à /connexion',
      (tester) async {
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc(
        (_) async => _json(401, {'erreur': 'x', 'code': 'jeton_invalide'}));
    await banc.auth.initialiser();
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    expect(banc.chemin, Routes.accueil);
    await expectLater(
      banc.session.requete(
          (jeton) => banc.client.get('/api/patients/moi', jeton: jeton)),
      throwsA(isA<SessionFermee>()),
    );
    await tester.pumpAndSettle();
    expect(banc.chemin, Routes.connexion);
    expect(find.byType(ConnexionEcran), findsOneWidget);
  });

  testWidgets('6. les trois actions de la connexion empilent leur page, '
      'avec une flèche retour', (tester) async {
    final banc = _Banc((_) async => http.Response('', 500));
    await banc.auth.initialiser();
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    for (final (texte, chemin) in [
      ('Mot de passe oublié ?', Routes.motDePasseOublie),
      ('Créer mon compte', Routes.inscription),
      ("Numéros d'urgence", Routes.urgences),
    ]) {
      await _toucher(tester, find.text(texte));
      expect(banc.chemin, chemin, reason: texte);
      expect(find.byType(BackButton), findsOneWidget, reason: texte);
      await _toucher(tester, find.byType(BackButton));
      expect(banc.chemin, Routes.connexion, reason: texte);
    }
  });

  testWidgets('7. « Se déconnecter » : jetons effacés, retour à /connexion',
      (tester) async {
    FlutterSecureStorage.setMockInitialValues(
        {'jeton_acces': 'acces-1', 'jeton_renouvellement': 'renouv-1'});
    final banc = _Banc((_) async => http.Response('', 204));
    await banc.auth.initialiser();
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    await _toucher(tester, find.text('Se déconnecter'));
    expect(banc.chemin, Routes.connexion);
    expect(await StockageJeton().lireJetonAcces(), isNull);
  });

  testWidgets('8. depuis /connexion, /urgences puis retour : /connexion',
      (tester) async {
    final banc = _Banc((_) async => http.Response('', 500));
    await banc.auth.initialiser();
    await tester.pumpWidget(banc.app);
    await tester.pumpAndSettle();
    await _toucher(tester, find.text("Numéros d'urgence"));
    expect(banc.chemin, Routes.urgences);
    // Bouton retour du système (Android) : le moteur appelle
    // WidgetsBinding.handlePopRoute, simulé ici.
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(banc.chemin, Routes.connexion);
    expect(find.byType(ConnexionEcran), findsOneWidget);
  });
}
