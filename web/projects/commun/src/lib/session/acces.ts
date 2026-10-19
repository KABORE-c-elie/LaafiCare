import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { firstValueFrom } from 'rxjs';
import { decoderErreur } from '../api/erreur';
import { DELAI_REQUETE_MS, contexteJeton } from '../api/contexte';
import {
  Affectation,
  CorpsSecondFacteur,
  ListeAffectations,
  PaireJetons,
  ReponseConnexionProfessionnelle,
  ReponseSecondFacteur,
} from '../api/types';
import { SessionService } from './session';

/**
 * Règle des 30 secondes (docs/api.md §1.7) : si la réponse d'un renouvellement
 * se perd, le serveur accepte une seule fois l'ancien jeton dans les 30 s qui
 * suivent son premier usage. Au-delà, il ferme la session.
 */
export const FENETRE_RELANCE_RENOUVELLEMENT_MS = 30_000;

/** Connexion professionnelle, second facteur, affectations, sessions (docs/api.md §1.7, §2.4, §2.6). */
@Injectable({ providedIn: 'root' })
export class AccesService {
  private readonly http = inject(HttpClient);
  private readonly session = inject(SessionService);

  /** Horloge monotone (MDN, `performance.now()`), remplaçable dans les tests. */
  horloge: () => number = () => performance.now();

  private renouvellementEnCours: Promise<boolean> | null = null;

  /** Ouvre la session, ou le second temps si un second facteur est actif. Les erreurs remontent à l'écran. */
  async connecter(email: string, motDePasse: string): Promise<'complete' | 'second_facteur'> {
    const reponse = await firstValueFrom(
      this.http.post<ReponseConnexionProfessionnelle>(
        '/api/professionnels/connexion',
        { email, mot_de_passe: motDePasse },
        { context: contexteJeton('aucun'), timeout: DELAI_REQUETE_MS },
      ),
    );
    // docs/api.md §2.4 : « L'application teste toujours `etape` ».
    if (reponse.etape === 'second_facteur') {
      this.session.ouvrirSecondFacteur(reponse.jeton_intermediaire);
      return 'second_facteur';
    }
    this.session.ouvrirSansAffectation(reponse);
    return 'complete';
  }

  /** Second temps (§2.4) : un seul des deux champs est envoyé. */
  async validerSecondFacteur(corps: CorpsSecondFacteur): Promise<void> {
    const reponse = await firstValueFrom(
      this.http.post<ReponseSecondFacteur>('/api/professionnels/connexion/second-facteur', corps, {
        context: contexteJeton('intermediaire'),
        timeout: DELAI_REQUETE_MS,
      }),
    );
    this.session.ouvrirSansAffectation(reponse);
  }

  async listerAffectations(): Promise<Affectation[]> {
    const reponse = await firstValueFrom(
      this.http.get<ListeAffectations>('/api/professionnels/affectations', { timeout: DELAI_REQUETE_MS }),
    );
    return reponse.affectations;
  }

  /** Activation (§2.6) : la session courante est remplacée par celle de l'affectation choisie. */
  async activer(affectation: Affectation): Promise<void> {
    const paire = await firstValueFrom(
      this.http.post<PaireJetons>(
        `/api/professionnels/affectations/${encodeURIComponent(affectation.id)}/activer`,
        null,
        { timeout: DELAI_REQUETE_MS },
      ),
    );
    this.session.ouvrirEnExercice(paire, affectation);
  }

  /**
   * Déconnexion (§1.7) : la route répond toujours 204. La mémoire est effacée
   * même si la requête échoue : se déconnecter ne doit jamais rester bloqué.
   */
  async deconnecter(): Promise<void> {
    const jeton = this.session.jetonRenouvellement();
    try {
      if (jeton !== null) {
        await firstValueFrom(
          this.http.post<void>(
            '/api/sessions/deconnexion',
            { jeton_renouvellement: jeton },
            { context: contexteJeton('aucun'), timeout: DELAI_REQUETE_MS },
          ),
        );
      }
    } catch {
      // Sans effet : la session côté serveur expirera d'elle-même (§1.7, inactivité).
    } finally {
      this.session.effacer();
    }
  }

  /**
   * Renouvelle la paire (§1.7). Un seul renouvellement à la fois : les appels
   * suivants reçoivent la même promesse (« ne jamais lancer deux
   * renouvellements en même temps avec le même jeton »).
   * Renvoie `false` si la session ne peut plus être renouvelée.
   */
  renouveler(): Promise<boolean> {
    if (this.renouvellementEnCours === null) {
      this.renouvellementEnCours = this.executerRenouvellement().finally(() => {
        this.renouvellementEnCours = null;
      });
    }
    return this.renouvellementEnCours;
  }

  private async executerRenouvellement(): Promise<boolean> {
    const jeton = this.session.jetonRenouvellement();
    if (jeton === null) {
      return false;
    }
    const debut = this.horloge();
    for (let essai = 1; essai <= 2; essai++) {
      try {
        const paire = await firstValueFrom(
          this.http.post<PaireJetons>(
            '/api/sessions/renouveler',
            { jeton_renouvellement: jeton },
            { context: contexteJeton('aucun'), timeout: DELAI_REQUETE_MS },
          ),
        );
        // Déconnexion pendant le renouvellement : la nouvelle paire n'est pas gardée.
        if (this.session.jetonRenouvellement() !== jeton) {
          return false;
        }
        // §1.7 : toujours remplacer les deux jetons ; l'ancien n'est plus jamais renvoyé.
        this.session.remplacerPaire(paire);
        return true;
      } catch (erreur) {
        // Relance du même jeton seulement quand aucune réponse n'est arrivée
        // (réseau, délai), une seule fois, et sous les 30 s de §1.7. Une
        // réponse 401 du serveur, elle, est définitive.
        const relancer =
          essai === 1 &&
          decoderErreur(erreur).genre === 'reseau' &&
          this.horloge() - debut < FENETRE_RELANCE_RENOUVELLEMENT_MS;
        if (!relancer) {
          return false;
        }
      }
    }
    return false;
  }
}
