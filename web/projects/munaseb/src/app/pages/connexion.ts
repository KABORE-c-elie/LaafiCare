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
  messagePourErreur,
} from 'commun';

/** Connexion professionnelle (docs/api.md §2.4). */
@Component({
  selector: 'lc-connexion',
  imports: [ReactiveFormsModule, ChampComponent, IconeComponent, MessageEtatComponent],
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

          <lc-champ libelle="Email" pour="email" [erreur]="erreurEmail()">
            <input
              id="email"
              class="champ__saisie"
              type="email"
              autocomplete="username"
              formControlName="email"
              [attr.aria-invalid]="erreurEmail() !== null"
              [attr.aria-describedby]="erreurEmail() !== null ? 'email-erreur' : null"
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
    const { email, motDePasse } = this.formulaire.getRawValue();
    this.erreurEmail.set(email.trim() === '' ? 'Saisissez votre email.' : null);
    this.erreurMotDePasse.set(motDePasse === '' ? 'Saisissez votre mot de passe.' : null);
    if (this.erreurEmail() !== null || this.erreurMotDePasse() !== null) {
      return;
    }

    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      // Le mot de passe est envoyé tel que saisi (CLAUDE.md §14 : aucune espace retirée).
      const etape = await this.acces.connecter(email, motDePasse);
      await this.router.navigate([etape === 'second_facteur' ? '/second-facteur' : '/espaces']);
    } catch (erreur) {
      this.formulaire.controls.motDePasse.reset();
      this.message.set({ genre: 'erreur', texte: messagePourErreur(decoderErreur(erreur)) });
    } finally {
      this.envoiEnCours.set(false);
    }
  }
}
