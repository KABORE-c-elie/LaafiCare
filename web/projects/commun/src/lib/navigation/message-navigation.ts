import { Injectable, signal } from '@angular/core';

export interface MessageAffiche {
  genre: 'info' | 'erreur';
  texte: string;
}

/**
 * Message à montrer sur l'écran suivant (ex. « Votre session est terminée. »
 * après un retour forcé à la connexion). Lu une seule fois.
 */
@Injectable({ providedIn: 'root' })
export class MessageNavigation {
  private readonly message = signal<MessageAffiche | null>(null);

  poser(message: MessageAffiche): void {
    this.message.set(message);
  }

  consommer(): MessageAffiche | null {
    const message = this.message();
    this.message.set(null);
    return message;
  }
}
