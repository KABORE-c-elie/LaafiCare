import { HttpInterceptorFn } from '@angular/common/http';

/** Construction de production : aucun intercepteur de développement. */
export const intercepteursDeveloppement: HttpInterceptorFn[] = [];
