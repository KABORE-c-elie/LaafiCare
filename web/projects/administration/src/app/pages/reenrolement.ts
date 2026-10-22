import { ChangeDetectionStrategy, Component, DestroyRef, ElementRef, computed, inject, signal, viewChild } from '@angular/core';
import { FormControl, FormGroup, ReactiveFormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import {
  ChampComponent,
  MESSAGES,
  MessageEtatComponent,
  MessageNavigation,
  SessionService,
  decoderErreur,
  messagePourErreur,
} from 'commun';
import { EcranAccueilComponent } from 'commun/connexion';
import { QrCodeComponent } from '../qr-code';
import { ReenrolementService } from '../reenrolement';

// Textes de la page. « Mot de passe incorrect. » est celui de docs/api.md §2.5.
export const MESSAGES_REENROLEMENT = {
  motDePasseVide: 'Saisissez votre mot de passe.',
  motDePasseIncorrect: 'Mot de passe incorrect.',
  codeVide: 'Saisissez le code à 6 chiffres.',
  termine: 'Second facteur enregistré. Connectez-vous avec votre mot de passe, puis un code de votre application.',
} as const;

type Etape = 'mot_de_passe' | 'enregistrement' | 'codes';

/** Clé base32 par groupes de 4, pour la recopier à la main sans se tromper. */
export function grouperCle(cle: string): string {
  return (cle.match(/.{1,4}/gu) ?? []).join(' ');
}

/**
 * Réenrôlement d'un administrateur sans second facteur (docs/api.md §2.5),
 * pendant la fenêtre de 10 minutes ouverte par l'équipe :
 * 1. le mot de passe est redemandé (`commencer`) ;
 * 2. le QR code est dessiné dans le navigateur à partir de `url_otpauth`, la
 *    clé base32 s'affiche à la demande, puis le premier code (`confirmer`) ;
 * 3. les 10 codes de secours sont montrés une seule fois, et la personne
 *    confirme les avoir notés avant de revenir à la connexion (aucune session
 *    n'est ouverte, §2.5).
 * L'adresse otpauth, la clé et les codes ne vivent que dans cette page, en
 * mémoire : ni stockage du navigateur, ni console, ni copie, téléchargement ou
 * impression (porteur, B3). Ils sont effacés dès qu'ils ne servent plus, et
 * quand on quitte la page.
 */
@Component({
  selector: 'lc-reenrolement',
  imports: [ReactiveFormsModule, ChampComponent, MessageEtatComponent, EcranAccueilComponent, QrCodeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-accueil>
      <div class="carte formulaire">
        <h2 #titre class="formulaire__titre" tabindex="-1">Enregistrer votre second facteur</h2>

        @if (message(); as texte) {
          <lc-message-etat genre="erreur" [texte]="texte" />
        }

        @switch (etape()) {
          @case ('mot_de_passe') {
            <p>
              Votre compte n'a pas encore de second facteur, obligatoire pour l'équipe LaafiCare. Pour l'enregistrer,
              confirmez d'abord votre mot de passe. Vous avez 10 minutes.
            </p>
            <form class="formulaire" [formGroup]="formulaireMotDePasse" (ngSubmit)="commencer()" novalidate>
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
              <button type="submit" class="bouton bouton--principal bouton--large" [disabled]="envoiEnCours()">
                {{ envoiEnCours() ? 'Vérification…' : 'Continuer' }}
              </button>
            </form>
          }
          @case ('enregistrement') {
            <p>
              Dans votre application d'authentification (par exemple Google Authenticator), ajoutez un compte en
              scannant ce QR code.
            </p>
            @if (urlOtpauth(); as url) {
              <lc-qr-code [texte]="url" libelle="QR code à scanner avec votre application d'authentification" />
            }
            <button
              type="button"
              class="bouton bouton--lien"
              [attr.aria-pressed]="cleVisible()"
              (click)="cleVisible.set(!cleVisible())"
            >
              {{ cleVisible() ? 'Masquer la clé' : 'Afficher la clé pour la saisir à la main' }}
            </button>
            @if (cleVisible()) {
              <p>Clé : <span class="cle">{{ cleGroupee() }}</span></p>
            }

            <form class="formulaire" [formGroup]="formulaireCode" (ngSubmit)="confirmer()" novalidate>
              <lc-champ libelle="Code à 6 chiffres affiché par l'application" pour="code" [erreur]="erreurCode()">
                <input
                  #saisieCode
                  id="code"
                  class="champ__saisie"
                  type="text"
                  inputmode="numeric"
                  autocomplete="one-time-code"
                  maxlength="6"
                  formControlName="code"
                  [attr.aria-invalid]="erreurCode() !== null"
                  [attr.aria-describedby]="erreurCode() !== null ? 'code-erreur' : null"
                />
              </lc-champ>
              <button type="submit" class="bouton bouton--principal bouton--large" [disabled]="envoiEnCours()">
                {{ envoiEnCours() ? 'Vérification…' : 'Valider' }}
              </button>
            </form>
            <button type="button" class="bouton bouton--lien" [disabled]="envoiEnCours()" (click)="recommencer()">
              Obtenir un nouveau QR code
            </button>
          }
          @case ('codes') {
            <p>
              Votre second facteur est enregistré. Voici vos 10 codes de secours : chacun ne sert qu'une fois, si vous
              perdez votre téléphone. <strong>Ils ne seront plus jamais affichés</strong> : notez-les maintenant et
              gardez-les en lieu sûr.
            </p>
            <ol class="codes-secours" aria-label="Codes de secours">
              @for (code of codesSecours(); track code) {
                <li>{{ code }}</li>
              }
            </ol>
            <label class="case">
              <input type="checkbox" [checked]="codesNotes()" (change)="noterCodes($event)" />
              <span>J'ai noté mes 10 codes de secours en lieu sûr.</span>
            </label>
            <button
              type="button"
              class="bouton bouton--principal bouton--large"
              [disabled]="!codesNotes()"
              (click)="terminer()"
            >
              Revenir à la connexion
            </button>
          }
        }
      </div>
    </lc-accueil>
  `,
})
export class ReenrolementPage {
  private readonly service = inject(ReenrolementService);
  private readonly session = inject(SessionService);
  private readonly router = inject(Router);
  private readonly messages = inject(MessageNavigation);
  private readonly titre = viewChild.required<ElementRef<HTMLElement>>('titre');
  private readonly saisieCode = viewChild<ElementRef<HTMLInputElement>>('saisieCode');

  protected readonly etape = signal<Etape>('mot_de_passe');
  protected readonly envoiEnCours = signal(false);
  protected readonly message = signal<string | null>(null);

  protected readonly formulaireMotDePasse = new FormGroup({
    motDePasse: new FormControl('', { nonNullable: true }),
  });
  protected readonly erreurMotDePasse = signal<string | null>(null);

  // Secrets : en mémoire dans cette page seulement.
  protected readonly urlOtpauth = signal<string | null>(null);
  private readonly cle = signal<string | null>(null);
  protected readonly cleVisible = signal(false);
  protected readonly cleGroupee = computed(() => grouperCle(this.cle() ?? ''));
  protected readonly formulaireCode = new FormGroup({ code: new FormControl('', { nonNullable: true }) });
  protected readonly erreurCode = signal<string | null>(null);

  protected readonly codesSecours = signal<readonly string[]>([]);
  protected readonly codesNotes = signal(false);

  constructor() {
    inject(DestroyRef).onDestroy(() => this.oublierSecrets());
  }

  async commencer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    const motDePasse = this.formulaireMotDePasse.controls.motDePasse.value;
    this.erreurMotDePasse.set(motDePasse === '' ? MESSAGES_REENROLEMENT.motDePasseVide : null);
    if (motDePasse === '') {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      const reponse = await this.service.commencer(motDePasse);
      this.urlOtpauth.set(reponse.url_otpauth);
      this.cle.set(reponse.cle_base32);
      this.cleVisible.set(false);
      this.changerEtape('enregistrement');
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      if (decodee.genre === 'api' && decodee.code === 'jeton_invalide') {
        await this.quitter();
        return;
      }
      this.message.set(
        decodee.genre === 'api' && decodee.code === 'identifiants_invalides'
          ? MESSAGES_REENROLEMENT.motDePasseIncorrect
          : messagePourErreur(decodee),
      );
    } finally {
      // Le mot de passe ne reste jamais dans le champ.
      this.formulaireMotDePasse.reset();
      this.envoiEnCours.set(false);
    }
  }

  async confirmer(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    const code = this.formulaireCode.controls.code.value.trim();
    this.erreurCode.set(code === '' ? MESSAGES_REENROLEMENT.codeVide : null);
    if (code === '') {
      this.saisieCode()?.nativeElement.focus();
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      const reponse = await this.service.confirmer(code);
      // Le second facteur est enregistré : le QR et la clé ne servent plus.
      this.urlOtpauth.set(null);
      this.cle.set(null);
      this.cleVisible.set(false);
      this.codesSecours.set(reponse.codes_secours);
      this.changerEtape('codes');
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      if (decodee.genre === 'api' && decodee.code === 'jeton_invalide') {
        await this.quitter();
        return;
      }
      // §2.5 : `code_invalide` « Code incorrect. » ; le champ est vidé.
      this.formulaireCode.reset();
      this.message.set(messagePourErreur(decodee));
      this.saisieCode()?.nativeElement.focus();
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  /** QR code perdu : `commencer` peut être relancé tant que la fenêtre est ouverte (§2.5), avec le mot de passe. */
  recommencer(): void {
    this.urlOtpauth.set(null);
    this.cle.set(null);
    this.cleVisible.set(false);
    this.formulaireCode.reset();
    this.message.set(null);
    this.changerEtape('mot_de_passe');
  }

  noterCodes(evenement: Event): void {
    this.codesNotes.set((evenement.target as HTMLInputElement).checked);
  }

  /** §2.5 : « Aucune session n'est ouverte » ; retour à la connexion, codes effacés. */
  async terminer(): Promise<void> {
    if (!this.codesNotes()) {
      return;
    }
    this.oublierSecrets();
    this.session.effacer();
    this.messages.poser({ genre: 'info', texte: MESSAGES_REENROLEMENT.termine });
    await this.router.navigate(['/connexion']);
  }

  /** Jeton de réenrôlement refusé (expiré, fenêtre fermée…, §2.5) : retour à la connexion. */
  private async quitter(): Promise<void> {
    this.oublierSecrets();
    this.session.effacer();
    this.messages.poser({ genre: 'erreur', texte: MESSAGES.delaiDepasse });
    await this.router.navigate(['/connexion']);
  }

  private oublierSecrets(): void {
    this.urlOtpauth.set(null);
    this.cle.set(null);
    this.codesSecours.set([]);
  }

  /** Le bouton cliqué disparaît avec l'étape : le focus passe au titre (WCAG 2.2, 2.4.3). */
  private changerEtape(etape: Etape): void {
    this.etape.set(etape);
    this.message.set(null);
    this.titre().nativeElement.focus();
  }
}
