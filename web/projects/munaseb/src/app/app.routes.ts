import { Routes } from '@angular/router';
import { exigerConnecte, exigerEnExercice, exigerSecondFacteur } from './gardes';

// Chargement différé des pages (loadComponent, angular.dev/guide/routing/loading-strategies),
// sauf la connexion, première page vue.
export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'connexion' },
  {
    path: 'connexion',
    title: 'Connexion · Assurance MUNASEB',
    loadComponent: () => import('./pages/connexion').then((m) => m.ConnexionPage),
  },
  {
    path: 'second-facteur',
    title: 'Code de vérification · Assurance MUNASEB',
    canActivate: [exigerSecondFacteur],
    loadComponent: () => import('./pages/second-facteur').then((m) => m.SecondFacteurPage),
  },
  {
    path: 'espaces',
    title: 'Choix de l’espace · Assurance MUNASEB',
    canActivate: [exigerConnecte],
    loadComponent: () => import('./pages/espaces').then((m) => m.EspacesPage),
  },
  {
    path: 'demandes',
    title: 'Demandes de remboursement · Assurance MUNASEB',
    canActivate: [exigerEnExercice],
    loadComponent: () => import('./pages/demandes').then((m) => m.DemandesPage),
  },
  {
    path: 'demandes/:id',
    title: 'Détail de la demande · Assurance MUNASEB',
    canActivate: [exigerEnExercice],
    loadComponent: () => import('./pages/detail').then((m) => m.DetailPage),
  },
  { path: '**', redirectTo: 'connexion' },
];
