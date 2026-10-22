import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { Router } from '@angular/router';
import { AccesService, EnteteComponent } from 'commun';

/** Mise en page des écrans connectés : en-tête « Administration LaafiCare », puis le contenu. */
@Component({
  selector: 'lc-cadre',
  imports: [EnteteComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-entete
      logo="/logo-laaficare.svg"
      titre="Administration LaafiCare"
      [afficherChangerEspace]="false"
      (deconnecter)="deconnecter()"
    />
    <main class="page" id="contenu">
      <ng-content />
    </main>
  `,
})
export class CadreComponent {
  private readonly acces = inject(AccesService);
  private readonly router = inject(Router);

  async deconnecter(): Promise<void> {
    await this.acces.deconnecter();
    void this.router.navigate(['/connexion']);
  }
}
