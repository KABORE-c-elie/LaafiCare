import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { Router } from '@angular/router';
import { AccesService, EnteteComponent, SessionService } from 'commun';

/** Mise en page des écrans connectés : en-tête commun, puis le contenu de la page. */
@Component({
  selector: 'lc-cadre',
  imports: [EnteteComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-entete
      logo="/logo-laaficare.svg"
      titre="Assurance MUNASEB"
      [espace]="espace()"
      [afficherChangerEspace]="afficherChangerEspace()"
      (changerEspace)="changerEspace()"
      (deconnecter)="deconnecter()"
    />
    <main class="page" id="contenu">
      <ng-content />
    </main>
  `,
})
export class CadreComponent {
  private readonly session = inject(SessionService);
  private readonly acces = inject(AccesService);
  private readonly router = inject(Router);

  readonly afficherChangerEspace = input(true);
  protected readonly espace = computed(() => this.session.affectation()?.structure_nom ?? null);

  /** Retour au choix : l'activation d'une autre affectation se fait sans mot de passe (§2.6). */
  changerEspace(): void {
    void this.router.navigate(['/espaces']);
  }

  async deconnecter(): Promise<void> {
    await this.acces.deconnecter();
    void this.router.navigate(['/connexion']);
  }
}
