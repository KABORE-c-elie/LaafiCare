import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

/** En-tête des pages connectées : logo, nom de l'application, espace actif, actions. */
@Component({
  selector: 'lc-entete',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <header class="entete">
      <img class="entete__logo" [src]="logo()" alt="LaafiCare" width="128" height="27" />
      <span class="entete__titre">{{ titre() }}</span>
      @if (espace(); as nom) {
        <span class="entete__espace">{{ nom }}</span>
      }
      <span class="entete__espaceur"></span>
      @if (afficherChangerEspace()) {
        <button type="button" class="bouton bouton--secondaire" (click)="changerEspace.emit()">
          Changer d'espace
        </button>
      }
      <button type="button" class="bouton bouton--secondaire" (click)="deconnecter.emit()">Se déconnecter</button>
    </header>
  `,
})
export class EnteteComponent {
  readonly logo = input.required<string>();
  readonly titre = input.required<string>();
  readonly espace = input<string | null>(null);
  readonly afficherChangerEspace = input(true);
  readonly changerEspace = output();
  readonly deconnecter = output();
}
