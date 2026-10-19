import { ChangeDetectionStrategy, Component, input } from '@angular/core';

export type NomIcone =
  | 'info'
  | 'erreur'
  | 'attente'
  | 'en-cours'
  | 'valide'
  | 'paye'
  | 'rejete'
  | 'oeil'
  | 'oeil-barre';

/**
 * Icônes dessinées dans le gabarit (aucun innerHTML). Toujours accompagnées
 * d'un texte : la couleur ni l'icône ne portent seules l'information
 * (WCAG 2.2, 1.4.1), d'où `aria-hidden`.
 */
@Component({
  selector: 'lc-icone',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'icone', 'aria-hidden': 'true' },
  template: `
    <svg viewBox="0 0 24 24" width="20" height="20" focusable="false">
      @switch (nom()) {
        @case ('info') {
          <circle cx="12" cy="12" r="9" />
          <path d="M12 11v6" />
          <path d="M12 7.5v.5" />
        }
        @case ('erreur') {
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v6" />
          <path d="M12 16.5v.5" />
        }
        @case ('attente') {
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v5l3 2" />
        }
        @case ('en-cours') {
          <path d="M4 12a8 8 0 0 1 13.7-5.6" />
          <path d="M18 3v4h-4" />
          <path d="M20 12a8 8 0 0 1-13.7 5.6" />
          <path d="M6 21v-4h4" />
        }
        @case ('valide') {
          <circle cx="12" cy="12" r="9" />
          <path d="M8 12.5l3 3 5-6" />
        }
        @case ('paye') {
          <rect x="3" y="6" width="18" height="12" rx="2" />
          <circle cx="12" cy="12" r="2.5" />
        }
        @case ('rejete') {
          <circle cx="12" cy="12" r="9" />
          <path d="M9 9l6 6" />
          <path d="M15 9l-6 6" />
        }
        @case ('oeil') {
          <path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" />
          <circle cx="12" cy="12" r="3" />
        }
        @case ('oeil-barre') {
          <path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" />
          <circle cx="12" cy="12" r="3" />
          <path d="M3 3l18 18" />
        }
      }
    </svg>
  `,
})
export class IconeComponent {
  readonly nom = input.required<NomIcone>();
}
