import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { SessionService } from 'commun';

// Gardes fonctionnelles (angular.dev/guide/routing/route-guards). Elles ne
// protègent pas les données : c'est le serveur qui refuse un jeton (docs/api.md
// §1.2, §2.9). Elles évitent d'afficher un écran qui ne peut pas fonctionner.

function exigerEtat(etat: ReturnType<SessionService['etat']>): CanActivateFn {
  return () => inject(SessionService).etat() === etat || inject(Router).createUrlTree(['/connexion']);
}

/** Écran du second facteur : seulement avec un jeton intermédiaire. */
export const exigerSecondFacteur: CanActivateFn = exigerEtat('second_facteur');

/** Réenrôlement : seulement avec un jeton de réenrôlement (§2.5). */
export const exigerReenrolement: CanActivateFn = exigerEtat('reenrolement');

/** Pages de l'équipe : seulement avec une session administrateur (§2.5, §2.9). */
export const exigerAdministrateur: CanActivateFn = exigerEtat('administrateur');
