import { ChangeDetectionStrategy, Component, input } from '@angular/core';
import { IconeComponent } from './icone';

/**
 * Champ de formulaire : libellé, contrôle projeté, aide facultative, erreur de saisie.
 * L'erreur est neutre (gris-bleu), jamais rouge ni orange (CLAUDE.md §6), avec
 * une icône et un texte. Le contrôle projeté doit porter `id` = `pour` et
 * `aria-describedby` listant `pour + '-aide'` (si une aide est donnée) et, en
 * erreur, `pour + '-erreur'`, avec `aria-invalid="true"`.
 */
@Component({
  selector: 'lc-champ',
  imports: [IconeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="champ" [class.champ--erreur]="erreur() !== null">
      <label class="champ__libelle" [attr.for]="pour()">{{ libelle() }}</label>
      <ng-content />
      @if (aide(); as texte) {
        <p class="champ__aide" [id]="pour() + '-aide'">{{ texte }}</p>
      }
      @if (erreur(); as texte) {
        <p class="champ__erreur" [id]="pour() + '-erreur'">
          <lc-icone nom="erreur" />
          <span>{{ texte }}</span>
        </p>
      }
    </div>
  `,
})
export class ChampComponent {
  readonly libelle = input.required<string>();
  readonly pour = input.required<string>();
  readonly aide = input<string | null>(null);
  readonly erreur = input<string | null>(null);
}
