import { registerLocaleData } from '@angular/common';
import localeFr from '@angular/common/locales/fr';
import { bootstrapApplication } from '@angular/platform-browser';
import { appConfig } from './app/app.config';
import { App } from './app/app';

// Données de la locale française pour les pipes d'Angular (LOCALE_ID = 'fr',
// voir app.config.ts). Les dates passent par Intl (commun/format).
registerLocaleData(localeFr);

bootstrapApplication(App, appConfig).catch((err) => console.error(err));
