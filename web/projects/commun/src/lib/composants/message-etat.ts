import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { IconeComponent } from './icone';

/**
 * Message d'état d'un écran. `erreur` : couleur neutre (jamais rouge ni orange,
 * CLAUDE.md §6). Rôle ARIA `alert` pour une erreur, `status` sinon (WAI-ARIA 1.2) :
 * le lecteur d'écran annonce le message sans déplacer le focus.
 */
@Component({
  selector: 'lc-message-etat',
  imports: [IconeComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div [class]="'message message--' + genre()" [attr.role]="genre() === 'erreur' ? 'alert' : 'status'">
      <lc-icone [nom]="icone()" />
      <span>{{ texte() }}</span>
    </div>
  `,
})
export class MessageEtatComponent {
  readonly genre = input<'info' | 'erreur'>('info');
  readonly texte = input.required<string>();
  protected readonly icone = computed(() => (this.genre() === 'erreur' ? 'erreur' : 'info'));
}
