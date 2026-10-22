import { ChangeDetectionStrategy, Component, inject, signal, viewChild } from '@angular/core';
import { MessageEtatComponent } from 'commun';
import { EcranAccueilComponent, FormulaireConnexionComponent } from 'commun/connexion';
import { RenvoiLien } from '../renvoi-lien';

/**
 * Connexion professionnelle (docs/api.md §2.4) : le formulaire commun
 * (`commun/connexion`), plus le renvoi du lien sur 403 `email_non_verifie` (§2.7).
 */
@Component({
  selector: 'lc-connexion',
  imports: [EcranAccueilComponent, FormulaireConnexionComponent, MessageEtatComponent],
  providers: [RenvoiLien],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-accueil [avecPhrase]="true">
      <lc-formulaire-connexion
        [garderMotDePasse]="garderMotDePasse"
        (envoi)="surEnvoi()"
        (refus)="emailNonVerifie.set($event === 'email_non_verifie')"
      >
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
      </lc-formulaire-connexion>
    </lc-accueil>
  `,
})
export class ConnexionPage {
  protected readonly renvoi = inject(RenvoiLien);
  private readonly formulaire = viewChild.required(FormulaireConnexionComponent);

  // Sur `email_non_verifie`, le mot de passe reste dans le champ (masqué) :
  // « Renvoyer le lien » le réutilise une fois, puis il est effacé.
  protected readonly garderMotDePasse = ['email_non_verifie'];
  /** Dernière réponse : 403 `email_non_verifie` ; le bouton « Renvoyer le lien » s'affiche. */
  protected readonly emailNonVerifie = signal(false);

  surEnvoi(): void {
    this.emailNonVerifie.set(false);
    this.renvoi.message.set(null);
  }

  async renvoyerLien(): Promise<void> {
    const saisie = this.formulaire().lireSaisie();
    if (saisie === null) {
      return;
    }
    try {
      await this.renvoi.renvoyer(saisie.email, saisie.motDePasse);
    } finally {
      // Gardé le temps de cet envoi seulement.
      this.formulaire().effacerMotDePasse();
    }
  }
}
