import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { FormControl, FormGroup, ReactiveFormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import {
  AccesService,
  ChampComponent,
  IconeComponent,
  MessageAffiche,
  MessageEtatComponent,
  MessageNavigation,
  decoderErreur,
  codeErreur,
  messagePourErreur,
} from 'commun';
import { RenvoiLien } from '../renvoi-lien';

/** Connexion professionnelle (docs/api.md §2.4), et renvoi du lien sur 403 `email_non_verifie` (§2.7). */
@Component({
  selector: 'lc-connexion',
  imports: [ReactiveFormsModule, ChampComponent, IconeComponent, MessageEtatComponent],
  providers: [RenvoiLien],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="accueil">
      <section class="accueil__panneau" aria-labelledby="titre-espace">
        <img class="accueil__logo" src="/logo-laaficare.svg" alt="LaafiCare" width="192" height="40" />
        <h1 class="accueil__titre" id="titre-espace">Espace Assurance MUNASEB</h1>
        <p class="accueil__phrase">Suivez et traitez les demandes de remboursement des adhérents.</p>
      </section>

      <main class="accueil__formulaire">
        <form class="carte formulaire" [formGroup]="formulaire" (ngSubmit)="envoyer()" novalidate>
          <h2 class="formulaire__titre">Connexion</h2>

          @if (message(); as m) {
            <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
          }
          @if (emailNonVerifie()) {
            @if (renvoi.message(); as m) {
              <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
            }
            <button
              type="button"
              class="bouton bouton--secondaire bouton--large"
              [disabled]="renvoi.bloque()"
              (click)="renvoyerLien()"
            >
              {{ renvoi.libelle() }}
            </button>
          }

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
      </main>
    </div>
  `,
})
export class ConnexionPage {
  private readonly acces = inject(AccesService);
  private readonly router = inject(Router);
  protected readonly renvoi = inject(RenvoiLien);

  protected readonly formulaire = new FormGroup({
    email: new FormControl('', { nonNullable: true }),
    motDePasse: new FormControl('', { nonNullable: true }),
  });

  protected readonly motDePasseVisible = signal(false);
  protected readonly envoiEnCours = signal(false);
  protected readonly erreurEmail = signal<string | null>(null);
  protected readonly erreurMotDePasse = signal<string | null>(null);
  protected readonly message = signal<MessageAffiche | null>(inject(MessageNavigation).consommer());
  /** Dernière réponse : 403 `email_non_verifie` ; le bouton « Renvoyer le lien » s'affiche. */
  protected readonly emailNonVerifie = signal(false);

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
    this.emailNonVerifie.set(false);
    this.renvoi.message.set(null);
    try {
      // Le mot de passe est envoyé tel que saisi (CLAUDE.md §14 : aucune espace retirée).
      const etape = await this.acces.connecter(saisie.email, saisie.motDePasse);
      await this.router.navigate([etape === 'second_facteur' ? '/second-facteur' : '/espaces']);
    } catch (erreur) {
      // Sur `email_non_verifie`, le mot de passe reste dans le champ (masqué) :
      // « Renvoyer le lien » le réutilise une fois, puis il est effacé.
      if (codeErreur(erreur) === 'email_non_verifie') {
        this.emailNonVerifie.set(true);
      } else {
        this.formulaire.controls.motDePasse.reset();
      }
      this.message.set({ genre: 'erreur', texte: messagePourErreur(decoderErreur(erreur)) });
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  async renvoyerLien(): Promise<void> {
    const saisie = this.lireSaisie();
    if (saisie === null) {
      return;
    }
    try {
      await this.renvoi.renvoyer(saisie.email, saisie.motDePasse);
    } finally {
      // Gardé le temps de cet envoi seulement.
      this.formulaire.controls.motDePasse.reset();
    }
  }

  private lireSaisie(): { email: string; motDePasse: string } | null {
    const { email, motDePasse } = this.formulaire.getRawValue();
    this.erreurEmail.set(email.trim() === '' ? 'Saisissez votre email.' : null);
    this.erreurMotDePasse.set(motDePasse === '' ? 'Saisissez votre mot de passe.' : null);
    if (this.erreurEmail() !== null || this.erreurMotDePasse() !== null) {
      return null;
    }
    return { email, motDePasse };
  }
}
