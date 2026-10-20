import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { DemandeDetail, StatutDemande, intercepteurAcces } from 'commun';
import { FenetreService } from 'commun/fenetre';
import { of } from 'rxjs';
import { IssueFenetre } from '../fenetres';
import { DetailPage, MESSAGE_PLAFOND_ATTEINT } from './detail';
import { attendre, demandeDetail } from './donnees-test';

const ID = '0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55';
const DETAIL = `/api/assurance-munaseb/remboursements/${ID}`;

describe('DetailPage', () => {
  let fixture: ComponentFixture<DetailPage>;
  let controleur: HttpTestingController;
  let issue: IssueFenetre<unknown> | undefined;
  let ouvrir: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    issue = undefined;
    ouvrir = vi.fn(() => ({ closed: of(issue) }));
    TestBed.configureTestingModule({
      imports: [DetailPage],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
        { provide: FenetreService, useValue: { ouvrir } },
      ],
    });
    fixture = TestBed.createComponent(DetailPage);
    fixture.componentRef.setInput('id', ID);
    controleur = TestBed.inject(HttpTestingController);
  });

  afterEach(() => controleur.verify());

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function stabiliser(): Promise<void> {
    await attendre();
    await fixture.whenStable();
  }

  async function charger(demande: DemandeDetail): Promise<void> {
    await fixture.whenStable();
    controleur.expectOne(DETAIL).flush(demande);
    await stabiliser();
  }

  function boutons(): string[] {
    return [...element().querySelectorAll('.actions button')].map((b) => b.textContent?.trim() ?? '');
  }

  function bouton(libelle: string): HTMLButtonElement {
    const trouve = [...element().querySelectorAll<HTMLButtonElement>('.actions button')].find(
      (b) => b.textContent?.trim() === libelle,
    );
    if (trouve === undefined) {
      throw new Error(`bouton ${libelle} absent`);
    }
    return trouve;
  }

  const ATTENDUS: [StatutDemande, string[]][] = [
    ['en_attente', ['Prendre en charge', 'Rejeter']],
    ['en_cours', ['Valider', 'Rejeter']],
    ['valide', ['Marquer comme payée']],
    ['rejete', []],
    ['paye', []],
  ];

  for (const [statut, attendus] of ATTENDUS) {
    it(`statut ${statut} : boutons ${attendus.join(', ') || 'aucun'}`, async () => {
      await charger(demandeDetail({ statut, montant_rembourse_fcfa: statut === 'valide' || statut === 'paye' ? 10000 : null }));
      expect(boutons()).toEqual(attendus);
    });
  }

  it('affiche le titre, la mention du solde et l’historique « par le partenaire »', async () => {
    await charger(demandeDetail());
    expect(element().querySelector('h1')?.textContent).toContain('Pharmacie · OUEDRAOGO Awa');
    expect(element().textContent).toContain('Solde indicatif : il est recalculé au moment de la validation.');
    expect(element().querySelector('.historique')?.textContent).toContain('par le partenaire');
  });

  it('motif du rejet visible quand la demande est rejetée', async () => {
    await charger(demandeDetail({ statut: 'rejete', motif_rejet: 'Pièce manquante' }));
    expect(element().textContent).toContain('Pièce manquante');
  });

  it('double envoi impossible : deux clics, une seule requête', async () => {
    await charger(demandeDetail());
    const prendre = bouton('Prendre en charge');
    prendre.click();
    prendre.click();
    await fixture.whenStable();
    expect(prendre.disabled).toBe(true);
    const requetes = controleur.match(`${DETAIL}/prendre-en-charge`);
    expect(requetes.length).toBe(1);
    requetes[0].flush(null, { status: 204, statusText: 'No Content' });
    await stabiliser();
    controleur.expectOne(DETAIL).flush(demandeDetail({ statut: 'en_cours' }));
    await stabiliser();
    expect(boutons()).toEqual(['Valider', 'Rejeter']);
    expect(element().textContent).toContain('La demande est prise en charge.');
  });

  it('validation à 0 FCFA : encadré d’information, pas une erreur', async () => {
    await charger(demandeDetail({ statut: 'en_cours' }));
    issue = { ok: true, resultat: { montant_rembourse_fcfa: 0 } };
    bouton('Valider').click();
    await stabiliser();
    expect(ouvrir).toHaveBeenCalledTimes(1);
    controleur.expectOne(DETAIL).flush(demandeDetail({ statut: 'valide', montant_rembourse_fcfa: 0 }));
    await stabiliser();
    const info = element().querySelector('.message--info');
    expect(info?.textContent).toContain(MESSAGE_PLAFOND_ATTEINT);
    expect(element().querySelector('.message--erreur')).toBeNull();
    expect(boutons()).toEqual(['Marquer comme payée']);
  });

  it('409 transition_invalide : message avec le statut actuel, puis rechargement', async () => {
    await charger(demandeDetail({ statut: 'valide', montant_rembourse_fcfa: 10000 }));
    bouton('Marquer comme payée').click();
    await fixture.whenStable();
    controleur
      .expectOne(`${DETAIL}/payer`)
      .flush(
        { erreur: 'x', code: 'transition_invalide', statut_actuel: 'paye' },
        { status: 409, statusText: 'Conflict' },
      );
    await stabiliser();
    controleur.expectOne(DETAIL).flush(demandeDetail({ statut: 'paye', montant_rembourse_fcfa: 10000 }));
    await stabiliser();
    expect(element().textContent).toContain('Cette demande a changé entre-temps. Elle est maintenant : Payée.');
    expect(boutons()).toEqual([]);
  });

  it('demande_inconnue : message et lien vers la liste', async () => {
    await fixture.whenStable();
    controleur
      .expectOne(DETAIL)
      .flush({ erreur: 'x', code: 'demande_inconnue' }, { status: 404, statusText: 'Not Found' });
    await stabiliser();
    expect(element().textContent).toContain("Cette demande n'existe pas.");
    expect(element().querySelector('a.lien-retour')?.getAttribute('href')).toBe('/demandes');
  });
});
