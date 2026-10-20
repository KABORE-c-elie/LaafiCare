import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { DELAI_REQUETE_MS, DemandeDetail, PageDemandes, ReponseValidation, StatutDemande, TypeActe } from 'commun';
import { firstValueFrom } from 'rxjs';

const BASE = '/api/assurance-munaseb/remboursements';

/** Nombre de demandes par page (paramètre `limite`, 1 à 100, docs/api.md §2.3.4). */
export const DEMANDES_PAR_PAGE = 20;

/** Longueur maximale du motif de rejet (docs/api.md §2.3.6). */
export const MOTIF_MAX = 1000;

const LIBELLES_ACTE: Readonly<Record<TypeActe, string>> = {
  consultation: 'Consultation',
  hospitalisation: 'Hospitalisation',
  pharmacie: 'Pharmacie',
  laboratoire: 'Laboratoire',
  lunetterie: 'Lunetterie',
};

export function libelleActe(type: TypeActe): string {
  return LIBELLES_ACTE[type];
}

/** Nom affiché de l'adhérent. Jamais son NIP ni son téléphone : l'API ne les donne pas à l'agent (§2.3.4). */
export function nomAdherent(demande: { patient_nom: string | null; patient_prenom: string | null }): string {
  const nom = [demande.patient_nom, demande.patient_prenom].filter((m) => m !== null && m.trim() !== '').join(' ');
  return nom === '' ? 'Nom non renseigné' : nom;
}

/** Routes de remboursement de l'agent (docs/api.md §2.3). Jeton en exercice ajouté par l'intercepteur. */
@Injectable({ providedIn: 'root' })
export class RemboursementsService {
  private readonly http = inject(HttpClient);

  lister(statut: StatutDemande | null, apres: string | null): Promise<PageDemandes> {
    let params = new HttpParams().set('limite', DEMANDES_PAR_PAGE);
    if (statut !== null) {
      params = params.set('statut', statut);
    }
    if (apres !== null) {
      params = params.set('apres', apres);
    }
    return firstValueFrom(this.http.get<PageDemandes>(BASE, { params, timeout: DELAI_REQUETE_MS }));
  }

  detail(id: string): Promise<DemandeDetail> {
    return firstValueFrom(this.http.get<DemandeDetail>(this.chemin(id), { timeout: DELAI_REQUETE_MS }));
  }

  prendreEnCharge(id: string): Promise<void> {
    return this.transition(id, 'prendre-en-charge', null);
  }

  valider(id: string): Promise<ReponseValidation> {
    return firstValueFrom(
      this.http.post<ReponseValidation>(`${this.chemin(id)}/valider`, null, { timeout: DELAI_REQUETE_MS }),
    );
  }

  rejeter(id: string, motif: string): Promise<void> {
    return this.transition(id, 'rejeter', { motif });
  }

  payer(id: string): Promise<void> {
    return this.transition(id, 'payer', null);
  }

  private transition(id: string, action: string, corps: object | null): Promise<void> {
    return firstValueFrom(this.http.post<void>(`${this.chemin(id)}/${action}`, corps, { timeout: DELAI_REQUETE_MS }));
  }

  private chemin(id: string): string {
    return `${BASE}/${encodeURIComponent(id)}`;
  }
}
