import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { Invitation, PageInvitations, intercepteurAcces, intercepteurJournalDev } from 'commun';
import { DonneesConfirmation, FenetreService, IssueFenetre } from 'commun/fenetre';
import { from, of } from 'rxjs';
import { INVITATIONS_PAR_PAGE } from '../invitations';
import { attendre } from './donnees-test';
import { InvitationsPage, MESSAGES_INVITATION } from './invitations';

const BASE = '/api/professionnels/invitations';
// Numéro fictif (CLAUDE.md §13), sous ses trois formes : saisie, normalisée, chiffres seuls.
const SAISI = '70 00 00 02';
const NORMALISE = '+22670000002';

function invitation(modifs: Partial<Invitation> = {}): Invitation {
  return {
    id: '0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a01',
    telephone: NORMALISE,
    role: 'agent_assurance_munaseb',
    statut: 'en_attente',
    date_creation: '2026-10-22T09:00:00Z',
    expire_le: '2026-10-29T09:00:00Z',
    date_reponse: null,
    accepte_par_nom: null,
    accepte_par_prenom: null,
    ...modifs,
  };
}

function page(invitations: Invitation[], suivant: string | null = null): PageInvitations {
  return { invitations, suivant };
}

describe('InvitationsPage', () => {
  let fixture: ComponentFixture<InvitationsPage>;
  let controleur: HttpTestingController;
  let ouvrir: ReturnType<typeof vi.fn>;
  /** Ce que fait la fenêtre de confirmation : envoyer (comme la vraie), ou être fermée sans rien faire. */
  let reponseFenetre: 'confirmer' | 'fermer';
  let consoles: ReturnType<typeof vi.spyOn>[];

  beforeEach(async () => {
    reponseFenetre = 'confirmer';
    ouvrir = vi.fn((_composant: unknown, donnees: DonneesConfirmation<unknown>) => ({
      closed:
        reponseFenetre === 'fermer'
          ? of(undefined)
          : from(
              donnees.envoyer().then(
                (resultat): IssueFenetre<unknown> => ({ ok: true, resultat }),
                (erreur: unknown): IssueFenetre<unknown> => ({ ok: false, erreur }),
              ),
            ),
    }));
    consoles = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) =>
      vi.spyOn(console, m).mockImplementation(() => undefined),
    );
    TestBed.configureTestingModule({
      imports: [InvitationsPage],
      providers: [
        provideRouter([]),
        // Journal de développement branché, comme avec `ng serve`.
        provideHttpClient(withInterceptors([intercepteurAcces, intercepteurJournalDev])),
        provideHttpClientTesting(),
        { provide: FenetreService, useValue: { ouvrir } },
      ],
    });
    fixture = TestBed.createComponent(InvitationsPage);
    controleur = TestBed.inject(HttpTestingController);
    await fixture.whenStable();
  });

  afterEach(() => {
    controleur.verify();
    consoles.forEach((espion) => espion.mockRestore());
  });

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function stabiliser(): Promise<void> {
    await attendre();
    await fixture.whenStable();
    await attendre();
    await fixture.whenStable();
  }

  async function chargerListe(contenu: PageInvitations = page([])): Promise<void> {
    controleur.expectOne((r) => r.method === 'GET' && r.url === BASE).flush(contenu);
    await stabiliser();
  }

  function saisir(valeur: string): void {
    const champ = element().querySelector<HTMLInputElement>('#telephone');
    if (champ === null) {
      throw new Error('champ absent');
    }
    champ.value = valeur;
    champ.dispatchEvent(new Event('input'));
  }

  function choisirResponsable(): void {
    const radio = element().querySelector<HTMLInputElement>('input[value="responsable"]');
    radio?.click();
  }

  async function soumettre(): Promise<void> {
    element().querySelector('form')?.dispatchEvent(new Event('submit'));
    await fixture.whenStable();
  }

  function journal(): string {
    return consoles.flatMap((e) => e.mock.calls.map((appel: unknown[]) => appel.map(String).join(' '))).join('\n');
  }

  it('liste : statut en texte, personne seulement après acceptation, « Annuler » seulement en attente', async () => {
    await chargerListe(
      page([
        invitation(),
        invitation({
          id: 'i2',
          statut: 'acceptee',
          accepte_par_nom: 'SAWADOGO',
          accepte_par_prenom: 'Issa',
          date_reponse: '2026-10-22T10:00:00Z',
        }),
        invitation({ id: 'i3', statut: 'expiree', role: 'responsable' }),
      ]),
    );
    const lignes = [...element().querySelectorAll('tbody tr')].map((l) => l.textContent ?? '');
    expect(lignes[0]).toContain('En attente');
    expect(lignes[0]).toContain('Agent MUNASEB');
    expect(lignes[0]).toContain('Annuler');
    expect(lignes[1]).toContain('Acceptée');
    expect(lignes[1]).toContain('SAWADOGO Issa');
    expect(lignes[1]).not.toContain('Annuler');
    expect(lignes[2]).toContain('Expirée');
    expect(lignes[2]).toContain('Responsable');
    expect(lignes[2]).not.toContain('Annuler');
  });

  it('pagination : 20 par page, puis `avant` = `suivant`', async () => {
    const premiere = controleur.expectOne((r) => r.method === 'GET' && r.url === BASE);
    expect(premiere.request.params.get('limite')).toBe(String(INVITATIONS_PAR_PAGE));
    expect(premiere.request.params.has('avant')).toBe(false);
    premiere.flush(page([invitation()], 'curseur-1'));
    await stabiliser();
    [...element().querySelectorAll('button')].find((b) => b.textContent?.includes('suivantes'))?.click();
    const suite = controleur.expectOne((r) => r.method === 'GET' && r.url === BASE);
    expect(suite.request.params.get('avant')).toBe('curseur-1');
    suite.flush(page([invitation({ id: 'i2' })]));
    await stabiliser();
    expect(element().querySelectorAll('tbody tr').length).toBe(2);
  });

  it('numéro vide : rien n’est envoyé', async () => {
    await chargerListe();
    await soumettre();
    controleur.expectNone(BASE);
    expect(element().querySelector('#telephone-erreur')?.textContent).toContain(MESSAGES_INVITATION.telephoneVide);
  });

  it('agent : numéro envoyé tel que saisi, sans fenêtre ; « Invitation envoyée. » et date de fin ; liste rechargée', async () => {
    await chargerListe();
    expect(element().querySelector('#telephone-aide')?.textContent).toContain('Un numéro étranger commence par +.');
    saisir(SAISI);
    await soumettre();
    const requete = controleur.expectOne((r) => r.method === 'POST' && r.url === BASE);
    expect(requete.request.body).toEqual({ telephone: SAISI, role: 'agent_assurance_munaseb' });
    expect(ouvrir).not.toHaveBeenCalled();
    requete.flush({ invitation_id: 'i1', expire_le: '2026-10-29T09:00:00Z' }, { status: 201, statusText: 'Created' });
    await stabiliser();
    await chargerListe(page([invitation()]));
    expect(element().querySelector('[role="status"]')?.textContent).toContain(
      "Invitation envoyée. Valable jusqu'au 29/10/2026.",
    );
    expect(element().querySelector<HTMLInputElement>('#telephone')?.value).toBe('');
  });

  it('double envoi impossible : deux soumissions, une seule requête', async () => {
    await chargerListe();
    saisir(SAISI);
    await soumettre();
    await soumettre();
    const requetes = controleur.match((r) => r.method === 'POST' && r.url === BASE);
    expect(requetes.length).toBe(1);
    requetes[0].flush({ invitation_id: 'i1', expire_le: '2026-10-29T09:00:00Z' }, { status: 201, statusText: 'Created' });
    await stabiliser();
    await chargerListe();
  });

  it('responsable : confirmation demandée avant l’envoi', async () => {
    await chargerListe();
    saisir(SAISI);
    choisirResponsable();
    await soumettre();
    expect(ouvrir).toHaveBeenCalledTimes(1);
    const donnees = ouvrir.mock.calls[0][1] as DonneesConfirmation<unknown>;
    expect(donnees.action).toBe('Inviter comme responsable');
    const requete = controleur.expectOne((r) => r.method === 'POST' && r.url === BASE);
    expect(requete.request.body).toEqual({ telephone: SAISI, role: 'responsable' });
    requete.flush({ invitation_id: 'i1', expire_le: '2026-10-29T09:00:00Z' }, { status: 201, statusText: 'Created' });
    await stabiliser();
    await chargerListe();
    expect(element().textContent).toContain('Invitation envoyée.');
  });

  it('responsable : fenêtre fermée sans confirmer, rien n’est envoyé', async () => {
    await chargerListe();
    reponseFenetre = 'fermer';
    saisir(SAISI);
    choisirResponsable();
    await soumettre();
    await stabiliser();
    expect(ouvrir).toHaveBeenCalledTimes(1);
    controleur.expectNone((r) => r.method === 'POST');
  });

  const REFUS: [number, string, string][] = [
    [409, 'invitation_deja_en_attente', MESSAGES_INVITATION.dejaEnAttente],
    [409, 'trop_d_invitations_en_attente', MESSAGES_INVITATION.tropEnAttente],
    [409, 'structure_non_validee', MESSAGES_INVITATION.structureNonValidee],
    [422, 'role_non_permis', MESSAGES_INVITATION.roleNonPermis],
    [422, 'auto_invitation', MESSAGES_INVITATION.autoInvitation],
    [429, 'trop_de_demandes', MESSAGES_INVITATION.tropDeDemandes],
  ];

  for (const [status, code, texte] of REFUS) {
    it(`refus ${status} ${code} : message clair, jamais le texte du serveur`, async () => {
      await chargerListe();
      saisir(SAISI);
      await soumettre();
      controleur
        .expectOne((r) => r.method === 'POST' && r.url === BASE)
        .flush({ erreur: 'texte du serveur', code }, { status, statusText: 'x' });
      await stabiliser();
      expect(element().querySelector('form [role="alert"]')?.textContent).toContain(texte);
      expect(element().textContent).not.toContain('texte du serveur');
    });
  }

  it('422 telephone_invalide : message sous le champ', async () => {
    await chargerListe();
    saisir('123');
    await soumettre();
    controleur
      .expectOne((r) => r.method === 'POST' && r.url === BASE)
      .flush({ erreur: 'x', code: 'telephone_invalide' }, { status: 422, statusText: 'x' });
    await stabiliser();
    expect(element().querySelector('#telephone-erreur')?.textContent).toContain(MESSAGES_INVITATION.telephoneInvalide);
    expect(element().querySelector('#telephone')?.getAttribute('aria-describedby')).toBe(
      'telephone-aide telephone-erreur',
    );
  });

  it('403 acces_refuse : aucun message ici (l’intercepteur ramène au choix de l’espace)', async () => {
    await chargerListe();
    saisir(SAISI);
    await soumettre();
    controleur
      .expectOne((r) => r.method === 'POST' && r.url === BASE)
      .flush({ erreur: 'x', code: 'acces_refuse' }, { status: 403, statusText: 'Forbidden' });
    await stabiliser();
    expect(element().querySelector('form [role="alert"]')).toBeNull();
  });

  it('annuler : confirmation, POST …/annuler, message et liste rechargée', async () => {
    await chargerListe(page([invitation()]));
    [...element().querySelectorAll('tbody button')].find((b) => b.textContent?.includes('Annuler'))?.dispatchEvent(
      new Event('click'),
    );
    await fixture.whenStable();
    const donnees = ouvrir.mock.calls[0][1] as DonneesConfirmation<unknown>;
    expect(donnees.action).toBe('Annuler l’invitation');
    controleur.expectOne(`${BASE}/${invitation().id}/annuler`).flush(null, { status: 204, statusText: 'No Content' });
    await stabiliser();
    await chargerListe(page([invitation({ statut: 'annulee' })]));
    expect(element().textContent).toContain(MESSAGES_INVITATION.annulee);
    expect(element().querySelector('tbody tr')?.textContent).toContain('Annulée');
  });

  const REFUS_ANNULATION: [string, Record<string, string>, string][] = [
    ['invitation_inconnue', {}, MESSAGES_INVITATION.inconnue],
    ['invitation_expiree', {}, MESSAGES_INVITATION.expiree],
    ['transition_invalide', { statut_actuel: 'acceptee' }, 'Cette invitation a changé entre-temps. Elle est maintenant : acceptée.'],
  ];

  for (const [code, extra, texte] of REFUS_ANNULATION) {
    it(`annulation refusée (${code}) : message, puis liste rechargée`, async () => {
      await chargerListe(page([invitation()]));
      [...element().querySelectorAll('tbody button')].find((b) => b.textContent?.includes('Annuler'))?.dispatchEvent(
        new Event('click'),
      );
      await fixture.whenStable();
      controleur
        .expectOne(`${BASE}/${invitation().id}/annuler`)
        .flush({ erreur: 'x', code, ...extra }, { status: code === 'invitation_inconnue' ? 404 : 409, statusText: 'x' });
      await stabiliser();
      await chargerListe(page([invitation({ statut: 'acceptee' })]));
      expect(element().querySelector('[role="alert"]')?.textContent).toContain(texte);
    });
  }

  it('le numéro n’apparaît jamais dans la console (journal de développement compris)', async () => {
    await chargerListe();
    saisir(SAISI);
    await soumettre();
    controleur
      .expectOne((r) => r.method === 'POST' && r.url === BASE)
      .flush({ erreur: `numéro ${NORMALISE}`, code: 'invitation_deja_en_attente' }, { status: 409, statusText: 'x' });
    await stabiliser();
    expect(journal()).toContain('[API] 409 invitation_deja_en_attente POST /api/professionnels/invitations');
    for (const forme of [SAISI, NORMALISE, '70000002']) {
      expect(journal()).not.toContain(forme);
    }
  });
});
