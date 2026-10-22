import { ChangeDetectionStrategy, Component, ElementRef, inject, signal, viewChild } from '@angular/core';
import { FormControl, FormGroup, ReactiveFormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import {
  AccesService,
  ChampComponent,
  MESSAGES,
  MessageEtatComponent,
  MessageNavigation,
  PARCOURS_CONNEXION,
  SessionService,
  decoderErreur,
  messagePourErreur,
} from 'commun';
import { EcranAccueilComponent } from './accueil';

/**
 * Second temps de la connexion (docs/api.md §2.4, et §2.5 pour les
 * administrateurs) : un code à 6 chiffres ou un code de secours. Pages
 * suivantes lues dans `PARCOURS_CONNEXION`.
 */
@Component({
  selector: 'lc-second-facteur',
  imports: [ReactiveFormsModule, ChampComponent, MessageEtatComponent, EcranAccueilComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-accueil>
      <form class="carte formulaire" [formGroup]="formulaire" (ngSubmit)="envoyer()" novalidate>
        <h2 class="formulaire__titre">Code de vérification</h2>
        @if (secours()) {
          <p>Saisissez l'un de vos codes de secours.</p>
        } @else {
          <p>Saisissez le code à 6 chiffres affiché par votre application d'authentification.</p>
        }

        @if (message(); as texte) {
          <lc-message-etat genre="erreur" [texte]="texte" />
        }

        @if (secours()) {
          <lc-champ libelle="Code de secours (XXXXX-XXXXX)" pour="code-secours">
            <input
              #saisie
              id="code-secours"
              class="champ__saisie"
              type="text"
              autocomplete="off"
              autocapitalize="characters"
              spellcheck="false"
              maxlength="11"
              [formControl]="code"
            />
          </lc-champ>
        } @else {
          <lc-champ libelle="Code à 6 chiffres" pour="code">
            <input
              #saisie
              id="code"
              class="champ__saisie"
              type="text"
              inputmode="numeric"
              autocomplete="one-time-code"
              maxlength="6"
              [formControl]="code"
            />
          </lc-champ>
        }

        <button type="submit" class="bouton bouton--principal bouton--large" [disabled]="envoiEnCours()">
          {{ envoiEnCours() ? 'Vérification…' : 'Valider' }}
        </button>
        <button type="button" class="bouton bouton--lien" (click)="basculer()">
          {{ secours() ? 'Utiliser le code de l’application' : 'Utiliser un code de secours' }}
        </button>
        <button type="button" class="bouton bouton--lien" (click)="revenir()">Revenir à la connexion</button>
      </form>
    </lc-accueil>
  `,
})
export class SecondFacteurPage {
  private readonly acces = inject(AccesService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  private readonly messages = inject(MessageNavigation);
  private readonly parcours = inject(PARCOURS_CONNEXION);
  private readonly saisie = viewChild<ElementRef<HTMLInputElement>>('saisie');

  protected readonly code = new FormControl('', { nonNullable: true });
  // Avec les formulaires réactifs, seule la directive `[formGroup]`
  // (FormGroupDirective) émet `ngSubmit` ; `NgForm` est dans FormsModule. Sans
  // elle, « Valider » déclenchait la soumission native du navigateur : la page
  // se rechargeait et la session (en mémoire) était perdue (essai du 2026-10-22).
  protected readonly formulaire = new FormGroup({ code: this.code });
  protected readonly secours = signal(false);
  protected readonly envoiEnCours = signal(false);
  protected readonly message = signal<string | null>(null);

  basculer(): void {
    this.secours.set(!this.secours());
    this.code.reset();
    this.message.set(null);
  }

  revenir(): void {
    this.session.effacer();
    void this.router.navigate([this.parcours.pageConnexion]);
  }

  async envoyer(): Promise<void> {
    const valeur = this.code.value.trim();
    if (this.envoiEnCours() || valeur === '') {
      this.saisie()?.nativeElement.focus();
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      // §2.4 : un seul des deux champs, jamais les deux.
      await this.acces.validerSecondFacteur(this.secours() ? { code_secours: valeur } : { code: valeur });
      await this.router.navigate([this.parcours.pageApresConnexion]);
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      if (decodee.genre === 'api' && decodee.code === 'jeton_invalide') {
        // §2.4 : « Sur 401 jeton_invalide : elle revient à l'écran de connexion. »
        this.session.effacer();
        this.messages.poser({ genre: 'erreur', texte: MESSAGES.delaiDepasse });
        await this.router.navigate([this.parcours.pageConnexion]);
        return;
      }
      // §2.4 : « Sur 401 code_invalide : l'application vide le champ du code. »
      this.code.reset();
      this.message.set(messagePourErreur(decodee));
      this.saisie()?.nativeElement.focus();
    } finally {
      this.envoiEnCours.set(false);
    }
  }
}
