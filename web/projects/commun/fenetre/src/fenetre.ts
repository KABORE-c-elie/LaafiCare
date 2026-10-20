import { Dialog, DialogConfig, DialogRef } from '@angular/cdk/dialog';
import { ComponentType } from '@angular/cdk/portal';
import { ChangeDetectionStrategy, Component, Injectable, inject, input } from '@angular/core';

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
