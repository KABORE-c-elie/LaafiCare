import { DOCUMENT, Injectable, inject, signal } from '@angular/core';

/** Texte affiché quand le navigateur est hors connexion (validé par le porteur). */
export const MESSAGE_HORS_CONNEXION = "Pas de connexion. Vos demandes s'afficheront dès le retour du réseau.";

/**
 * État du réseau d'après le navigateur : `navigator.onLine` et les
 * événements `online` / `offline` (MDN, Navigator.onLine). MDN prévient
 * qu'« en ligne » ne garantit pas l'accès à internet : une requête peut encore
 * échouer, et chaque écran garde son message d'erreur réseau.
 */
@Injectable({ providedIn: 'root' })
export class ReseauService {
  private readonly fenetre = inject(DOCUMENT).defaultView;
  private readonly _enLigne = signal(this.fenetre?.navigator.onLine ?? true);
  readonly enLigne = this._enLigne.asReadonly();

  constructor() {
    this.fenetre?.addEventListener('online', () => this._enLigne.set(true));
    this.fenetre?.addEventListener('offline', () => this._enLigne.set(false));
  }
}
