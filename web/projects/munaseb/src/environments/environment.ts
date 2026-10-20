import { HttpInterceptorFn } from '@angular/common/http';

/**
 * Construction de production. En développement, ce fichier est remplacé par
 * `environment.development.ts` (`fileReplacements` dans angular.json,
 * angular.dev/tools/cli/environments).
 */
export const intercepteursDeveloppement: HttpInterceptorFn[] = [];
