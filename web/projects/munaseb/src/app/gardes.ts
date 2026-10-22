import { inject } from '@angular/core';
import { CanActivateFn, Router } from '@angular/router';
import { SessionService } from 'commun';
import { RoleMunaseb } from './roles';

// Gardes fonctionnelles (angular.dev/guide/routing/route-guards). Elles ne
// protègent pas les données : c'est le serveur qui refuse un jeton (docs/api.md
// §1.2). Elles évitent d'afficher un écran qui ne peut pas fonctionner.

/** Écran du second facteur : seulement avec un jeton intermédiaire. */
export const exigerSecondFacteur: CanActivateFn = () => {
  const session = inject(SessionService);
  return session.etat() === 'second_facteur' || inject(Router).createUrlTree(['/connexion']);
};

/** Choix de l'espace : connecté, avec ou sans affectation (§2.6). */
export const exigerConnecte: CanActivateFn = () => {
  const session = inject(SessionService);
  return session.connecte() || inject(Router).createUrlTree(['/connexion']);
};

/**
 * Pages d'un rôle (à placer après `exigerEnExercice`) : l'affectation active
 * doit avoir ce rôle, sinon retour au choix de l'espace. Le serveur refuse de
 * toute façon un autre rôle (403 `acces_refuse`, docs/api.md §2.10).
 */
export function exigerRole(role: RoleMunaseb): CanActivateFn {
  return () => {
    const session = inject(SessionService);
    return session.affectation()?.role === role || inject(Router).createUrlTree(['/espaces']);
  };
}

/** Pages métier : jamais sans jeton en exercice. */
export const exigerEnExercice: CanActivateFn = () => {
  const session = inject(SessionService);
  if (session.etat() === 'en_exercice') {
    return true;
  }
  return inject(Router).createUrlTree([session.connecte() ? '/espaces' : '/connexion']);
};
