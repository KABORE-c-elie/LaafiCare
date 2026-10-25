import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { intercepteurAcces } from 'commun';
import { STRUCTURES_PAR_PAGE } from '../structures';
import { attendre, resumeStructure } from './donnees-test';
import { StructuresPage } from './structures';

const BASE = '/api/administrateurs/structures';

describe('StructuresPage', () => {
  let fixture: ComponentFixture<StructuresPage>;
  let controleur: HttpTestingController;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [StructuresPage],
      providers: [provideRouter([]), provideHttpClient(withInterceptors([intercepteurAcces])), provideHttpClientTesting()],
    });
    fixture = TestBed.createComponent(StructuresPage);
    controleur = TestBed.inject(HttpTestingController);
    await fixture.whenStable();
  });

  afterEach(() => controleur.verify());

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function stabiliser(): Promise<void> {
    await attendre();
    await fixture.whenStable();
  }

  function requete() {
    return controleur.expectOne((r) => r.method === 'GET' && r.url === BASE);
  }

  it('« En attente » par défaut, 20 par page ; ligne avec type, responsable et statut en texte', async () => {
    const premiere = requete();
    expect(premiere.request.params.get('statut')).toBe('en_attente');
    expect(premiere.request.params.get('limite')).toBe(String(STRUCTURES_PAR_PAGE));
    premiere.flush({ structures: [resumeStructure()], suivant: null });
    await stabiliser();
    const ligne = element().querySelector('tbody tr')?.textContent ?? '';
    expect(ligne).toContain('MUNASEB');
    expect(ligne).toContain('OUEDRAOGO Awa');
    expect(ligne).toContain('En attente');
    expect(element().querySelector('a.lien-ouvrir')?.getAttribute('href')).toBe(`/structures/${resumeStructure().id}`);
  });

  it('« Toutes » : sans paramètre statut ; page suivante avec `apres`', async () => {
    requete().flush({ structures: [], suivant: null });
    await stabiliser();
    expect(element().textContent).toContain('Aucune demande avec ce statut.');
    [...element().querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Toutes')?.click();
    const toutes = requete();
    expect(toutes.request.params.has('statut')).toBe(false);
    toutes.flush({ structures: [resumeStructure()], suivant: 'curseur-1' });
    await stabiliser();
    [...element().querySelectorAll('button')].find((b) => b.textContent?.includes('suivantes'))?.click();
    const suite = requete();
    expect(suite.request.params.get('apres')).toBe('curseur-1');
    suite.flush({ structures: [resumeStructure({ id: 'autre', statut: 'validee' })], suivant: null });
    await stabiliser();
    expect(element().querySelectorAll('tbody tr').length).toBe(2);
    expect(element().querySelectorAll('tbody tr')[1].textContent).toContain('Validée');
  });
});
