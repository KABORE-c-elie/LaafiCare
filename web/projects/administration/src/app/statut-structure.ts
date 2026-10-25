import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { IconeComponent, StatutStructure } from 'commun';
import { LIBELLES_STATUT_STRUCTURE, PASTILLES_STRUCTURE } from './structures';

/** Statut d'une structure : icône et texte, jamais la couleur seule (WCAG 2.2, 1.4.1). */
@Component({
  selector: 'lc-statut-structure',
  imports: [IconeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <span [class]="'pastille ' + pastille().classe">
      <lc-icone [nom]="pastille().icone" />
      <span>{{ libelle() }}</span>
    </span>
  `,
})
export class StatutStructureComponent {
  readonly statut = input.required<StatutStructure>();
  protected readonly pastille = computed(() => PASTILLES_STRUCTURE[this.statut()]);
  protected readonly libelle = computed(() => LIBELLES_STATUT_STRUCTURE[this.statut()]);
}
