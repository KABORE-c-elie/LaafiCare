import { HttpErrorResponse, provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { DetailStructure, intercepteurAcces, jourAuBurkina, lendemain } from 'commun';
import { DonneesMotif, FenetreMotifComponent, FenetreService, IssueFenetre } from 'commun/fenetre';
import { of } from 'rxjs';
import { DonneesValidationStructure, FenetreValidationStructureComponent } from '../fenetre-validation';
import { attendre, detailStructure } from './donnees-test';
import { MESSAGES_STRUCTURE, StructurePage } from './structure';

const ID = detailStructure().id;
const DETAIL = `/api/administrateurs/structures/${ID}`;

function erreur(status: number, code: string, extra: Record<string, string> = {}): HttpErrorResponse {
  return new HttpErrorResponse({ status, error: { erreur: 'texte du serveur', code, ...extra } });
}

describe('StructurePage', () => {
  let fixture: ComponentFixture<StructurePage>;
  let controleur: HttpTestingController;
  let issue: IssueFenetre<void> | undefined;
  let ouvrir: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    issue = undefined;
    ouvrir = vi.fn(() => ({ closed: of(issue) }));
    TestBed.configureTestingModule({
      imports: [StructurePage],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
        { provide: FenetreService, useValue: { ouvrir } },
      ],
    });
    fixture = TestBed.createComponent(StructurePage);
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

  async function charger(structure: DetailStructure): Promise<void> {
    await fixture.whenStable();
    controleur.expectOne(DETAIL).flush(structure);
    await stabiliser();
  }

  function boutons(): string[] {
    return [...element().querySelectorAll('.actions button')].map((b) => b.textContent?.trim() ?? '');
  }

  function cliquer(libelle: string): void {
    [...element().querySelectorAll<HTMLButtonElement>('.actions button')].find((b) => b.textContent?.trim() === libelle)?.click();
  }

  it('affiche la structure, le responsable, le statut ; Valider et Refuser seulement en attente', async () => {
    await charger(detailStructure());
    expect(element().querySelector('h1')?.textContent).toContain('MUNASEB');
    expect(element().textContent).toContain('+22670000001');
    expect(element().textContent).toContain('responsable@example.org');
    expect(element().textContent).toContain('12, B');
    expect(boutons()).toEqual(['Valider', 'Refuser']);
  });

  for (const statut of ['brouillon', 'validee', 'refusee', 'suspendue', 'abandonnee'] as const) {
    it(`statut ${statut} : aucune décision possible`, async () => {
      await charger(detailStructure({ statut }));
      expect(boutons()).toEqual([]);
    });
  }

  it('avertissements MUNASEB : la MUNASEB validée et les autres demandes, avec leurs liens', async () => {
    await charger(
      detailStructure({ munaseb_deja_validee: 'validee-1', autres_demandes_munaseb_en_attente: ['autre-1', 'autre-2'] }),
    );
    expect(element().textContent).toContain('Une MUNASEB est déjà validée ou suspendue : une seule est admise.');
    expect(element().textContent).toContain('2 autres demandes MUNASEB attendent une décision');
    const liens = [...element().querySelectorAll('.avertissement a')].map((a) => a.getAttribute('href'));
    expect(liens).toEqual(['/structures/validee-1', '/structures/autre-1', '/structures/autre-2']);
  });

  it('email du responsable non vérifié : avertissement, la validation reste proposée', async () => {
    await charger(detailStructure({ responsable_email_verifie: false }));
    expect(element().textContent).toContain(MESSAGES_STRUCTURE.emailNonVerifie);
    expect(boutons()).toEqual(['Valider', 'Refuser']);
  });

  it('historique des décisions, avec l’administrateur, la licence et le motif', async () => {
    await charger(
      detailStructure({
        statut: 'validee',
        licence_expire_le: '2027-10-21',
        decisions: [
          {
            decision: 'refusee',
            motif: 'Adresse incomplète',
            licence_expire_le: null,
            date_decision: '2026-10-20T10:00:00Z',
            administrateur_nom: 'KONE',
            administrateur_prenom: 'Issa',
          },
          {
            decision: 'validee',
            motif: null,
            licence_expire_le: '2027-10-21',
            date_decision: '2026-10-21T10:00:00Z',
            administrateur_nom: 'KONE',
            administrateur_prenom: 'Issa',
          },
        ],
      }),
    );
    const lignes = [...element().querySelectorAll('.historique li')].map((li) => li.textContent ?? '');
    expect(lignes[0]).toContain('Refusée');
    expect(lignes[0]).toContain('Adresse incomplète');
    expect(lignes[1]).toContain('Validée');
    expect(lignes[1]).toContain('par KONE Issa');
    expect(lignes[1]).toContain('21 octobre 2027');
  });

  it('valider : fenêtre avec le nom et demain (Burkina) ; succès, rechargement et message', async () => {
    await charger(detailStructure());
    issue = { ok: true, resultat: undefined };
    cliquer('Valider');
    await stabiliser();
    expect(ouvrir).toHaveBeenCalledTimes(1);
    expect(ouvrir.mock.calls[0][0]).toBe(FenetreValidationStructureComponent);
    const donnees = ouvrir.mock.calls[0][1] as DonneesValidationStructure;
    expect(donnees.nom).toBe('MUNASEB');
    expect(donnees.demain).toBe(lendemain(jourAuBurkina()));
    controleur.expectOne(DETAIL).flush(detailStructure({ statut: 'validee' }));
    await stabiliser();
    expect(element().querySelector('[role="status"]')?.textContent).toContain(MESSAGES_STRUCTURE.validee);
    expect(boutons()).toEqual([]);
  });

  it('valider : l’envoi part avec la licence choisie, ou {} sans fin', async () => {
    await charger(detailStructure());
    cliquer('Valider');
    await stabiliser();
    const donnees = ouvrir.mock.calls[0][1] as DonneesValidationStructure;
    void donnees.envoyer('2027-10-21');
    expect(controleur.expectOne(`${DETAIL}/valider`).request.body).toEqual({ licence_expire_le: '2027-10-21' });
    void donnees.envoyer(null);
    expect(controleur.expectOne(`${DETAIL}/valider`).request.body).toEqual({});
  });

  const REFUS: [number, string, string][] = [
    [403, 'decision_sur_sa_propre_structure', MESSAGES_STRUCTURE.propreStructure],
    [409, 'munaseb_deja_validee', MESSAGES_STRUCTURE.munasebDejaValidee],
    [409, 'email_responsable_non_verifie', MESSAGES_STRUCTURE.emailNonVerifie],
    [409, 'autorisation_deja_validee', MESSAGES_STRUCTURE.autorisationDejaValidee],
  ];

  for (const [status, code, texte] of REFUS) {
    it(`valider refusé ${status} ${code} : message clair, jamais le texte du serveur`, async () => {
      await charger(detailStructure());
      issue = { ok: false, erreur: erreur(status, code) };
      cliquer('Valider');
      await stabiliser();
      controleur.expectOne(DETAIL).flush(detailStructure());
      await stabiliser();
      expect(element().querySelector('[role="alert"]')?.textContent).toContain(texte);
      expect(element().textContent).not.toContain('texte du serveur');
    });
  }

  it('409 transition_invalide : message avec le statut actuel, puis rechargement', async () => {
    await charger(detailStructure());
    issue = { ok: false, erreur: erreur(409, 'transition_invalide', { statut_actuel: 'validee' }) };
    cliquer('Refuser');
    await stabiliser();
    controleur.expectOne(DETAIL).flush(detailStructure({ statut: 'validee' }));
    await stabiliser();
    expect(element().textContent).toContain('Cette demande a changé entre-temps. Elle est maintenant : validée.');
  });

  it('structure_inconnue : « Cette structure n’existe pas. » et lien vers la liste', async () => {
    await fixture.whenStable();
    controleur
      .expectOne(DETAIL)
      .flush({ erreur: 'x', code: 'structure_inconnue' }, { status: 404, statusText: 'Not Found' });
    await stabiliser();
    expect(element().textContent).toContain(MESSAGES_STRUCTURE.inconnue);
    expect(element().querySelector('a.lien-retour')?.getAttribute('href')).toBe('/structures');
  });

  it('refuser : fenêtre de motif commune, avec les textes du refus et 1 000 caractères', async () => {
    await charger(detailStructure());
    issue = { ok: true, resultat: undefined };
    cliquer('Refuser');
    await stabiliser();
    expect(ouvrir.mock.calls[0][0]).toBe(FenetreMotifComponent);
    const donnees = ouvrir.mock.calls[0][1] as DonneesMotif;
    expect(donnees.max).toBe(1000);
    expect(donnees.messageVide).toBe(MESSAGES_STRUCTURE.motifVide);
    expect(donnees.messageTropLong).toBe(MESSAGES_STRUCTURE.motifTropLong);
    void donnees.envoyer('Pièces manquantes');
    expect(controleur.expectOne(`${DETAIL}/refuser`).request.body).toEqual({ motif: 'Pièces manquantes' });
    controleur.expectOne(DETAIL).flush(detailStructure({ statut: 'refusee' }));
    await stabiliser();
    expect(element().textContent).toContain(MESSAGES_STRUCTURE.refusee);
  });

  it('double clic : une seule fenêtre', async () => {
    await charger(detailStructure());
    ouvrir.mockReturnValue({ closed: of(undefined) });
    cliquer('Valider');
    cliquer('Valider');
    await stabiliser();
    expect(ouvrir).toHaveBeenCalledTimes(1);
  });
});
