import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { intercepteurAcces } from 'commun';
import { DemandesPage } from './demandes';
import { attendre, demandeResume } from './donnees-test';

const LISTE = '/api/assurance-munaseb/remboursements';

describe('DemandesPage', () => {
  let fixture: ComponentFixture<DemandesPage>;
  let controleur: HttpTestingController;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [DemandesPage],
      providers: [provideRouter([]), provideHttpClient(withInterceptors([intercepteurAcces])), provideHttpClientTesting()],
    });
    fixture = TestBed.createComponent(DemandesPage);
    controleur = TestBed.inject(HttpTestingController);
    await fixture.whenStable();
  });

  afterEach(() => controleur.verify());

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function repondre(attendu: Record<string, string>, corps: object): Promise<void> {
    const requete = controleur.expectOne((r) => r.url === LISTE);
    for (const [cle, valeur] of Object.entries(attendu)) {
      expect(requete.request.params.get(cle)).toBe(valeur);
    }
    requete.flush(corps);
    await attendre();
    await fixture.whenStable();
  }

  function boutonSuivantes(): HTMLButtonElement | undefined {
    return [...element().querySelectorAll('button')].find((b) => b.textContent?.includes('demandes suivantes'));
  }

  it('pagination : limite=20, apres = suivant, bouton caché quand suivant = null', async () => {
    await repondre({ limite: '20' }, { demandes: [demandeResume()], suivant: 'curseur-1' });
    expect(boutonSuivantes()).toBeDefined();

    boutonSuivantes()?.click();
    await repondre({ limite: '20', apres: 'curseur-1' }, { demandes: [demandeResume({ id: 'd2' })], suivant: null });
    expect(element().querySelectorAll('tbody tr').length).toBe(2);
    expect(boutonSuivantes()).toBeUndefined();
  });

  it('les pilules filtrent par statut et repartent du début', async () => {
    await repondre({}, { demandes: [demandeResume()], suivant: null });
    const pilule = [...element().querySelectorAll<HTMLButtonElement>('.pilule')].find(
      (b) => b.textContent?.trim() === 'En cours',
    );
    pilule?.click();
    await repondre({ statut: 'en_cours', limite: '20' }, { demandes: [], suivant: null });
    expect(controleur.match(() => true).length).toBe(0);
    expect(pilule?.getAttribute('aria-pressed')).toBe('true');
    expect(element().textContent).toContain('Aucune demande avec ce statut.');
  });

  it('nom absent, remboursé null, montants en FCFA', async () => {
    await repondre({}, { demandes: [demandeResume({ patient_nom: null, patient_prenom: null })], suivant: null });
    const ligne = element().querySelector('tbody tr');
    expect(ligne?.textContent).toContain('Nom non renseigné');
    expect(ligne?.textContent).toContain('—');
    expect(ligne?.textContent?.replace(/\s/gu, ' ')).toContain('12 500 FCFA');
  });

  it('erreur réseau : message de connexion internet, pas « Aucune demande »', async () => {
    controleur.expectOne((r) => r.url === LISTE).error(new ProgressEvent('error'));
    await attendre();
    await fixture.whenStable();
    expect(element().textContent).toContain('Impossible de joindre le serveur.');
    expect(element().textContent).not.toContain('Aucune demande avec ce statut.');
  });
});
