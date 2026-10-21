import { DestroyRef, Injectable, computed, inject, signal } from '@angular/core';
import {
  AccesService,
  MESSAGES,
  MessageAffiche,
  decoderErreur,
  messagePourErreur,
  secondesRetryAfter,
} from 'commun';

/**
 * Attente quand le 429 arrive sans `Retry-After` lisible : docs/api.md §2.7,
 * « un lien a été envoyé il y a moins d'une minute ».
 */
const ATTENTE_PAR_DEFAUT_S = 60;

/** Durée affichée : en secondes jusqu'à une minute, en minutes au-delà (docs/api.md §2.2.5). */
export function formaterAttente(secondes: number): string {
  return secondes <= 60 ? `${secondes} s` : `${Math.ceil(secondes / 60)} min`;
}

/**
 * Renvoi du lien de vérification (docs/api.md §2.7), partagé par l'écran de
 * connexion et la page `/verifier-email` : fourni par chaque page
 * (`providers`), il disparaît avec elle. Ne garde ni l'email ni le mot de
 * passe : la page les passe à `renvoyer`, le temps de l'envoi seulement.
 */
@Injectable()
export class RenvoiLien {
  private readonly acces = inject(AccesService);

  readonly envoiEnCours = signal(false);
  /** Secondes restantes avant de pouvoir redemander un lien, après un 429. */
  readonly attente = signal(0);
  readonly message = signal<MessageAffiche | null>(null);
  /** Bouton désactivé pendant l'envoi et pendant l'attente imposée. */
  readonly bloque = computed(() => this.envoiEnCours() || this.attente() > 0);
  readonly libelle = computed(() => {
    if (this.envoiEnCours()) {
      return 'Envoi…';
    }
    return this.attente() > 0 ? `Renvoyer le lien (${formaterAttente(this.attente())})` : 'Renvoyer le lien';
  });

  private minuterie: ReturnType<typeof setInterval> | null = null;

  constructor() {
    inject(DestroyRef).onDestroy(() => this.arreterMinuterie());
  }

  async renvoyer(email: string, motDePasse: string): Promise<void> {
    if (this.bloque()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      await this.acces.renvoyerLien(email, motDePasse);
      this.message.set({ genre: 'info', texte: MESSAGES.lienRenvoye });
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      if (decodee.genre === 'api' && decodee.code === 'trop_de_demandes') {
        // L'application lit `Retry-After`, jamais le message (docs/api.md §2.2.5).
        this.attendre(secondesRetryAfter(erreur) ?? ATTENTE_PAR_DEFAUT_S);
      } else {
        this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
      }
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  /**
   * Décompte sur le bouton, seconde par seconde. Le message, lui, est écrit une
   * seule fois : réécrit chaque seconde, il serait relu sans cesse par les
   * lecteurs d'écran (rôle `status`, zone annoncée à chaque changement).
   */
  private attendre(secondes: number): void {
    this.arreterMinuterie();
    const fin = Date.now() + secondes * 1000;
    this.attente.set(secondes);
    this.message.set({
      genre: 'info',
      texte: `Vous pourrez demander un nouveau lien dans ${formaterAttente(secondes)}.`,
    });
    this.minuterie = setInterval(() => {
      const restant = Math.max(0, Math.ceil((fin - Date.now()) / 1000));
      this.attente.set(restant);
      if (restant === 0) {
        this.arreterMinuterie();
        this.message.set(null);
      }
    }, 1000);
  }

  private arreterMinuterie(): void {
    if (this.minuterie !== null) {
      clearInterval(this.minuterie);
      this.minuterie = null;
    }
  }
}
