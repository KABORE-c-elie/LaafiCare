import { Routes } from '@angular/router';
import { exigerAdministrateur, exigerReenrolement, exigerSecondFacteur } from './gardes';

// Chargement différé des pages (loadComponent, angular.dev/guide/routing/loading-strategies).
export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'connexion' },
  {
    path: 'connexion',
    title: 'Connexion · Administration LaafiCare',
    loadComponent: () => import('./pages/connexion').then((m) => m.ConnexionPage),
  },
  {
    path: 'second-facteur',
    title: 'Code de vérification · Administration LaafiCare',
    canActivate: [exigerSecondFacteur],
    // Écran commun (commun/connexion) ; parcours administrateur (app.config.ts).
    loadComponent: () => import('commun/connexion').then((m) => m.SecondFacteurPage),
  },
  {
    path: 'reenrolement',
    title: 'Second facteur · Administration LaafiCare',
    canActivate: [exigerReenrolement],
    // La bibliothèque du QR code n'est chargée qu'avec cette page (décision B4).
    loadComponent: () => import('./pages/reenrolement').then((m) => m.ReenrolementPage),
  },
  {
    path: 'structures',
    title: 'Demandes de structure · Administration LaafiCare',
    canActivate: [exigerAdministrateur],
    loadComponent: () => import('./pages/structures').then((m) => m.StructuresPage),
  },
  {
    path: 'structures/:id',
    title: 'Demande de structure · Administration LaafiCare',
    canActivate: [exigerAdministrateur],
    loadComponent: () => import('./pages/structure').then((m) => m.StructurePage),
  },
  { path: '**', redirectTo: 'connexion' },
];
