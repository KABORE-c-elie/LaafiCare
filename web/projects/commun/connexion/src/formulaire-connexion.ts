import { ChangeDetectionStrategy, Component, inject, input, output, signal } from '@angular/core';
import { FormControl, FormGroup, ReactiveFormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import {
  AccesService,
  ChampComponent,
  IconeComponent,
  MessageAffiche,
  MessageEtatComponent,
  MessageNavigation,
  PARCOURS_CONNEXION,
  codeErreur,
  decoderErreur,
  messagePourErreur,
} from 'commun';

/**
 * Formulaire de connexion par email et mot de passe (docs/api.md §2.4, §2.5),
 * route de l'API et pages suivantes lues dans `PARCOURS_CONNEXION`. Le contenu
 * projeté s'affiche sous le message (ex. « Renvoyer le lien » de l'application
 * MUNASEB). L'élément hôte est transparent pour la mise en page : le
 * formulaire reste l'enfant direct de la zone centrée.
 */
@Component({
  selector: 'lc-formulaire-connexion',
  imports: [ReactiveFormsModule, ChampComponent, IconeComponent, MessageEtatComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'transparent' },
  template: `
    <form class="carte formulaire" [formGroup]="formulaire" (ngSubmit)="envoyer()" novalidate>
      <h2 class="formulaire__titre">Connexion</h2>

      @if (message(); as m) {
        <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
      }
      <ng-content />

      <lc-champ libelle="Email" pour="email" aide="Sans accent ni espace" [erreur]="erreurEmail()">
        <input
          id="email"
          class="champ__saisie"
          type="email"
          autocomplete="username"
          formControlName="email"
          [attr.aria-invalid]="erreurEmail() !== null"
          [attr.aria-describedby]="erreurEmail() !== null ? 'email-aide email-erreur' : 'email-aide'"
        />
      </lc-champ>

      <lc-champ libelle="Mot de passe" pour="mot-de-passe" [erreur]="erreurMotDePasse()">
        <div class="champ__groupe">
          <input
            id="mot-de-passe"
            class="champ__saisie"
            [type]="motDePasseVisible() ? 'text' : 'password'"
            autocomplete="current-password"
            formControlName="motDePasse"
            [attr.aria-invalid]="erreurMotDePasse() !== null"
            [attr.aria-describedby]="erreurMotDePasse() !== null ? 'mot-de-passe-erreur' : null"
          />
          <button
            type="button"
            class="bouton bouton--icone"
            [attr.aria-pressed]="motDePasseVisible()"
            (click)="motDePasseVisible.set(!motDePasseVisible())"
          >
            <lc-icone [nom]="motDePasseVisible() ? 'oeil-barre' : 'oeil'" />
            <span>{{ motDePasseVisible() ? 'Masquer' : 'Afficher' }}</span>
          </button>
        </div>
      </lc-champ>

      <button type="submit" class="bouton bouton--principal bouton--large" [disabled]="envoiEnCours()">
        {{ envoiEnCours() ? 'Connexion…' : 'Se connecter' }}
      </button>
    </form>
  `,
})
export class FormulaireConnexionComponent {
  private readonly acces = inject(AccesService);
  private readonly router = inject(Router);
  private readonly parcours = inject(PARCOURS_CONNEXION);

  /** Codes après lesquels le mot de passe reste dans son champ (sinon il est effacé). */
  readonly garderMotDePasse = input<readonly string[]>([]);
  /** Une connexion part (saisie valide). */
  readonly envoi = output<void>();
  /** La connexion est refusée : le `code` de l'erreur, ou `null` sans réponse de l'API. */
  readonly refus = output<string | null>();

  protected readonly formulaire = new FormGroup({
    email: new FormControl('', { nonNullable: true }),
    motDePasse: new FormControl('', { nonNullable: true }),
  });

  protected readonly motDePasseVisible = signal(false);
  protected readonly envoiEnCours = signal(false);
  protected readonly erreurEmail = signal<string | null>(null);
  protected readonly erreurMotDePasse = signal<string | null>(null);
  protected readonly message = signal<MessageAffiche | null>(inject(MessageNavigation).consommer());

  async envoyer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    const saisie = this.lireSaisie();
    if (saisie === null) {
      return;
    }

    this.envoiEnCours.set(true);
    this.message.set(null);
    this.envoi.emit();
    try {
      // Le mot de passe est envoyé tel que saisi (CLAUDE.md §14 : aucune espace retirée).
      const etape = await this.acces.connecter(saisie.email, saisie.motDePasse);
      await this.router.navigate([
        etape === 'second_facteur' ? this.parcours.pageSecondFacteur : this.parcours.pageApresConnexion,
      ]);
    } catch (erreur) {
      const code = codeErreur(erreur);
      if (code === null || !this.garderMotDePasse().includes(code)) {
        this.effacerMotDePasse();
      }
      this.message.set({ genre: 'erreur', texte: messagePourErreur(decoderErreur(erreur)) });
      this.refus.emit(code);
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  /** Email et mot de passe saisis, ou `null` (et l'erreur sous le champ vide). */
  lireSaisie(): { email: string; motDePasse: string } | null {
    const { email, motDePasse } = this.formulaire.getRawValue();
    this.erreurEmail.set(email.trim() === '' ? 'Saisissez votre email.' : null);
    this.erreurMotDePasse.set(motDePasse === '' ? 'Saisissez votre mot de passe.' : null);
    if (this.erreurEmail() !== null || this.erreurMotDePasse() !== null) {
      return null;
    }
    return { email, motDePasse };
  }

  effacerMotDePasse(): void {
    this.formulaire.controls.motDePasse.reset();
  }
}
