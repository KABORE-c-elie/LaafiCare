import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import {
  DELAI_REQUETE_MS,
  ReponseReenrolementCommencer,
  ReponseReenrolementConfirmer,
  contexteJeton,
} from 'commun';
import { firstValueFrom } from 'rxjs';

const BASE = '/api/administrateurs/reenrolement';

/**
 * Réenrôlement d'un administrateur sans second facteur (docs/api.md §2.5), avec
 * le jeton de réenrôlement (10 min), présenté par l'intercepteur et jamais
 * renouvelé. Les réponses (adresse otpauth, clé, codes de secours) ne sont
 * gardées que par la page, en mémoire.
 */
@Injectable({ providedIn: 'root' })
export class ReenrolementService {
  private readonly http = inject(HttpClient);

  /** Le mot de passe est redemandé, comme pour toute activation d'un second facteur. */
  commencer(motDePasse: string): Promise<ReponseReenrolementCommencer> {
    return firstValueFrom(
      this.http.post<ReponseReenrolementCommencer>(
        `${BASE}/commencer`,
        { mot_de_passe: motDePasse },
        { context: contexteJeton('reenrolement'), timeout: DELAI_REQUETE_MS },
      ),
    );
  }

  /** Premier code de l'application d'authentification ; la réponse donne les 10 codes de secours. */
  confirmer(code: string): Promise<ReponseReenrolementConfirmer> {
    return firstValueFrom(
      this.http.post<ReponseReenrolementConfirmer>(
        `${BASE}/confirmer`,
        { code },
        { context: contexteJeton('reenrolement'), timeout: DELAI_REQUETE_MS },
      ),
    );
  }
}
