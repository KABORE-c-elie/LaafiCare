import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:provider/provider.dart';

import '../data/repositories/auth_repository.dart';
import '../l10n/app_localizations.dart';
import '../ui/connexion/connexion_ecran.dart';
import '../ui/connexion/connexion_view_model.dart';
import '../ui/core/widgets/ecran_a_venir.dart';
import '../ui/demarrage/demarrage_ecran.dart';
import 'routes.dart';

/// Redirection d'après l'état de la session ; null = rester sur [chemin].
///
/// Fonction pure, testée sans écran. Appelée par go_router avant chaque
/// navigation, et à chaque changement d'AuthRepository (refreshListenable).
String? rediriger(EtatSession etat, String chemin) => switch (etat) {
      // Stockage pas encore lu : écran de démarrage.
      EtatSession.inconnu => chemin == Routes.demarrage ? null : Routes.demarrage,
      EtatSession.deconnecte =>
        Routes.sansCompte.contains(chemin) ? null : Routes.connexion,
      EtatSession.connecte =>
        Routes.avantConnexion.contains(chemin) ? Routes.accueil : null,
    };

/// Routeur de l'application, d'après l'exemple officiel de go_router 18.0.2
/// (example/lib/redirection.dart) : redirect pour la session, et
/// refreshListenable (« changes on the listenable will cause the router to
/// refresh it's route ») : une session ouverte ou fermée relance la
/// redirection d'elle-même.
GoRouter creerRouteur(AuthRepository auth) => GoRouter(
      initialLocation: Routes.demarrage,
      refreshListenable: auth,
      redirect: (context, state) =>
          rediriger(auth.etat, state.matchedLocation),
      routes: [
        GoRoute(
          path: Routes.demarrage,
          builder: (context, state) => const DemarrageEcran(),
        ),
        GoRoute(
          path: Routes.connexion,
          // Le view model est créé avec l'écran et libéré à sa fermeture
          // (ChangeNotifierProvider, paquet provider).
          builder: (context, state) => ChangeNotifierProvider(
            create: (_) => ConnexionViewModel(auth: auth),
            child: Builder(
              builder: (context) => ConnexionEcran(
                vm: context.read<ConnexionViewModel>(),
                // push et non go : la page est empilée, le bouton retour
                // d'Android et la flèche de l'AppBar ramènent à la
                // connexion. Doc go_router (navigation.md) : go « will
                // replace the current stack of screens » ; push « can push
                // a screen onto the Navigator's history stack ».
                surMotDePasseOublie: () =>
                    context.push(Routes.motDePasseOublie),
                surCreerCompte: () => context.push(Routes.inscription),
                surUrgences: () => context.push(Routes.urgences),
              ),
            ),
          ),
        ),
        GoRoute(
          path: Routes.urgences,
          builder: (context, state) =>
              EcranAVenir(titre: Textes.of(context).urgencesTitre),
        ),
        GoRoute(
          path: Routes.inscription,
          builder: (context, state) =>
              EcranAVenir(titre: Textes.of(context).boutonCreerCompte),
        ),
        GoRoute(
          path: Routes.motDePasseOublie,
          builder: (context, state) =>
              EcranAVenir(titre: Textes.of(context).motDePasseOublie),
        ),
        GoRoute(
          path: Routes.accueil,
          // Accueil provisoire : le bouton « Se déconnecter » permet de
          // tester le parcours complet sur le téléphone (décision du
          // porteur). La redirection ramène ensuite à la connexion.
          builder: (context, state) => EcranAVenir(
            titre: Textes.of(context).accueilTitre,
            action: FilledButton(
              onPressed: auth.deconnecter,
              child: Text(Textes.of(context).boutonSeDeconnecter),
            ),
          ),
        ),
      ],
    );
