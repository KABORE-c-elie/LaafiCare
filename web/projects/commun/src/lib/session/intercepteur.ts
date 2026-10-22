import { HttpEvent, HttpHandlerFn, HttpInterceptorFn, HttpRequest } from '@angular/common/http';
import { inject } from '@angular/core';
import { Observable, catchError, from, switchMap, throwError } from 'rxjs';
import { codeErreur } from '../api/erreur';
import { DEJA_REJOUEE, JETON_REQUETE } from '../api/contexte';
import { AccesService } from './acces';
import { GESTION_SESSION, GestionSession } from './gestion-session';
import { SessionService } from './session';

/**
 * Intercepteur fonctionnel (angular.dev/guide/http/interceptors), enregistré
 * par `provideHttpClient(withInterceptors([intercepteurAcces]))`.
 * - ajoute `Authorization: Bearer` (RFC 6750, docs/api.md §1.2) ;
 * - sur 401 `jeton_invalide` : renouvelle (un seul renouvellement à la fois,
 *   voir `AccesService.renouveler`), puis rejoue la requête une seule fois ;
 * - renouvellement refusé : session effacée, retour à la connexion ;
 * - 403 `acces_refuse` : l'application relit les affectations (§2.6).
 */
export const intercepteurAcces: HttpInterceptorFn = (req, next) => {
  const session = inject(SessionService);
  const acces = inject(AccesService);
  const gestion = inject(GESTION_SESSION, { optional: true });

  const sorte = req.context.get(JETON_REQUETE);
  if (sorte === 'aucun') {
    return next(req);
  }
  if (sorte === 'intermediaire') {
    // Le jeton intermédiaire ne se renouvelle pas : l'écran du second facteur
    // traite lui-même son 401 `jeton_invalide` (retour à la connexion).
    return next(avecJeton(req, session.jetonIntermediaire()));
  }
  if (sorte === 'reenrolement') {
    // Même règle : l'écran du réenrôlement traite son 401 `jeton_invalide` (§2.5).
    return next(avecJeton(req, session.jetonReenrolement()));
  }

  const jetonUtilise = session.jetonAcces();
  return next(avecJeton(req, jetonUtilise)).pipe(
    catchError((erreur: unknown) => {
      const code = codeErreur(erreur);
      if (code === 'acces_refuse') {
        gestion?.accesRetire();
        return throwError(() => erreur);
      }
      if (code !== 'jeton_invalide' || jetonUtilise === null || req.context.get(DEJA_REJOUEE)) {
        return throwError(() => erreur);
      }
      // Un autre renouvellement a déjà remplacé le jeton pendant cette requête :
      // pas de nouveau renouvellement, la requête est rejouée avec le jeton actuel.
      if (session.jetonAcces() !== null && session.jetonAcces() !== jetonUtilise) {
        return rejouer(req, next, session, gestion);
      }
      return from(acces.renouveler()).pipe(
        switchMap((renouvele) => {
          if (!renouvele) {
            terminer(session, gestion);
            return throwError(() => erreur);
          }
          return rejouer(req, next, session, gestion);
        }),
      );
    }),
  );
};

/** Rejoue une seule fois : la requête rejouée est marquée et n'entre plus dans le renouvellement. */
function rejouer(
  req: HttpRequest<unknown>,
  next: HttpHandlerFn,
  session: SessionService,
  gestion: GestionSession | null,
): Observable<HttpEvent<unknown>> {
  const rejouee = req.clone({ context: req.context.set(DEJA_REJOUEE, true) });
  return next(avecJeton(rejouee, session.jetonAcces())).pipe(
    catchError((erreur: unknown) => {
      const code = codeErreur(erreur);
      if (code === 'jeton_invalide') {
        terminer(session, gestion);
      } else if (code === 'acces_refuse') {
        gestion?.accesRetire();
      }
      return throwError(() => erreur);
    }),
  );
}

function terminer(session: SessionService, gestion: GestionSession | null): void {
  session.effacer();
  gestion?.sessionTerminee();
}

function avecJeton(req: HttpRequest<unknown>, jeton: string | null): HttpRequest<unknown> {
  return jeton === null ? req : req.clone({ setHeaders: { Authorization: `Bearer ${jeton}` } });
}
