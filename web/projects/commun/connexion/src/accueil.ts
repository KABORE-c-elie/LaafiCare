import { ChangeDetectionStrategy, Component, inject, input } from '@angular/core';
import { ESPACE_APPLICATION } from 'commun';

/**
 * Écran d'accueil : panneau de l'application à gauche, contenu projeté à
 * droite (formulaire de connexion, second facteur, vérification de l'email).
 * L'élément hôte porte la grille (`.accueil`).
 */
@Component({
  selector: 'lc-accueil',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'accueil' },
  template: `
    <section class="accueil__panneau" aria-labelledby="titre-espace">
      <img class="accueil__logo" [src]="espace.logo" alt="LaafiCare" width="192" height="40" />
      <h1 class="accueil__titre" id="titre-espace">{{ espace.titre }}</h1>
      @if (avecPhrase()) {
        <p class="accueil__phrase">{{ espace.phrase }}</p>
      }
    </section>

    <main class="accueil__formulaire">
      <ng-content />
    </main>
  `,
})
export class EcranAccueilComponent {
  protected readonly espace = inject(ESPACE_APPLICATION);
  readonly avecPhrase = input(false);
}
