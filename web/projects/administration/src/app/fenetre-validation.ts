import { DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { ChampComponent, IconeComponent, codeErreur, formaterJour } from 'commun';
import { FenetreComponent, IssueFenetre } from 'commun/fenetre';

export interface DonneesValidationStructure {
  nom: string;
  /** Premier jour permis pour la fin de licence (`AAAA-MM-JJ`) : demain, au Burkina Faso. */
  demain: string;
  /** `null` : licence sans fin. */
  envoyer: (licence: string | null) => Promise<void>;
}

// Textes de la fenêtre. `licenceInvalide` est celui de M1 (code `licence_invalide`).
export const MESSAGES_VALIDATION = {
  nomNonVerifie: 'Cochez la case après avoir vérifié le nom de la structure.',
  choixLicence: 'Choisissez la durée de la licence.',
  dateManquante: 'Choisissez le dernier jour de la licence.',
  licenceInvalide: "La date de fin doit être après aujourd'hui.",
} as const;

type ChoixLicence = 'sans_fin' | 'jusqu_au';

/**
 * Validation d'une demande de structure (docs/api.md §2.9, décision B8 du
 * porteur) :
 * - avertissement sur le nom, qui part tel quel dans les SMS d'invitation
 *   (§2.10), avec une case obligatoire ;
 * - licence sans fin, ou jusqu'à une date comprise, choisie sans valeur par
 *   défaut. §2.9 : la date « doit être postérieure à aujourd'hui » ; le
 *   premier jour permis est donc demain, au Burkina Faso.
 * L'envoi se fait dans la fenêtre : jamais deux envois.
 */
@Component({
  selector: 'lc-fenetre-validation-structure',
  imports: [FenetreComponent, ChampComponent, IconeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-fenetre titre="Valider la demande" idTitre="titre-validation">
      <div class="message message--info avertissement-nom">
        <lc-icone nom="info" />
        <p>
          Avant de valider, vérifiez que le nom <strong class="nom-exact">« {{ donnees.nom }} »</strong> ne ressemble
          ni à un lien ni à une consigne : il est repris tel quel dans les SMS d'invitation.
        </p>
      </div>
      <div class="champ" [class.champ--erreur]="erreurNom() !== null">
        <label class="case">
          <input
            type="checkbox"
            [checked]="nomVerifie()"
            (change)="cocherNom($event)"
            [attr.aria-describedby]="erreurNom() !== null ? 'nom-erreur' : null"
          />
          <span>J'ai vérifié le nom de la structure.</span>
        </label>
        @if (erreurNom(); as texte) {
          <p class="champ__erreur" id="nom-erreur"><lc-icone nom="erreur" /><span>{{ texte }}</span></p>
        }
      </div>

      <fieldset class="choix" [attr.aria-describedby]="erreurChoix() !== null ? 'choix-erreur' : null">
        <legend class="champ__libelle">Licence</legend>
        <label class="choix__option">
          <input type="radio" name="licence" value="sans_fin" [checked]="choix() === 'sans_fin'" (change)="choisir('sans_fin')" />
          <span>Licence sans fin</span>
        </label>
        <label class="choix__option">
          <input type="radio" name="licence" value="jusqu_au" [checked]="choix() === 'jusqu_au'" (change)="choisir('jusqu_au')" />
          <span>Jusqu'à une date</span>
        </label>
      </fieldset>
      @if (erreurChoix(); as texte) {
        <p class="champ__erreur" id="choix-erreur"><lc-icone nom="erreur" /><span>{{ texte }}</span></p>
      }

      @if (choix() === 'jusqu_au') {
        <lc-champ
          libelle="Dernier jour de la licence"
          pour="licence"
          [aide]="aideDate"
          [erreur]="erreurDate()"
        >
          <input
            id="licence"
            class="champ__saisie"
            type="date"
            [min]="donnees.demain"
            [value]="date()"
            (input)="saisirDate($event)"
            [attr.aria-invalid]="erreurDate() !== null"
            [attr.aria-describedby]="erreurDate() !== null ? 'licence-aide licence-erreur' : 'licence-aide'"
          />
        </lc-champ>
      }

      <div actions>
        <button type="button" class="bouton bouton--secondaire" [disabled]="envoiEnCours()" (click)="annuler()">
          Annuler
        </button>
        <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="confirmer()">
          {{ envoiEnCours() ? 'Validation…' : 'Valider la demande' }}
        </button>
      </div>
    </lc-fenetre>
  `,
})
export class FenetreValidationStructureComponent {
  protected readonly donnees = inject<DonneesValidationStructure>(DIALOG_DATA);
  private readonly ref = inject<DialogRef<IssueFenetre<void>>>(DialogRef);

  protected readonly aideDate = `Valable jusqu'à ce jour inclus (heure du Burkina Faso). Au plus tôt : ${formaterJour(this.donnees.demain)}.`;
  protected readonly nomVerifie = signal(false);
  protected readonly choix = signal<ChoixLicence | null>(null);
  protected readonly date = signal('');
  protected readonly erreurNom = signal<string | null>(null);
  protected readonly erreurChoix = signal<string | null>(null);
  protected readonly erreurDate = signal<string | null>(null);
  protected readonly envoiEnCours = signal(false);

  cocherNom(evenement: Event): void {
    this.nomVerifie.set((evenement.target as HTMLInputElement).checked);
    this.erreurNom.set(null);
  }

  choisir(choix: ChoixLicence): void {
    this.choix.set(choix);
    this.erreurChoix.set(null);
    this.erreurDate.set(null);
  }

  saisirDate(evenement: Event): void {
    this.date.set((evenement.target as HTMLInputElement).value);
    this.erreurDate.set(null);
  }

  annuler(): void {
    this.ref.close();
  }

  async confirmer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    const licence = this.controler();
    if (licence === undefined) {
      return;
    }
    this.envoiEnCours.set(true);
    this.ref.disableClose = true;
    try {
      await this.donnees.envoyer(licence);
      this.ref.close({ ok: true, resultat: undefined });
    } catch (erreur) {
      // Le serveur garde le dernier mot sur la date (horloge du Burkina) : son refus s'affiche ici.
      if (codeErreur(erreur) === 'licence_invalide') {
        this.erreurDate.set(MESSAGES_VALIDATION.licenceInvalide);
      } else {
        this.ref.close({ ok: false, erreur });
      }
    } finally {
      this.ref.disableClose = false;
      this.envoiEnCours.set(false);
    }
  }

  /** Licence à envoyer (`null` : sans fin), ou `undefined` si la saisie est incomplète. */
  private controler(): string | null | undefined {
    let complet = true;
    if (!this.nomVerifie()) {
      this.erreurNom.set(MESSAGES_VALIDATION.nomNonVerifie);
      complet = false;
    }
    const choix = this.choix();
    if (choix === null) {
      this.erreurChoix.set(MESSAGES_VALIDATION.choixLicence);
      return undefined;
    }
    if (choix === 'jusqu_au') {
      const date = this.date();
      if (date === '') {
        this.erreurDate.set(MESSAGES_VALIDATION.dateManquante);
        return undefined;
      }
      // Dates `AAAA-MM-JJ` : l'ordre alphabétique est l'ordre des jours.
      if (date < this.donnees.demain) {
        this.erreurDate.set(MESSAGES_VALIDATION.licenceInvalide);
        return undefined;
      }
      return complet ? date : undefined;
    }
    return complet ? null : undefined;
  }
}
