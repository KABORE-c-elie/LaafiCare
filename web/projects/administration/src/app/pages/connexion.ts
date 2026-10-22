import { ChangeDetectionStrategy, Component } from '@angular/core';
import { EcranAccueilComponent, FormulaireConnexionComponent } from 'commun/connexion';
import { MESSAGE_EMAIL_NON_VERIFIE_ADMINISTRATEUR } from '../espace';

/**
 * Connexion administrateur (docs/api.md §2.5) : le formulaire commun, avec le
 * parcours administrateur (second facteur toujours, ou réenrôlement).
 */
@Component({
  selector: 'lc-connexion',
  imports: [EcranAccueilComponent, FormulaireConnexionComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-accueil [avecPhrase]="true">
      <lc-formulaire-connexion [messagesParCode]="messages" />
    </lc-accueil>
  `,
})
export class ConnexionPage {
  protected readonly messages = { email_non_verifie: MESSAGE_EMAIL_NON_VERIFIE_ADMINISTRATEUR };
}
