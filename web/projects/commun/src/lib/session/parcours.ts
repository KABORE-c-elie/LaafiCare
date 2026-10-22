import { InjectionToken } from '@angular/core';

/**
 * Routes de l'API et pages du parcours de connexion d'une application. Les
 * écrans communs (connexion, second facteur) et `AccesService` les lisent :
 * le même code sert aux professionnels (docs/api.md §2.4) et aux
 * administrateurs LaafiCare (§2.5).
 */
export interface ParcoursConnexion {
  /** Sorte de compte : décide des réponses attendues et de la session ouverte. */
  sorte: 'professionnel' | 'administrateur';
  routeConnexion: string;
  routeSecondFacteur: string;
  pageConnexion: string;
  pageSecondFacteur: string;
  /** Page du réenrôlement d'un administrateur sans second facteur (§2.5) ; `null` pour les professionnels. */
  pageReenrolement: string | null;
  /** Page ouverte quand la session est ouverte. */
  pageApresConnexion: string;
}

/** Parcours professionnel (§2.4, §2.6) : après la connexion, le choix de l'espace. */
export const PARCOURS_PROFESSIONNEL: ParcoursConnexion = {
  sorte: 'professionnel',
  routeConnexion: '/api/professionnels/connexion',
  routeSecondFacteur: '/api/professionnels/connexion/second-facteur',
  pageConnexion: '/connexion',
  pageSecondFacteur: '/second-facteur',
  pageReenrolement: null,
  pageApresConnexion: '/espaces',
};

/** Parcours administrateur (§2.5, §2.9) : second facteur toujours, puis les demandes de structure. */
export const PARCOURS_ADMINISTRATEUR: ParcoursConnexion = {
  sorte: 'administrateur',
  routeConnexion: '/api/administrateurs/connexion',
  routeSecondFacteur: '/api/administrateurs/connexion/second-facteur',
  pageConnexion: '/connexion',
  pageSecondFacteur: '/second-facteur',
  pageReenrolement: '/reenrolement',
  pageApresConnexion: '/structures',
};

/** Parcours de l'application : professionnel par défaut ; l'administration fournit le sien. */
export const PARCOURS_CONNEXION = new InjectionToken<ParcoursConnexion>('PARCOURS_CONNEXION', {
  providedIn: 'root',
  factory: () => PARCOURS_PROFESSIONNEL,
});
