import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { StatutDemande } from '../api/types';
import { IconeComponent, NomIcone } from './icone';

const LIBELLES: Readonly<Record<StatutDemande, string>> = {
  en_attente: 'En attente',
  en_cours: 'En cours',
  valide: 'Validée',
  paye: 'Payée',
  rejete: 'Rejetée',
};

const ICONES: Readonly<Record<StatutDemande, NomIcone>> = {
  en_attente: 'attente',
  en_cours: 'en-cours',
  valide: 'valide',
  paye: 'paye',
  rejete: 'rejete',
};

/** Statut d'une demande (docs/api.md §2.3.2) : icône et texte, jamais la couleur seule (WCAG 2.2, 1.4.1). */
@Component({
  selector: 'lc-pastille-statut',
  imports: [IconeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <span [class]="'pastille pastille--' + statut()">
      <lc-icone [nom]="icone()" />
      <span>{{ libelle() }}</span>
    </span>
  `,
})
export class PastilleStatutComponent {
  readonly statut = input.required<StatutDemande>();
  protected readonly libelle = computed(() => LIBELLES[this.statut()]);
  protected readonly icone = computed(() => ICONES[this.statut()]);
}

export function libelleStatut(statut: StatutDemande): string {
  return LIBELLES[statut];
}
