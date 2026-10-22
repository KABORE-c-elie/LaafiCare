import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { Affectation, intercepteurAcces } from 'commun';
import { attendre } from './donnees-test';
import { EspacesPage } from './espaces';

function affectation(modifs: Partial<Affectation>): Affectation {
  return {
    id: 'aff',
    role: 'agent_assurance_munaseb',
    structure_id: 'st-1',
    structure_nom: 'MUNASEB',
    structure_type: 'munaseb',
    structure_statut: 'validee',
    licence_expire_le: null,
    exercable: true,
    ...modifs,
  };
}

const PAIRE = { jeton_acces: 'a2', expire_dans: 900, jeton_renouvellement: 'r2' };

describe('EspacesPage', () => {
  let fixture: ComponentFixture<EspacesPage>;
  let controleur: HttpTestingController;
  let router: Router;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [EspacesPage],
      providers: [provideRouter([]), provideHttpClient(withInterceptors([intercepteurAcces])), provideHttpClientTesting()],
    });
    fixture = TestBed.createComponent(EspacesPage);
    controleur = TestBed.inject(HttpTestingController);
    router = TestBed.inject(Router);
    vi.spyOn(router, 'navigate').mockResolvedValue(true);
    await fixture.whenStable();
  });

  afterEach(() => controleur.verify());

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function lister(affectations: Affectation[]): Promise<void> {
    controleur.expectOne('/api/professionnels/affectations').flush({ affectations });
    await attendre();
    await fixture.whenStable();
  }

  it('montre les rôles agent et responsable de la MUNASEB, rien d’autre', async () => {
    await lister([
      affectation({ id: 'a1' }),
      affectation({ id: 'a2', role: 'responsable' }),
      affectation({ id: 'a3', role: 'medecin', structure_type: 'clinique', structure_nom: 'Clinique' }),
      affectation({ id: 'a4', role: 'medecin' }),
    ]);
    const lignes = [...element().querySelectorAll('.espace')].map((l) => l.textContent ?? '');
    expect(lignes.length).toBe(2);
    expect(lignes[0]).toContain('Agent MUNASEB');
    expect(lignes[1]).toContain('Responsable');
  });

  for (const [role, page] of [
    ['agent_assurance_munaseb', '/demandes'],
    ['responsable', '/invitations'],
  ] as const) {
    it(`${role} : activation, puis ${page}`, async () => {
      await lister([affectation({ id: 'a1', role })]);
      element().querySelector<HTMLButtonElement>('.espace button')?.click();
      controleur.expectOne('/api/professionnels/affectations/a1/activer').flush(PAIRE);
      await attendre();
      expect(router.navigate).toHaveBeenCalledWith([page]);
    });
  }
});
