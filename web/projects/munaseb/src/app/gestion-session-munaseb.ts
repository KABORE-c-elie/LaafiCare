import { Injectable, inject } from '@angular/core';
import { Router } from '@angular/router';
import { GestionSession, MESSAGES, MessageNavigation } from 'commun';

/** Réactions de l'application MUNASEB quand la session ne peut plus servir. */
@Injectable({ providedIn: 'root' })
export class GestionSessionMunaseb implements GestionSession {
  private readonly router = inject(Router);
  private readonly messages = inject(MessageNavigation);

  sessionTerminee(): void {
    this.messages.poser({ genre: 'erreur', texte: MESSAGES.sessionTerminee });
    void this.router.navigate(['/connexion']);
  }

  accesRetire(): void {
    this.messages.poser({ genre: 'erreur', texte: MESSAGES.accesRetire });
    void this.router.navigate(['/espaces']);
  }
}
