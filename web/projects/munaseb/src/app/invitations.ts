import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { DELAI_REQUETE_MS, InvitationEnvoyee, PageInvitations, StatutInvitation } from 'commun';
import { firstValueFrom } from 'rxjs';
import { RoleMunaseb } from './roles';

const BASE = '/api/professionnels/invitations';

/** Nombre d'invitations par page (paramètre `limite`, 1 à 100, docs/api.md §2.10). */
export const INVITATIONS_PAR_PAGE = 20;

export const LIBELLES_STATUT_INVITATION: Readonly<Record<StatutInvitation, string>> = {
  en_attente: 'En attente',
  expiree: 'Expirée',
  acceptee: 'Acceptée',
  refusee: 'Refusée',
  annulee: 'Annulée',
};

/** Libellé d'un statut ; une valeur inconnue (ex. `statut_actuel` d'une erreur) est rendue telle quelle. */
export function libelleStatutInvitation(statut: string): string {
  return LIBELLES_STATUT_INVITATION[statut as StatutInvitation] ?? statut;
}

/**
 * Invitations du responsable (docs/api.md §2.10). La structure est celle de
 * l'affectation active, portée par le jeton en exercice (ajouté par
 * l'intercepteur) : jamais un identifiant de structure dans l'adresse.
 */
@Injectable({ providedIn: 'root' })
export class InvitationsService {
  private readonly http = inject(HttpClient);

  /** Le numéro part tel que saisi : le serveur le normalise (§1.6). */
  inviter(telephone: string, role: RoleMunaseb): Promise<InvitationEnvoyee> {
    return firstValueFrom(this.http.post<InvitationEnvoyee>(BASE, { telephone, role }, { timeout: DELAI_REQUETE_MS }));
  }

  /** Les plus récentes d'abord ; `avant` = `suivant` de la page précédente. */
  lister(avant: string | null): Promise<PageInvitations> {
    let params = new HttpParams().set('limite', INVITATIONS_PAR_PAGE);
    if (avant !== null) {
      params = params.set('avant', avant);
    }
    return firstValueFrom(this.http.get<PageInvitations>(BASE, { params, timeout: DELAI_REQUETE_MS }));
  }

  annuler(id: string): Promise<void> {
    return firstValueFrom(
      this.http.post<void>(`${BASE}/${encodeURIComponent(id)}/annuler`, null, { timeout: DELAI_REQUETE_MS }),
    );
  }
}
