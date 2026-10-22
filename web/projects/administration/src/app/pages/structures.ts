import { ChangeDetectionStrategy, Component } from '@angular/core';
import { CadreComponent } from '../cadre';

/** Demandes de structure (docs/api.md §2.9). La liste et les décisions arrivent au lot D. */
@Component({
  selector: 'lc-structures',
  imports: [CadreComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <h1 class="page__titre">Demandes de structure</h1>
    </lc-cadre>
  `,
})
export class StructuresPage {}
