import { ChangeDetectionStrategy, Component } from '@angular/core';
import { RouterOutlet } from '@angular/router';

/**
 * Racine de l'application d'administration. Pas de service worker (décision
 * B11 du porteur) : aucun besoin hors connexion, et l'application vient
 * toujours du serveur, jamais d'une ancienne version en cache.
 */
@Component({
  selector: 'lc-root',
  imports: [RouterOutlet],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<router-outlet />`,
})
export class App {}
