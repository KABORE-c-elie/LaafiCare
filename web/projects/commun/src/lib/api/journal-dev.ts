import { HttpInterceptorFn } from '@angular/common/http';
import { tap } from 'rxjs';
import { decoderErreur } from './erreur';

/**
 * Journal des erreurs de l'API, pour le développement seulement : il remplace
 * Postman pour voir pourquoi une requête échoue. L'application ne l'enregistre
 * que par `environment.development.ts` (remplacement de fichier à la
 * construction, angular.dev/tools/cli/environments) : il n'existe pas dans la
 * construction de production.
 *
 * Une ligne par erreur, avec seulement le code HTTP, le champ `code`, la
 * méthode et le chemin. Jamais le corps envoyé ou reçu, ni les en-têtes, ni
 * les paramètres après `?`, ni le message du serveur : aucun jeton, mot de
 * passe, code TOTP ou donnée d'adhérent ne peut s'y trouver.
 */
export const intercepteurJournalDev: HttpInterceptorFn = (req, next) =>
  next(req).pipe(
    tap({
      error: (erreur: unknown) => {
        console.warn(ligneJournal(req.method, req.url, erreur));
      },
    }),
  );

/** Texte de la ligne de journal ; exporté pour les tests. */
export function ligneJournal(methode: string, url: string, erreur: unknown): string {
  const chemin = url.split(/[?#]/u)[0];
  const decodee = decoderErreur(erreur);
  switch (decodee.genre) {
    case 'api':
      return `[API] ${decodee.statut} ${decodee.code} ${methode} ${chemin}`;
    case 'reseau':
      return `[API] aucune réponse (réseau ou délai dépassé) ${methode} ${chemin}`;
    case 'inconnue':
      return `[API] ${decodee.statut} (réponse sans champ code) ${methode} ${chemin}`;
  }
}
