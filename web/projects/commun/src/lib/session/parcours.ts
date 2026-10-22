import { InjectionToken } from '@angular/core';

/**
 * Routes de l'API et pages du parcours de connexion d'une application. Les
 * écrans communs (connexion, second facteur) et `AccesService` les lisent :
 * le même code sert aux professionnels (docs/api.md §2.4) et aux
 * administrateurs LaafiCare (§2.5).
 */
export interface ParcoursConnexion {
  routeConnexion: string;
  routeSecondFacteur: string;
  pageConnexion: string;
  pageSecondFacteur: string;
  /** Page ouverte quand la session est ouverte. */
  pageApresConnexion: string;
}

/** Parcours professionnel (§2.4, §2.6) : après la connexion, le choix de l'espace. */
export const PARCOURS_PROFESSIONNEL: ParcoursConnexion = {
  routeConnexion: '/api/professionnels/connexion',
  routeSecondFacteur: '/api/professionnels/connexion/second-facteur',
  pageConnexion: '/connexion',
  pageSecondFacteur: '/second-facteur',
  pageApresConnexion: '/espaces',
};

/** Parcours de l'application : professionnel par défaut ; l'administration fournit le sien. */
export const PARCOURS_CONNEXION = new InjectionToken<ParcoursConnexion>('PARCOURS_CONNEXION', {
  providedIn: 'root',
  factory: () => PARCOURS_PROFESSIONNEL,
});
