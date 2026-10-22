import { Routes } from '@angular/router';
import { exigerConnecte, exigerEnExercice, exigerRole, exigerSecondFacteur } from './gardes';

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
    // Lien reçu par email (docs/api.md §2.7) : accessible sans connexion, sans garde.
    path: 'verifier-email',
    title: 'Vérification de l’email · Assurance MUNASEB',
    loadComponent: () => import('./pages/verifier-email').then((m) => m.VerifierEmailPage),
  },
  {
    path: 'second-facteur',
    title: 'Code de vérification · Assurance MUNASEB',
    canActivate: [exigerSecondFacteur],
    // Écran commun (commun/connexion), parcours professionnel (PARCOURS_CONNEXION par défaut).
    loadComponent: () => import('commun/connexion').then((m) => m.SecondFacteurPage),
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
    canActivate: [exigerEnExercice, exigerRole('agent_assurance_munaseb')],
    loadComponent: () => import('./pages/demandes').then((m) => m.DemandesPage),
  },
  {
    path: 'demandes/:id',
    title: 'Détail de la demande · Assurance MUNASEB',
    canActivate: [exigerEnExercice, exigerRole('agent_assurance_munaseb')],
    loadComponent: () => import('./pages/detail').then((m) => m.DetailPage),
  },
  {
    path: 'invitations',
    title: 'Invitations · Assurance MUNASEB',
    canActivate: [exigerEnExercice, exigerRole('responsable')],
    loadComponent: () => import('./pages/invitations').then((m) => m.InvitationsPage),
  },
  { path: '**', redirectTo: 'connexion' },
];
