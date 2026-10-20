import { ChangeDetectionStrategy, Component, DOCUMENT, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { RouterOutlet } from '@angular/router';
import { SwUpdate } from '@angular/service-worker';
import { MESSAGE_HORS_CONNEXION, MessageEtatComponent, ReseauService } from 'commun';
import { filter } from 'rxjs';

@Component({
  selector: 'lc-root',
  imports: [RouterOutlet, MessageEtatComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bandeaux">
      @if (!reseau.enLigne()) {
        <lc-message-etat genre="info" [texte]="horsConnexion" />
      }
      @if (nouvelleVersion()) {
        <div class="message message--info bandeau-version" role="status">
          <span>{{ texteVersion() }}</span>
          <button type="button" class="bouton bouton--secondaire" (click)="recharger()">Recharger maintenant</button>
        </div>
      }
    </div>
    <router-outlet />
  `,
})
export class App {
  protected readonly reseau = inject(ReseauService);
  private readonly document = inject(DOCUMENT);

  protected readonly horsConnexion = MESSAGE_HORS_CONNEXION;
  protected readonly nouvelleVersion = signal(false);
  protected readonly texteVersion = signal('');

  constructor() {
    const miseAJour = inject(SwUpdate);
    if (!miseAJour.isEnabled) {
      return;
    }
    // Doc Angular (service workers, « Communicating with the service worker ») :
    // proposer le rechargement et attendre l'accord, plutôt que de recharger
    // de force. Ici, rien ne se recharge sans un clic de l'agent.
    miseAJour.versionUpdates
      .pipe(
        filter((evenement) => evenement.type === 'VERSION_READY'),
        takeUntilDestroyed(),
      )
      .subscribe(() => {
        this.texteVersion.set(
          'Une nouvelle version de l’application est disponible. Rechargez quand vous aurez terminé votre action : il faudra vous reconnecter.',
        );
        this.nouvelleVersion.set(true);
      });
    // Version en cache inutilisable (UnrecoverableStateEvent) : seul un rechargement la répare.
    miseAJour.unrecoverable.pipe(takeUntilDestroyed()).subscribe(() => {
      this.texteVersion.set('L’application doit être rechargée pour continuer à fonctionner.');
      this.nouvelleVersion.set(true);
    });
  }

  recharger(): void {
    this.document.location.reload();
  }
}
