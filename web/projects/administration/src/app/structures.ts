import { HttpClient, HttpParams } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { DELAI_REQUETE_MS, DetailStructure, NomIcone, PageStructures, StatutStructure } from 'commun';
import { firstValueFrom } from 'rxjs';

const BASE = '/api/administrateurs/structures';

/** Nombre de structures par page (paramètre `limite`, 1 à 100, docs/api.md §2.9). */
export const STRUCTURES_PAR_PAGE = 20;

/** Longueur maximale du motif de refus (docs/api.md §2.9 : « 1 000 caractères au plus »). */
export const MOTIF_REFUS_MAX = 1000;

export const LIBELLES_STATUT_STRUCTURE: Readonly<Record<StatutStructure, string>> = {
  brouillon: 'Brouillon',
  en_attente: 'En attente',
  validee: 'Validée',
  refusee: 'Refusée',
  suspendue: 'Suspendue',
  abandonnee: 'Abandonnée',
};

/** Libellé d'un statut ; une valeur inconnue (ex. `statut_actuel` d'une erreur) est rendue telle quelle. */
export function libelleStatutStructure(statut: string): string {
  return LIBELLES_STATUT_STRUCTURE[statut as StatutStructure] ?? statut;
}

// Statuts neutres ; « Validée » en vert ; « En attente » (la file de travail) en
// bleu d'information. Jamais de rouge ni d'orange (CLAUDE.md §6) ; toujours une
// icône et un texte (WCAG 2.2, 1.4.1).
export const PASTILLES_STRUCTURE: Readonly<Record<StatutStructure, { classe: string; icone: NomIcone }>> = {
  brouillon: { classe: 'pastille--en_attente', icone: 'info' },
  en_attente: { classe: 'pastille--en_cours', icone: 'attente' },
  validee: { classe: 'pastille--valide', icone: 'valide' },
  refusee: { classe: 'pastille--rejete', icone: 'rejete' },
  suspendue: { classe: 'pastille--rejete', icone: 'attente' },
  abandonnee: { classe: 'pastille--rejete', icone: 'rejete' },
};

/** Types de structure (migration 0012 ; seule la MUNASEB est ouverte pour l'instant, §2.8). */
const LIBELLES_TYPE: Readonly<Record<string, string>> = {
  munaseb: 'MUNASEB',
  clinique: 'Clinique',
  hopital: 'Hôpital',
  pharmacie: 'Pharmacie',
  laboratoire: 'Laboratoire',
};

export function libelleType(type: string): string {
  return LIBELLES_TYPE[type] ?? type;
}

const LIBELLES_DECISION: Readonly<Record<string, string>> = {
  validee: 'Validée',
  refusee: 'Refusée',
  suspendue: 'Suspendue',
  reactivee: 'Réactivée',
};

export function libelleDecision(decision: string): string {
  return LIBELLES_DECISION[decision] ?? decision;
}

/** Nom et prénom d'une personne, ou un tiret s'ils manquent. */
export function nomComplet(nom: string | null, prenom: string | null): string {
  const texte = [nom, prenom].filter((m) => m !== null && m.trim() !== '').join(' ');
  return texte === '' ? '—' : texte;
}

/** Demandes de structure, pour l'équipe LaafiCare (docs/api.md §2.9). Jeton administrateur ajouté par l'intercepteur. */
@Injectable({ providedIn: 'root' })
export class StructuresService {
  private readonly http = inject(HttpClient);

  /** Les plus anciennes d'abord ; `apres` = `suivant` de la page précédente. */
  lister(statut: StatutStructure | null, apres: string | null): Promise<PageStructures> {
    let params = new HttpParams().set('limite', STRUCTURES_PAR_PAGE);
    if (statut !== null) {
      params = params.set('statut', statut);
    }
    if (apres !== null) {
      params = params.set('apres', apres);
    }
    return firstValueFrom(this.http.get<PageStructures>(BASE, { params, timeout: DELAI_REQUETE_MS }));
  }

  detail(id: string): Promise<DetailStructure> {
    return firstValueFrom(this.http.get<DetailStructure>(this.chemin(id), { timeout: DELAI_REQUETE_MS }));
  }

  /** `licence` : dernier jour `AAAA-MM-JJ`, ou `null` pour une licence sans fin (corps `{}`, §2.9). */
  valider(id: string, licence: string | null): Promise<void> {
    const corps = licence === null ? {} : { licence_expire_le: licence };
    return firstValueFrom(this.http.post<void>(`${this.chemin(id)}/valider`, corps, { timeout: DELAI_REQUETE_MS }));
  }

  refuser(id: string, motif: string): Promise<void> {
    return firstValueFrom(
      this.http.post<void>(`${this.chemin(id)}/refuser`, { motif }, { timeout: DELAI_REQUETE_MS }),
    );
  }

  private chemin(id: string): string {
    return `${BASE}/${encodeURIComponent(id)}`;
  }
}
