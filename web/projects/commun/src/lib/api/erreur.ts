import { HttpErrorResponse } from '@angular/common/http';
import { CorpsErreur } from './types';

/**
 * Erreur d'un appel à l'API, réduite à ce qui décide de la suite.
 * - `api` : le serveur a répondu avec le corps `{erreur, code}` de docs/api.md §1.3 ;
 * - `reseau` : aucune réponse (réseau coupé, délai dépassé) ;
 * - `inconnue` : une réponse sans ce corps (ex. proxy en panne), traitée comme `erreur_interne`.
 */
export type ErreurDecodee =
  | { genre: 'api'; statut: number; code: string; corps: CorpsErreur }
  | { genre: 'reseau' }
  | { genre: 'inconnue'; statut: number };

export function decoderErreur(erreur: unknown): ErreurDecodee {
  if (!(erreur instanceof HttpErrorResponse)) {
    return { genre: 'inconnue', statut: 0 };
  }
  // Angular : « Network and timeout errors have a `status` code of `0` »
  // (angular.dev/guide/http/making-requests, Request timeouts).
  if (erreur.status === 0) {
    return { genre: 'reseau' };
  }
  const corps: unknown = erreur.error;
  if (estCorpsErreur(corps)) {
    return { genre: 'api', statut: erreur.status, code: corps.code, corps };
  }
  return { genre: 'inconnue', statut: erreur.status };
}

/** Code machine de l'erreur, ou `null` si ce n'est pas une erreur de l'API. */
export function codeErreur(erreur: unknown): string | null {
  const decodee = decoderErreur(erreur);
  return decodee.genre === 'api' ? decodee.code : null;
}

/**
 * Secondes à attendre d'après l'en-tête `Retry-After` d'une réponse 429
 * (RFC 6585 §4). docs/api.md (§2.2.5, §2.7) le donne toujours en secondes :
 * la forme « date HTTP » de la RFC 9110 §10.2.3 n'est pas lue. `null` si
 * l'en-tête manque ou n'est pas un nombre entier.
 */
export function secondesRetryAfter(erreur: unknown): number | null {
  if (!(erreur instanceof HttpErrorResponse)) {
    return null;
  }
  const valeur = erreur.headers.get('Retry-After')?.trim() ?? '';
  return /^\d+$/u.test(valeur) ? Number(valeur) : null;
}

function estCorpsErreur(valeur: unknown): valeur is CorpsErreur {
  if (typeof valeur !== 'object' || valeur === null) {
    return false;
  }
  const objet = valeur as Record<string, unknown>;
  return typeof objet['code'] === 'string' && typeof objet['erreur'] === 'string';
}
