import { InjectionToken } from '@angular/core';

/**
 * Ce que l'application fait quand la session ne peut plus servir. La
 * bibliothèque ne connaît pas les routes de chaque application : chacune
 * fournit ce jeton d'injection.
 */
export interface GestionSession {
  /** Renouvellement refusé : retour à l'écran de connexion (docs/api.md §1.7). */
  sessionTerminee(): void;
  /** 403 `acces_refuse` d'une route métier : relire la liste des affectations (§2.6). */
  accesRetire(): void;
}

export const GESTION_SESSION = new InjectionToken<GestionSession>('GESTION_SESSION');
