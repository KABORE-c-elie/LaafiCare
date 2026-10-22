import { DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FcfaPipe, ReponseValidation } from 'commun';
import { DonneesMotif, FenetreComponent, IssueFenetre } from 'commun/fenetre';
import { MOTIF_MAX } from './remboursements';

export interface DonneesValidation {
  montantDemande: number;
  solde: number;
  envoyer: () => Promise<ReponseValidation>;
}

export const MESSAGE_MOTIF_VIDE = 'Écrivez le motif du rejet.';
export const MESSAGE_MOTIF_TROP_LONG = 'Le motif ne doit pas dépasser 1 000 caractères.';

/**
 * Fenêtre du rejet d'une demande (docs/api.md §2.3.6) : la fenêtre de motif
 * commune (`commun/fenetre`), avec les textes de l'agent MUNASEB.
 */
export function donneesRejet(envoyer: (motif: string) => Promise<void>): DonneesMotif {
  return {
    titre: 'Rejeter la demande',
    idTitre: 'titre-rejet',
    libelle: 'Motif du rejet',
    action: 'Rejeter la demande',
    rappel: "Le motif n'apparaît pas dans la notification de l'adhérent.",
    max: MOTIF_MAX,
    maxAffiche: '1 000',
    messageVide: MESSAGE_MOTIF_VIDE,
    messageTropLong: MESSAGE_MOTIF_TROP_LONG,
    envoyer,
  };
}

/** Confirmation de la validation (docs/api.md §2.3.6). L'envoi se fait ici : jamais deux envois. */
@Component({
  selector: 'lc-fenetre-validation',
  imports: [FenetreComponent, FcfaPipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-fenetre titre="Valider la demande" idTitre="titre-validation">
      <p>
        Le montant remboursé sera le montant demandé ({{ donnees.montantDemande | fcfa }}), dans la limite du solde
        de la période ({{ donnees.solde | fcfa }}, indicatif : il est recalculé maintenant).
      </p>
      <p>L'adhérent recevra une notification.</p>
      <div actions>
        <button type="button" class="bouton bouton--secondaire" [disabled]="envoiEnCours()" (click)="annuler()">
          Annuler
        </button>
        <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="confirmer()">
          {{ envoiEnCours() ? 'Validation…' : 'Valider' }}
        </button>
      </div>
    </lc-fenetre>
  `,
})
export class FenetreValidationComponent {
  protected readonly donnees = inject<DonneesValidation>(DIALOG_DATA);
  private readonly ref = inject<DialogRef<IssueFenetre<ReponseValidation>>>(DialogRef);
  protected readonly envoiEnCours = signal(false);

  annuler(): void {
    this.ref.close();
  }

  async confirmer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    // Pendant l'envoi, ni Échap ni un clic à côté ne ferment la fenêtre.
    this.ref.disableClose = true;
    try {
      this.ref.close({ ok: true, resultat: await this.donnees.envoyer() });
    } catch (erreur) {
      this.ref.close({ ok: false, erreur });
    } finally {
      this.ref.disableClose = false;
      this.envoiEnCours.set(false);
    }
  }
}
