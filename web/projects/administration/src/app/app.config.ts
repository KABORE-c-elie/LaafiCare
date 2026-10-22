import { APP_BASE_HREF } from '@angular/common';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { ApplicationConfig, LOCALE_ID, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideRouter, withComponentInputBinding } from '@angular/router';
import {
  ESPACE_APPLICATION,
  GESTION_SESSION,
  PARCOURS_ADMINISTRATEUR,
  PARCOURS_CONNEXION,
  intercepteurAcces,
} from 'commun';
import { intercepteursDeveloppement } from '../environments/environment';
import { routes } from './app.routes';
import { ESPACE_ADMINISTRATION } from './espace';
import { GestionSessionAdministration } from './gestion-session-administration';

// Même configuration que l'application MUNASEB (zoneless, HttpClient sur
// fetch, jetons en mémoire seulement : option A), sans service worker (B11).
export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideRouter(routes, withComponentInputBinding()),
    provideHttpClient(withInterceptors([intercepteurAcces, ...intercepteursDeveloppement])),
    { provide: LOCALE_ID, useValue: 'fr' },
    // Pas de balise <base> (CSP `base-uri 'none'`) : angular.dev/api/common/APP_BASE_HREF.
    { provide: APP_BASE_HREF, useValue: '/' },
    { provide: GESTION_SESSION, useClass: GestionSessionAdministration },
    // Connexion administrateur (docs/api.md §2.5) : routes, pages et session de ce parcours.
    { provide: PARCOURS_CONNEXION, useValue: PARCOURS_ADMINISTRATEUR },
    { provide: ESPACE_APPLICATION, useValue: ESPACE_ADMINISTRATION },
  ],
};
