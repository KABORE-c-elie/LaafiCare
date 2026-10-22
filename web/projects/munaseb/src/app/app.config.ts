import { APP_BASE_HREF } from '@angular/common';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { ApplicationConfig, LOCALE_ID, isDevMode, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideServiceWorker } from '@angular/service-worker';
import { provideRouter, withComponentInputBinding } from '@angular/router';
import { ESPACE_APPLICATION, GESTION_SESSION, intercepteurAcces } from 'commun';
import { routes } from './app.routes';
import { ESPACE_MUNASEB } from './espace';
import { intercepteursDeveloppement } from '../environments/environment';
import { GestionSessionMunaseb } from './gestion-session-munaseb';

// Zoneless : défaut d'Angular depuis la v21 (angular.dev/guide/zoneless) ; ni
// zone.js ni provideZoneChangeDetection. HttpClient passe par fetch, son
// moteur par défaut (withFetch est déprécié : « FetchBackend is the default »).
export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    // withComponentInputBinding : le paramètre `:id` arrive dans l'entrée `id` de la page de détail.
    provideRouter(routes, withComponentInputBinding()),
    // Le journal de développement passe après l'intercepteur d'accès : il voit
    // chaque erreur du serveur, y compris le 401 qui déclenche un renouvellement.
    provideHttpClient(withInterceptors([intercepteurAcces, ...intercepteursDeveloppement])),
    { provide: LOCALE_ID, useValue: 'fr' },
    // Sans balise <base> (interdite par la CSP `base-uri 'none'`), le routeur
    // reçoit la base des adresses par APP_BASE_HREF (angular.dev/api/common/APP_BASE_HREF).
    { provide: APP_BASE_HREF, useValue: '/' },
    { provide: GESTION_SESSION, useClass: GestionSessionMunaseb },
    // Panneau d'accueil des écrans communs (commun/connexion). Le parcours de
    // connexion reste le parcours professionnel, valeur par défaut de PARCOURS_CONNEXION.
    { provide: ESPACE_APPLICATION, useValue: ESPACE_MUNASEB },
    // Service worker officiel (angular.dev/ecosystem/service-workers) : met en cache les
    // fichiers de l'application seulement (ngsw-config.json, aucun `dataGroups` :
    // jamais les réponses de /api). Désactivé en développement.
    provideServiceWorker('ngsw-worker.js', {
      enabled: !isDevMode(),
      registrationStrategy: 'registerWhenStable:30000',
    }),
  ],
};
