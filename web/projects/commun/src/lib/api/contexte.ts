import { HttpContext, HttpContextToken } from '@angular/common/http';

/**
 * Jeton à présenter dans `Authorization` pour une requête (lu par l'intercepteur).
 * - `acces` : le jeton d'accès de la session ; un 401 `jeton_invalide` déclenche le renouvellement ;
 * - `intermediaire` : le jeton du second facteur (§2.4), jamais renouvelé ;
 * - `reenrolement` : le jeton de réenrôlement d'un administrateur (§2.5), jamais renouvelé ;
 * - `aucun` : routes sans jeton (connexion, renouvellement, déconnexion).
 */
export type JetonRequete = 'acces' | 'intermediaire' | 'reenrolement' | 'aucun';

export const JETON_REQUETE = new HttpContextToken<JetonRequete>(() => 'acces');

/** Requête déjà rejouée après un renouvellement : jamais rejouée une seconde fois. */
export const DEJA_REJOUEE = new HttpContextToken<boolean>(() => false);

export function contexteJeton(jeton: JetonRequete): HttpContext {
  return new HttpContext().set(JETON_REQUETE, jeton);
}

/**
 * Délai d'attente de chaque requête, en millisecondes (option `timeout`
 * d'HttpClient, angular.dev/guide/http/making-requests). 10 s : un
 * renouvellement sans réponse peut être relancé avant la limite de 30 s de
 * docs/api.md §1.7.
 */
export const DELAI_REQUETE_MS = 10_000;
