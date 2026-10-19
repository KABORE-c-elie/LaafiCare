import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { ApplicationConfig, LOCALE_ID, provideBrowserGlobalErrorListeners } from '@angular/core';
import { provideRouter } from '@angular/router';
import { GESTION_SESSION, intercepteurAcces } from 'commun';
import { routes } from './app.routes';
import { GestionSessionMunaseb } from './gestion-session-munaseb';

// Zoneless : défaut d'Angular depuis la v21 (angular.dev/guide/zoneless) ; ni
// zone.js ni provideZoneChangeDetection. HttpClient passe par fetch, son
// moteur par défaut (withFetch est déprécié : « FetchBackend is the default »).
export const appConfig: ApplicationConfig = {
  providers: [
    provideBrowserGlobalErrorListeners(),
    provideRouter(routes),
    provideHttpClient(withInterceptors([intercepteurAcces])),
    { provide: LOCALE_ID, useValue: 'fr' },
    { provide: GESTION_SESSION, useClass: GestionSessionMunaseb },
  ],
};
