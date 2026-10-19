import { ChangeDetectionStrategy, Component } from '@angular/core';
import { CadreComponent } from '../cadre';

/** Liste des demandes de remboursement (docs/api.md §2.3.4). */
@Component({
  selector: 'lc-demandes',
  imports: [CadreComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <h1 class="page__titre">Demandes de remboursement</h1>
    </lc-cadre>
  `,
})
export class DemandesPage {}
