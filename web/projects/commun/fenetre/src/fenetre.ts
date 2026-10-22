import { DIALOG_DATA, Dialog, DialogConfig, DialogRef } from '@angular/cdk/dialog';
import { ComponentType } from '@angular/cdk/portal';
import { ChangeDetectionStrategy, Component, Injectable, computed, inject, input, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ChampComponent, codeErreur } from 'commun';

/**
 * Fenêtres modales par le `Dialog` du CDK (material.angular.dev/cdk/dialog,
 * source : angular/components, src/cdk/dialog/dialog.md) :
 * - rôle ARIA `dialog`, focus piégé dans la fenêtre, premier élément
 *   tabulable focalisé, focus rendu à l'élément d'origine à la fermeture ;
 * - la touche Échap ferme la fenêtre (`disableClose` laissé à `false` :
 *   le désactiver « breaks the expected interaction pattern ») ;
 * - les styles de structure viennent de `@angular/cdk/overlay-prebuilt.css`,
 *   importé par la feuille globale de l'application.
 */
@Injectable({ providedIn: 'root' })
export class FenetreService {
  private readonly dialog = inject(Dialog);

  ouvrir<R, D, C>(composant: ComponentType<C>, donnees: D, idTitre: string): DialogRef<R, C> {
    const config: DialogConfig<D, DialogRef<R, C>> = {
      data: donnees,
      // La doc demande un nom accessible : le titre de la fenêtre.
      ariaLabelledBy: idTitre,
      panelClass: 'fenetre',
      backdropClass: 'fenetre-fond',
      autoFocus: 'first-tabbable',
      restoreFocus: true,
    };
    return this.dialog.open<R, D, C>(composant, config);
  }
}

/** Fin d'une fenêtre d'action : succès avec la réponse, ou erreur laissée à la page. `undefined` : annulée. */
export type IssueFenetre<R> = { ok: true; resultat: R } | { ok: false; erreur: unknown };

/** Mise en page commune d'une fenêtre : titre, contenu, boutons. */
@Component({
  selector: 'lc-fenetre',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="fenetre__corps">
      <h2 class="fenetre__titre" [id]="idTitre()">{{ titre() }}</h2>
      <div class="fenetre__contenu"><ng-content /></div>
      <div class="fenetre__actions"><ng-content select="[actions]" /></div>
    </div>
  `,
})
export class FenetreComponent {
  readonly titre = input.required<string>();
  readonly idTitre = input.required<string>();
}

export interface DonneesConfirmation<R> {
  titre: string;
  texte: string;
  /** Libellé du bouton qui confirme, ex. « Inviter comme responsable ». */
  action: string;
  envoyer: () => Promise<R>;
}

/**
 * Confirmation d'une action avant l'appel. L'envoi se fait dans la fenêtre :
 * jamais deux envois. Pendant l'envoi, ni Échap ni un clic à côté ne la ferment.
 */
@Component({
  selector: 'lc-fenetre-confirmation',
  imports: [FenetreComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-fenetre [titre]="donnees.titre" idTitre="titre-confirmation">
      <p>{{ donnees.texte }}</p>
      <div actions>
        <button type="button" class="bouton bouton--secondaire" [disabled]="envoiEnCours()" (click)="annuler()">
          Retour
        </button>
        <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="confirmer()">
          {{ envoiEnCours() ? 'Envoi…' : donnees.action }}
        </button>
      </div>
    </lc-fenetre>
  `,
})
export class FenetreConfirmationComponent {
  protected readonly donnees = inject<DonneesConfirmation<unknown>>(DIALOG_DATA);
  private readonly ref = inject<DialogRef<IssueFenetre<unknown>>>(DialogRef);
  protected readonly envoiEnCours = signal(false);

  annuler(): void {
    this.ref.close();
  }

  async confirmer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
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

/** Ce que chaque application fixe pour une fenêtre de motif (rejet d'une demande, refus d'une structure). */
export interface DonneesMotif {
  titre: string;
  idTitre: string;
  libelle: string;
  /** Libellé du bouton qui envoie. */
  action: string;
  /** Phrase sous le compteur, ex. « Le motif n'apparaît pas dans la notification de l'adhérent. » */
  rappel: string;
  /** Longueur maximale, en caractères (points de code), et sa forme affichée (« 1 000 »). */
  max: number;
  maxAffiche: string;
  messageVide: string;
  messageTropLong: string;
  envoyer: (motif: string) => Promise<void>;
}

/**
 * Motif obligatoire, borné en longueur, avec compteur. Les deux refus du serveur
 * (`motif_vide`, `motif_trop_long`, docs/api.md §2.3.6 et §2.9) s'affichent dans
 * la fenêtre, qui reste ouverte ; toute autre erreur est rendue à la page.
 */
@Component({
  selector: 'lc-fenetre-motif',
  imports: [FenetreComponent, ChampComponent, FormsModule],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-fenetre [titre]="donnees.titre" [idTitre]="donnees.idTitre">
      <lc-champ [libelle]="donnees.libelle" pour="motif" [erreur]="erreur()">
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
        <span [class.compteur--depasse]="longueur() > donnees.max">{{ longueur() }} / {{ donnees.maxAffiche }} caractères</span>.
        {{ donnees.rappel }}
      </p>
      <div actions>
        <button type="button" class="bouton bouton--secondaire" [disabled]="envoiEnCours()" (click)="annuler()">
          Annuler
        </button>
        <button type="button" class="bouton bouton--sombre" [disabled]="envoiEnCours()" (click)="confirmer()">
          {{ envoiEnCours() ? 'Envoi…' : donnees.action }}
        </button>
      </div>
    </lc-fenetre>
  `,
})
export class FenetreMotifComponent {
  protected readonly donnees = inject<DonneesMotif>(DIALOG_DATA);
  private readonly ref = inject<DialogRef<IssueFenetre<void>>>(DialogRef);

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
      this.erreur.set(this.donnees.messageVide);
      return;
    }
    if (this.longueur() > this.donnees.max) {
      this.erreur.set(this.donnees.messageTropLong);
      return;
    }
    this.envoiEnCours.set(true);
    this.ref.disableClose = true;
    try {
      await this.donnees.envoyer(this.motif());
      this.ref.close({ ok: true, resultat: undefined });
    } catch (erreur) {
      // Le serveur garde le dernier mot : ses deux refus du motif s'affichent ici.
      const code = codeErreur(erreur);
      if (code === 'motif_vide') {
        this.erreur.set(this.donnees.messageVide);
      } else if (code === 'motif_trop_long') {
        this.erreur.set(this.donnees.messageTropLong);
      } else {
        this.ref.close({ ok: false, erreur });
      }
    } finally {
      this.ref.disableClose = false;
      this.envoiEnCours.set(false);
    }
  }
}
