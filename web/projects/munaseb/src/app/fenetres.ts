import { DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ChampComponent, FcfaPipe, ReponseValidation, codeErreur } from 'commun';
import { FenetreComponent } from 'commun/fenetre';
import { MOTIF_MAX } from './remboursements';

/** Fin d'une fenêtre d'action : succès avec la réponse, ou erreur laissée à la page. `undefined` : annulée. */
export type IssueFenetre<R> = { ok: true; resultat: R } | { ok: false; erreur: unknown };

export interface DonneesValidation {
  montantDemande: number;
  solde: number;
  envoyer: () => Promise<ReponseValidation>;
}

export interface DonneesRejet {
  envoyer: (motif: string) => Promise<void>;
}

export const MESSAGE_MOTIF_VIDE = 'Écrivez le motif du rejet.';
export const MESSAGE_MOTIF_TROP_LONG = 'Le motif ne doit pas dépasser 1 000 caractères.';

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

/** Rejet avec motif obligatoire, 1 000 caractères au plus (docs/api.md §2.3.6). */
@Component({
  selector: 'lc-fenetre-rejet',
  imports: [FenetreComponent, ChampComponent, FormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-fenetre titre="Rejeter la demande" idTitre="titre-rejet">
      <lc-champ libelle="Motif du rejet" pour="motif" [erreur]="erreur()">
        <textarea
          id="motif"
          class="champ__saisie"
          rows="5"
          [ngModel]="motif()"
          (ngModelChange)="saisir($event)"
          [attr.aria-invalid]="erreur() !== null"
          [attr.aria-describedby]="erreur() !== null ? 'motif-erreur motif-aide' : 'motif-aide'"
        ></textarea>
      </lc-champ>
      <p class="champ__aide" id="motif-aide">
        <span [class.compteur--depasse]="longueur() > max">{{ longueur() }} / 1 000 caractères</span>.
        Le motif n'apparaît pas dans la notification de l'adhérent.
      </p>
      <div actions>
        <button type="button" class="bouton bouton--secondaire" [disabled]="envoiEnCours()" (click)="annuler()">
          Annuler
        </button>
        <button type="button" class="bouton bouton--sombre" [disabled]="envoiEnCours()" (click)="confirmer()">
          {{ envoiEnCours() ? 'Envoi…' : 'Rejeter la demande' }}
        </button>
      </div>
    </lc-fenetre>
  `,
})
export class FenetreRejetComponent {
  private readonly donnees = inject<DonneesRejet>(DIALOG_DATA);
  private readonly ref = inject<DialogRef<IssueFenetre<void>>>(DialogRef);

  protected readonly max = MOTIF_MAX;
  protected readonly motif = signal('');
  protected readonly erreur = signal<string | null>(null);
  protected readonly envoiEnCours = signal(false);
  // Compté en caractères (points de code), pas en unités UTF-16 : un emoji compte pour un.
  protected readonly longueur = computed(() => [...this.motif()].length);

  saisir(valeur: string): void {
    this.motif.set(valeur);
    this.erreur.set(null);
  }

  annuler(): void {
    this.ref.close();
  }

  async confirmer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    if (this.motif().trim() === '') {
      this.erreur.set(MESSAGE_MOTIF_VIDE);
      return;
    }
    if (this.longueur() > MOTIF_MAX) {
      this.erreur.set(MESSAGE_MOTIF_TROP_LONG);
      return;
    }
    this.envoiEnCours.set(true);
    this.ref.disableClose = true;
    try {
      await this.donnees.envoyer(this.motif());
      this.ref.close({ ok: true, resultat: undefined });
    } catch (erreur) {
      // Le serveur garde le dernier mot (§2.3.6) : ses deux refus du motif s'affichent ici.
      const code = codeErreur(erreur);
      if (code === 'motif_vide') {
        this.erreur.set(MESSAGE_MOTIF_VIDE);
      } else if (code === 'motif_trop_long') {
        this.erreur.set(MESSAGE_MOTIF_TROP_LONG);
      } else {
        this.ref.close({ ok: false, erreur });
      }
    } finally {
      this.ref.disableClose = false;
      this.envoiEnCours.set(false);
    }
  }
}
