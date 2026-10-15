import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:http/http.dart' as http;

import 'config/configuration.dart';
import 'data/repositories/auth_repository.dart';
import 'data/services/client_api.dart';
import 'data/services/gestionnaire_session.dart';
import 'data/services/stockage_jeton.dart';
import 'l10n/app_localizations.dart';
import 'routing/routeur.dart';
import 'ui/core/theme/theme_laaficare.dart';

void main() {
  // Le stockage sécurisé passe par des canaux de plateforme : le moteur doit
  // être prêt avant la première lecture.
  WidgetsFlutterBinding.ensureInitialized();

  final Configuration configuration;
  try {
    configuration = Configuration.charger();
  } on ErreurConfiguration catch (erreur) {
    // Adresse de l'API absente ou refusée (ex. http en version publiée) :
    // l'application ne démarre pas, et dit pourquoi.
    runApp(_ErreurDemarrage(erreur.message));
    return;
  }

  final client =
      ClientApi(configuration: configuration, client: http.Client());
  final auth = AuthRepository(
    client: client,
    session: GestionnaireSession(client: client, stockage: StockageJeton()),
  );
  runApp(ApplicationLaafiCare(auth: auth));
  // Lecture de la session pendant l'écran de démarrage : le routeur quitte
  // « / » dès que l'état est connu (refreshListenable). En cas d'erreur
  // imprévue, l'état passe à « déconnecté » (AuthRepository.initialiser).
  unawaited(auth.initialiser());
}

class ApplicationLaafiCare extends StatefulWidget {
  const ApplicationLaafiCare({super.key, required this.auth});

  final AuthRepository auth;

  @override
  State<ApplicationLaafiCare> createState() => _ApplicationLaafiCareState();
}

class _ApplicationLaafiCareState extends State<ApplicationLaafiCare> {
  // Créé une seule fois : recréer le GoRouter à chaque reconstruction
  // perdrait la pile des pages.
  late final GoRouter _routeur = creerRouteur(widget.auth);

  @override
  void dispose() {
    _routeur.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => MaterialApp.router(
        routerConfig: _routeur,
        onGenerateTitle: (context) => Textes.of(context).nomApplication,
        theme: ThemeLaafiCare.clair,
        localizationsDelegates: Textes.localizationsDelegates,
        supportedLocales: Textes.supportedLocales,
        // Bandeau « DEBUG » masqué. Doc de
        // MaterialApp.debugShowCheckedModeBanner : « This is on by default
        // (in debug mode), to turn it off, set the constructor argument to
        // false. » ; « In release mode this has no effect. »
        debugShowCheckedModeBanner: false,
      );
}

/// Configuration inutilisable : message pour la personne qui a lancé
/// l'application (développement), sans traduction ni thème.
class _ErreurDemarrage extends StatelessWidget {
  const _ErreurDemarrage(this.message);

  final String message;

  @override
  Widget build(BuildContext context) => MaterialApp(
        debugShowCheckedModeBanner: false,
        home: Scaffold(
          body: SafeArea(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Text(message),
            ),
          ),
        ),
      );
}
