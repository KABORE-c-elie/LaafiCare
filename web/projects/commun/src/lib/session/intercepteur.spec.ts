import { HttpClient, HttpErrorResponse, provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, TestRequest, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';
import { Affectation } from '../api/types';
import { AccesService } from './acces';
import { GESTION_SESSION, GestionSession } from './gestion-session';
import { intercepteurAcces } from './intercepteur';
import { SessionService } from './session';

const AFFECTATION: Affectation = {
  id: 'aff-1',
  role: 'agent_assurance_munaseb',
  structure_id: 'st-1',
  structure_nom: 'MUNASEB',
  structure_type: 'munaseb',
  structure_statut: 'validee',
  licence_expire_le: null,
  exercable: true,
};

const JETON_INVALIDE = { erreur: 'Jeton invalide.', code: 'jeton_invalide' };

/** Laisse les promesses en attente se résoudre. */
function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('intercepteurAcces', () => {
  let http: HttpClient;
  let controleur: HttpTestingController;
  let session: SessionService;
  let acces: AccesService;
  let gestion: { sessionTerminee: ReturnType<typeof vi.fn<() => void>>; accesRetire: ReturnType<typeof vi.fn<() => void>> };

  beforeEach(() => {
    gestion = { sessionTerminee: vi.fn<() => void>(), accesRetire: vi.fn<() => void>() };
    TestBed.configureTestingModule({
      providers: [
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
        { provide: GESTION_SESSION, useValue: gestion satisfies GestionSession },
      ],
    });
    http = TestBed.inject(HttpClient);
    controleur = TestBed.inject(HttpTestingController);
    session = TestBed.inject(SessionService);
    acces = TestBed.inject(AccesService);
    session.ouvrirEnExercice(
      { jeton_acces: 'acces-1', expire_dans: 900, jeton_renouvellement: 'renouv-1' },
      AFFECTATION,
    );
  });

  afterEach(() => {
    controleur.verify();
  });

  function renouvellements(): TestRequest[] {
    return controleur.match('/api/sessions/renouveler');
  }

  function repondreJetonInvalide(requete: TestRequest): void {
    requete.flush(JETON_INVALIDE, { status: 401, statusText: 'Unauthorized' });
  }

  it('ajoute Authorization: Bearer avec le jeton d’accès', async () => {
    const resultat = firstValueFrom(http.get('/api/x'));
    const requete = controleur.expectOne('/api/x');
    expect(requete.request.headers.get('Authorization')).toBe('Bearer acces-1');
    requete.flush({ ok: true });
    await expect(resultat).resolves.toEqual({ ok: true });
  });

  it('un seul renouvellement pour deux requêtes refusées, chacune rejouée une fois avec le nouveau jeton', async () => {
    const r1 = firstValueFrom(http.get('/api/a'));
    const r2 = firstValueFrom(http.get('/api/b'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/b'));

    const demandes = renouvellements();
    expect(demandes.length).toBe(1);
    expect(demandes[0].request.body).toEqual({ jeton_renouvellement: 'renouv-1' });
    expect(demandes[0].request.headers.has('Authorization')).toBe(false);
    demandes[0].flush({ jeton_acces: 'acces-2', expire_dans: 900, jeton_renouvellement: 'renouv-2' });
    await attendre();

    const rejouees = [controleur.expectOne('/api/a'), controleur.expectOne('/api/b')];
    for (const rejouee of rejouees) {
      expect(rejouee.request.headers.get('Authorization')).toBe('Bearer acces-2');
      rejouee.flush({ ok: true });
    }
    await expect(r1).resolves.toEqual({ ok: true });
    await expect(r2).resolves.toEqual({ ok: true });
    expect(session.jetonRenouvellement()).toBe('renouv-2');
    expect(session.etat()).toBe('en_exercice');
  });

  it('n’envoie plus jamais l’ancien jeton de renouvellement après un renouvellement réussi', async () => {
    const r1 = firstValueFrom(http.get('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    renouvellements()[0].flush({ jeton_acces: 'acces-2', expire_dans: 900, jeton_renouvellement: 'renouv-2' });
    await attendre();
    controleur.expectOne('/api/a').flush({});
    await r1;

    const r2 = firstValueFrom(http.get('/api/c'));
    repondreJetonInvalide(controleur.expectOne('/api/c'));
    const second = renouvellements();
    expect(second[0].request.body).toEqual({ jeton_renouvellement: 'renouv-2' });
    second[0].flush({ jeton_acces: 'acces-3', expire_dans: 900, jeton_renouvellement: 'renouv-3' });
    await attendre();
    controleur.expectOne('/api/c').flush({});
    await r2;
  });

  it('renouvellement refusé : session effacée, retour à la connexion, erreur transmise', async () => {
    const resultat = firstValueFrom(http.get('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    repondreJetonInvalide(renouvellements()[0]);
    await expect(resultat).rejects.toBeInstanceOf(HttpErrorResponse);
    expect(gestion.sessionTerminee).toHaveBeenCalledTimes(1);
    expect(session.etat()).toBe('deconnecte');
    expect(session.jetonRenouvellement()).toBeNull();
  });

  it('une requête rejouée n’est jamais rejouée une seconde fois', async () => {
    const resultat = firstValueFrom(http.get('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    renouvellements()[0].flush({ jeton_acces: 'acces-2', expire_dans: 900, jeton_renouvellement: 'renouv-2' });
    await attendre();
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    await expect(resultat).rejects.toBeInstanceOf(HttpErrorResponse);
    expect(renouvellements().length).toBe(0);
    expect(gestion.sessionTerminee).toHaveBeenCalledTimes(1);
  });

  it('sans réponse au renouvellement : relance une fois avec le même jeton', async () => {
    const resultat = firstValueFrom(http.get('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    renouvellements()[0].error(new ProgressEvent('error'));
    await attendre();
    const relance = renouvellements();
    expect(relance.length).toBe(1);
    expect(relance[0].request.body).toEqual({ jeton_renouvellement: 'renouv-1' });
    relance[0].flush({ jeton_acces: 'acces-2', expire_dans: 900, jeton_renouvellement: 'renouv-2' });
    await attendre();
    controleur.expectOne('/api/a').flush({ ok: true });
    await expect(resultat).resolves.toEqual({ ok: true });
  });

  it('sans réponse deux fois : pas de troisième essai, retour à la connexion', async () => {
    const resultat = firstValueFrom(http.get('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    renouvellements()[0].error(new ProgressEvent('error'));
    await attendre();
    renouvellements()[0].error(new ProgressEvent('error'));
    await expect(resultat).rejects.toBeInstanceOf(HttpErrorResponse);
    expect(renouvellements().length).toBe(0);
    expect(gestion.sessionTerminee).toHaveBeenCalledTimes(1);
  });

  it('pas de relance au-delà des 30 s de docs/api.md §1.7', async () => {
    let maintenant = 0;
    acces.horloge = () => maintenant;
    const resultat = firstValueFrom(http.get('/api/a'));
    repondreJetonInvalide(controleur.expectOne('/api/a'));
    maintenant = 30_000;
    renouvellements()[0].error(new ProgressEvent('error'));
    await expect(resultat).rejects.toBeInstanceOf(HttpErrorResponse);
    expect(renouvellements().length).toBe(0);
    expect(gestion.sessionTerminee).toHaveBeenCalledTimes(1);
  });

  it('403 acces_refuse : relit les affectations, sans renouvellement', async () => {
    const resultat = firstValueFrom(http.get('/api/a'));
    controleur
      .expectOne('/api/a')
      .flush({ erreur: 'Accès refusé.', code: 'acces_refuse' }, { status: 403, statusText: 'Forbidden' });
    await expect(resultat).rejects.toBeInstanceOf(HttpErrorResponse);
    expect(gestion.accesRetire).toHaveBeenCalledTimes(1);
    expect(renouvellements().length).toBe(0);
  });

  it('un 401 d’un autre code ne déclenche pas de renouvellement', async () => {
    const resultat = firstValueFrom(http.get('/api/a'));
    controleur
      .expectOne('/api/a')
      .flush({ erreur: 'x', code: 'identifiants_invalides' }, { status: 401, statusText: 'Unauthorized' });
    await expect(resultat).rejects.toBeInstanceOf(HttpErrorResponse);
    expect(renouvellements().length).toBe(0);
  });
});

describe('AccesService.deconnecter', () => {
  it('efface la mémoire même si la requête échoue', async () => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(withInterceptors([intercepteurAcces])), provideHttpClientTesting()],
    });
    const session = TestBed.inject(SessionService);
    const acces = TestBed.inject(AccesService);
    const controleur = TestBed.inject(HttpTestingController);
    session.ouvrirSansAffectation({ jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' });

    const fin = acces.deconnecter();
    const requete = controleur.expectOne('/api/sessions/deconnexion');
    expect(requete.request.body).toEqual({ jeton_renouvellement: 'r' });
    requete.error(new ProgressEvent('error'));
    await fin;
    expect(session.etat()).toBe('deconnecte');
    expect(session.jetonRenouvellement()).toBeNull();
    controleur.verify();
  });
});
