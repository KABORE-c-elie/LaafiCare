import { Injectable, computed, signal } from '@angular/core';
import { Affectation, PaireJetons } from '../api/types';

/**
 * Où en est la personne. L'application ne lit jamais le contenu d'un jeton
 * (docs/api.md §1.2) : elle sait quelle sorte de jeton elle détient d'après la
 * route qui le lui a donné.
 * - `second_facteur` : jeton intermédiaire de §2.4, valable 5 min ;
 * - `sans_affectation` : jeton `laaficare:professionnel_sans_affectation` (§2.4) ;
 * - `en_exercice` : jeton `laaficare:professionnel_en_exercice` (§2.6).
 */
export type EtatSession = 'deconnecte' | 'second_facteur' | 'sans_affectation' | 'en_exercice';

/**
 * Jetons de la session, en mémoire seulement (décision « option A », voir
 * web/docs/decisions.md) : jamais localStorage, sessionStorage ni cookie.
 * OWASP Session Management Cheat Sheet : « Do not store authentication tokens,
 * session IDs, JWTs, refresh tokens, or any credential in localStorage ». Un
 * rechargement de la page efface tout et redemande la connexion : c'est voulu.
 */
@Injectable({ providedIn: 'root' })
export class SessionService {
  private readonly _etat = signal<EtatSession>('deconnecte');
  private readonly _jetonAcces = signal<string | null>(null);
  private readonly _jetonRenouvellement = signal<string | null>(null);
  private readonly _jetonIntermediaire = signal<string | null>(null);
  private readonly _affectation = signal<Affectation | null>(null);

  readonly etat = this._etat.asReadonly();
  readonly jetonAcces = this._jetonAcces.asReadonly();
  readonly jetonRenouvellement = this._jetonRenouvellement.asReadonly();
  readonly jetonIntermediaire = this._jetonIntermediaire.asReadonly();
  /** Affectation choisie, pour l'en-tête ; `null` hors exercice. */
  readonly affectation = this._affectation.asReadonly();
  readonly connecte = computed(() => this._etat() === 'sans_affectation' || this._etat() === 'en_exercice');

  ouvrirSecondFacteur(jetonIntermediaire: string): void {
    this.effacer();
    this._jetonIntermediaire.set(jetonIntermediaire);
    this._etat.set('second_facteur');
  }

  ouvrirSansAffectation(paire: PaireJetons): void {
    this.effacer();
    this.poserPaire(paire);
    this._etat.set('sans_affectation');
  }

  /**
   * L'activation ferme l'ancienne session et en ouvre une nouvelle (§2.6) :
   * les anciens jetons ne servent plus, seule la nouvelle paire est gardée.
   */
  ouvrirEnExercice(paire: PaireJetons, affectation: Affectation): void {
    this.effacer();
    this.poserPaire(paire);
    this._affectation.set(affectation);
    this._etat.set('en_exercice');
  }

  /** Après un renouvellement : les deux jetons sont remplacés, la sorte ne change pas (§1.7). */
  remplacerPaire(paire: PaireJetons): void {
    this.poserPaire(paire);
  }

  effacer(): void {
    this._jetonAcces.set(null);
    this._jetonRenouvellement.set(null);
    this._jetonIntermediaire.set(null);
    this._affectation.set(null);
    this._etat.set('deconnecte');
  }

  private poserPaire(paire: PaireJetons): void {
    this._jetonAcces.set(paire.jeton_acces);
    this._jetonRenouvellement.set(paire.jeton_renouvellement);
    this._jetonIntermediaire.set(null);
  }
}
