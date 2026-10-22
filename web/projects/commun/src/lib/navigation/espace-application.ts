import { InjectionToken } from '@angular/core';

/** Ce qu'affiche le panneau d'accueil d'une application (fourni par chaque application). */
export interface EspaceApplication {
  /** Logo, en chemin absolu (pas de balise `<base>` : CSP `base-uri 'none'`). */
  logo: string;
  titre: string;
  /** Phrase sous le titre, sur l'écran de connexion seulement. */
  phrase: string;
}

/**
 * Dans le point d'entrée principal (et non dans `commun/connexion`) : il est
 * fourni par la configuration de l'application, chargée au démarrage, sans
 * tirer les écrans de connexion dans le paquet initial.
 */
export const ESPACE_APPLICATION = new InjectionToken<EspaceApplication>('ESPACE_APPLICATION');
