import { DOCUMENT } from '@angular/common';
import { ChangeDetectionStrategy, Component, ElementRef, inject, signal, viewChild } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { FormControl, FormGroup, ReactiveFormsModule } from '@angular/forms';
import { NavigationEnd, Router, RouterLink } from '@angular/router';
import { filter } from 'rxjs';
import { AccesService, ChampComponent, MESSAGES, MessageEtatComponent, decoderErreur, messagePourErreur } from 'commun';
import { EcranAccueilComponent } from 'commun/connexion';
import { RenvoiLien } from '../renvoi-lien';

/**
 * Lit le jeton du lien `/verifier-email#jeton=…` (docs/api.md §2.7), puis
 * retire aussitôt le fragment de l'adresse avec `history.replaceState()`
 * (MDN, History: replaceState() method : « modifies the current history
 * entry », sans rechargement ni nouvelle entrée) : le jeton ne reste ni à
 * l'écran, ni dans l'historique de cette entrée. L'état de l'historique est
 * gardé tel quel (le routeur d'Angular y range son numéro de navigation).
 * Le fragment n'est jamais envoyé au serveur par le navigateur (RFC 3986 §3.5).
 */
export function lireEtEffacerJeton(document: Document): string | null {
  const fragment = document.location.hash;
  const historique = document.defaultView?.history;
  if (fragment !== '' && historique !== undefined) {
    historique.replaceState(historique.state, '', document.location.pathname + document.location.search);
  }
  const jeton = new URLSearchParams(fragment.slice(1)).get('jeton');
  return jeton === null || jeton === '' ? null : jeton;
}

type Etat = 'confirmation' | 'verifie' | 'invalide';

/**
 * Vérification de l'email professionnel (docs/api.md §2.7), sans connexion.
 * Ouvrir le lien ne vérifie rien : le jeton n'est envoyé qu'au clic sur
 * « Confirmer mon adresse », par POST (les filtres de messagerie ouvrent les
 * liens tout seuls). Le jeton reste en mémoire dans cette page seulement
 * (ni stockage du navigateur, ni journal), et il est oublié dès que le
 * serveur a répondu 204 ou `lien_invalide`.
 */
@Component({
  selector: 'lc-verifier-email',
  imports: [ReactiveFormsModule, RouterLink, ChampComponent, MessageEtatComponent, EcranAccueilComponent],
  providers: [RenvoiLien],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-accueil>
        <div class="carte formulaire">
          <h2 #titre class="formulaire__titre" tabindex="-1">Vérification de votre email</h2>

          @switch (etat()) {
            @case ('confirmation') {
              <p>Confirmez votre adresse email pour pouvoir vous connecter.</p>
              @if (message(); as texte) {
                <lc-message-etat genre="erreur" [texte]="texte" />
              }
              <button
                type="button"
                class="bouton bouton--principal bouton--large"
                [disabled]="envoiEnCours()"
                (click)="confirmer()"
              >
                {{ envoiEnCours() ? 'Confirmation…' : 'Confirmer mon adresse' }}
              </button>
            }
            @case ('verifie') {
              <lc-message-etat genre="info" [texte]="textes.emailVerifie" />
              <a class="bouton bouton--principal bouton--large" routerLink="/connexion">Aller à la connexion</a>
            }
            @case ('invalide') {
              <lc-message-etat genre="erreur" [texte]="textes.lienInvalide" />
              <form class="formulaire" [formGroup]="formulaire" (ngSubmit)="renvoyer()" novalidate>
                @if (renvoi.message(); as m) {
                  <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
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
                  <input
                    id="mot-de-passe"
                    class="champ__saisie"
                    type="password"
                    autocomplete="current-password"
                    formControlName="motDePasse"
                    [attr.aria-invalid]="erreurMotDePasse() !== null"
                    [attr.aria-describedby]="erreurMotDePasse() !== null ? 'mot-de-passe-erreur' : null"
                  />
                </lc-champ>

                <button type="submit" class="bouton bouton--principal bouton--large" [disabled]="renvoi.bloque()">
                  {{ renvoi.libelle() }}
                </button>
              </form>
              <a class="bouton bouton--lien" routerLink="/connexion">Revenir à la connexion</a>
            }
          }
        </div>
    </lc-accueil>
  `,
})
export class VerifierEmailPage {
  private readonly acces = inject(AccesService);
  private readonly document = inject(DOCUMENT);
  protected readonly renvoi = inject(RenvoiLien);
  private readonly titre = viewChild.required<ElementRef<HTMLElement>>('titre');

  protected readonly textes = MESSAGES;
  private jeton: string | null = lireEtEffacerJeton(this.document);
  // Pas de jeton dans l'adresse : même message que `lien_invalide` (porteur, 2026-10-21).
  protected readonly etat = signal<Etat>(this.jeton === null ? 'invalide' : 'confirmation');
  protected readonly envoiEnCours = signal(false);
  protected readonly message = signal<string | null>(null);

  protected readonly formulaire = new FormGroup({
    email: new FormControl('', { nonNullable: true }),
    motDePasse: new FormControl('', { nonNullable: true }),
  });
  protected readonly erreurEmail = signal<string | null>(null);
  protected readonly erreurMotDePasse = signal<string | null>(null);

  constructor() {
    // Un second lien ouvert dans le même onglet ne change que le fragment : la
    // page n'est pas rechargée et le routeur réutilise ce composant. Le
    // fragment est donc relu à chaque fin de navigation, une fois l'adresse
    // posée par le routeur (essai du 2026-10-21 dans Chrome : sans cela, le
    // jeton restait dans l'adresse).
    inject(Router)
      .events.pipe(
        filter((evenement) => evenement instanceof NavigationEnd),
        takeUntilDestroyed(),
      )
      .subscribe(() => this.lireNouveauLien());
  }

  async confirmer(): Promise<void> {
    const jeton = this.jeton;
    if (jeton === null || this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      await this.acces.verifierEmail(jeton);
      this.jeton = null;
      this.changerEtat('verifie');
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      if (decodee.genre === 'api' && decodee.code === 'lien_invalide') {
        this.jeton = null;
        this.changerEtat('invalide');
      } else {
        // Réseau ou panne : le jeton reste en mémoire, la personne peut réessayer.
        this.message.set(messagePourErreur(decodee));
      }
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  async renvoyer(): Promise<void> {
    const { email, motDePasse } = this.formulaire.getRawValue();
    this.erreurEmail.set(email.trim() === '' ? 'Saisissez votre email.' : null);
    this.erreurMotDePasse.set(motDePasse === '' ? 'Saisissez votre mot de passe.' : null);
    if (this.erreurEmail() !== null || this.erreurMotDePasse() !== null) {
      return;
    }
    try {
      // Mot de passe envoyé tel que saisi (CLAUDE.md §14), puis effacé du formulaire.
      await this.renvoi.renvoyer(email, motDePasse);
    } finally {
      this.formulaire.controls.motDePasse.reset();
    }
  }

  private lireNouveauLien(): void {
    const jeton = lireEtEffacerJeton(this.document);
    if (jeton === null) {
      return;
    }
    this.jeton = jeton;
    this.message.set(null);
    this.renvoi.message.set(null);
    this.changerEtat('confirmation');
  }

  /** Le bouton cliqué disparaît avec l'état : le focus passe au titre (WCAG 2.2, 2.4.3). */
  private changerEtat(etat: Etat): void {
    this.etat.set(etat);
    this.titre().nativeElement.focus();
  }
}
