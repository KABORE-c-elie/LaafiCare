import { Injectable, inject } from '@angular/core';
import { Router } from '@angular/router';
import { GestionSession, MESSAGES, MessageNavigation } from 'commun';

/** Réactions de l'application d'administration quand la session ne peut plus servir. */
@Injectable({ providedIn: 'root' })
export class GestionSessionAdministration implements GestionSession {
  private readonly router = inject(Router);
  private readonly messages = inject(MessageNavigation);

  sessionTerminee(): void {
    this.messages.poser({ genre: 'erreur', texte: MESSAGES.sessionTerminee });
    void this.router.navigate(['/connexion']);
  }

  /**
   * Les routes de l'équipe (§2.9) ne répondent pas `acces_refuse` : elles ont
   * leurs propres codes. Par prudence, un tel refus ramène à la connexion.
   */
  accesRetire(): void {
    this.sessionTerminee();
  }
}
