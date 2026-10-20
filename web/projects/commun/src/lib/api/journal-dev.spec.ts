import { HttpClient, HttpHeaders, HttpParams, provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { firstValueFrom } from 'rxjs';
import { intercepteurJournalDev } from './journal-dev';

// Valeurs sensibles fictives : aucune ne doit apparaître dans le journal.
const SENSIBLES = [
  'Exemple#2026', // mot de passe
  '123456', // code TOTP
  'ABCDE-FGHIJ', // code de secours
  'jeton-acces-secret', // jeton d'accès
  'jeton-renouvellement-secret',
  'OUEDRAOGO', // nom d'adhérent
  'MUN-0001', // numéro de carte
  'en_attente', // paramètre de requête
  'message du serveur',
];

describe('intercepteurJournalDev', () => {
  let http: HttpClient;
  let controleur: HttpTestingController;
  let avertir: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [provideHttpClient(withInterceptors([intercepteurJournalDev])), provideHttpClientTesting()],
    });
    http = TestBed.inject(HttpClient);
    controleur = TestBed.inject(HttpTestingController);
    avertir = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
  });

  afterEach(() => {
    controleur.verify();
    avertir.mockRestore();
  });

  function journal(): string {
    return avertir.mock.calls.map((appel: unknown[]) => appel.map(String).join(' ')).join('\n');
  }

  it('écrit le code HTTP, le champ code, la méthode et le chemin, rien d’autre', async () => {
    const resultat = firstValueFrom(
      http.post(
        '/api/professionnels/connexion/second-facteur',
        { email: 'agent@example.org', mot_de_passe: 'Exemple#2026', code: '123456', code_secours: 'ABCDE-FGHIJ' },
        { headers: new HttpHeaders({ Authorization: 'Bearer jeton-acces-secret' }) },
      ),
    );
    controleur
      .expectOne('/api/professionnels/connexion/second-facteur')
      .flush(
        { erreur: 'message du serveur', code: 'code_invalide', jeton_renouvellement: 'jeton-renouvellement-secret' },
        { status: 401, statusText: 'Unauthorized' },
      );
    await expect(resultat).rejects.toBeDefined();

    expect(avertir).toHaveBeenCalledTimes(1);
    expect(journal()).toBe('[API] 401 code_invalide POST /api/professionnels/connexion/second-facteur');
    for (const valeur of SENSIBLES) {
      expect(journal()).not.toContain(valeur);
    }
    expect(journal()).not.toContain('agent@example.org');
  });

  it('retire les paramètres de requête du chemin', async () => {
    const resultat = firstValueFrom(
      http.get('/api/assurance-munaseb/remboursements', { params: new HttpParams().set('statut', 'en_attente') }),
    );
    controleur
      .expectOne((r) => r.url === '/api/assurance-munaseb/remboursements')
      .flush(
        { erreur: 'x', code: 'parametre_invalide', demandes: [{ patient_nom: 'OUEDRAOGO', numero_carte: 'MUN-0001' }] },
        { status: 400, statusText: 'Bad Request' },
      );
    await expect(resultat).rejects.toBeDefined();
    expect(journal()).toBe('[API] 400 parametre_invalide GET /api/assurance-munaseb/remboursements');
    for (const valeur of SENSIBLES) {
      expect(journal()).not.toContain(valeur);
    }
  });

  it('erreur réseau : le dit, sans détail', async () => {
    const resultat = firstValueFrom(http.get('/api/professionnels/affectations'));
    controleur.expectOne('/api/professionnels/affectations').error(new ProgressEvent('error'));
    await expect(resultat).rejects.toBeDefined();
    expect(journal()).toBe('[API] aucune réponse (réseau ou délai dépassé) GET /api/professionnels/affectations');
  });

  it('réponse sans champ code (ex. proxy) : statut seulement', async () => {
    const resultat = firstValueFrom(http.get('/api/x'));
    controleur.expectOne('/api/x').flush('<html>Bad Gateway</html>', { status: 502, statusText: 'Bad Gateway' });
    await expect(resultat).rejects.toBeDefined();
    expect(journal()).toBe('[API] 502 (réponse sans champ code) GET /api/x');
  });

  it('n’écrit rien quand la requête réussit', async () => {
    const resultat = firstValueFrom(http.get('/api/x'));
    controleur.expectOne('/api/x').flush({ patient_nom: 'OUEDRAOGO' });
    await resultat;
    expect(avertir).not.toHaveBeenCalled();
  });
});
