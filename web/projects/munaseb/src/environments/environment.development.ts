import { HttpInterceptorFn } from '@angular/common/http';
import { intercepteurJournalDev } from 'commun';

/** Développement seulement (`ng serve`) : journal des erreurs de l'API dans la console. */
export const intercepteursDeveloppement: HttpInterceptorFn[] = [intercepteurJournalDev];
