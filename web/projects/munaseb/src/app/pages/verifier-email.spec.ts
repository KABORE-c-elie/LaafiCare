import { BrowserPlatformLocation, PlatformLocation } from '@angular/common';
import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { MESSAGES, intercepteurAcces, intercepteurJournalDev } from 'commun';
import { VerifierEmailPage } from './verifier-email';

// Valeurs fictives (64 caractères hexadécimaux, docs/api.md §2.7).
const JETON = '0123456789abcdef'.repeat(4);
const EMAIL = 'agent@example.org';
const MOT_DE_PASSE = 'Exemple#2026';
const URL_VERIFIER = '/api/professionnels/email/verifier';
const URL_RENVOYER = '/api/professionnels/email/renvoyer';

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('VerifierEmailPage', () => {
  let fixture: ComponentFixture<VerifierEmailPage>;
  let controleur: HttpTestingController;
  let consoles: ReturnType<typeof vi.spyOn>[];

  async function ouvrir(adresse: string, etat: unknown = null): Promise<void> {
    window.history.replaceState(etat, '', adresse);
    TestBed.configureTestingModule({
      imports: [VerifierEmailPage],
      providers: [
        provideRouter([]),
        // Le journal de développement est branché, comme avec `ng serve`.
        provideHttpClient(withInterceptors([intercepteurAcces, intercepteurJournalDev])),
        provideHttpClientTesting(),
      ],
    });
    fixture = TestBed.createComponent(VerifierEmailPage);
    controleur = TestBed.inject(HttpTestingController);
    await fixture.whenStable();
  }

  beforeEach(() => {
    consoles = (['log', 'info', 'warn', 'error', 'debug'] as const).map((methode) =>
      vi.spyOn(console, methode).mockImplementation(() => undefined),
    );
  });

  afterEach(() => {
    controleur.verify();
    consoles.forEach((espion) => espion.mockRestore());
    vi.useRealTimers();
    window.history.replaceState(null, '', '/');
  });

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  function bouton(texte: string): HTMLButtonElement {
    const trouve = [...element().querySelectorAll('button')].find((b) => b.textContent?.includes(texte));
    if (trouve === undefined) {
      throw new Error(`bouton « ${texte} » absent`);
    }
    return trouve;
  }

  function saisir(id: string, valeur: string): void {
    const champ = element().querySelector<HTMLInputElement>(`#${id}`);
    if (champ === null) {
      throw new Error(`champ ${id} absent`);
    }
    champ.value = valeur;
    champ.dispatchEvent(new Event('input'));
  }

  async function rafraichir(): Promise<void> {
    await attendre();
    await fixture.whenStable();
  }

  function toutLeJournal(): string {
    return consoles.flatMap((espion) => espion.mock.calls.map((appel: unknown[]) => appel.map(String).join(' '))).join('\n');
  }

  async function arriverSurLienInvalide(): Promise<void> {
    await ouvrir(`/verifier-email#jeton=${JETON}`);
    bouton('Confirmer mon adresse').click();
    controleur
      .expectOne(URL_VERIFIER)
      .flush({ erreur: 'texte du serveur', code: 'lien_invalide' }, { status: 400, statusText: 'Bad Request' });
    await rafraichir();
  }

  async function demanderRenvoi(): Promise<void> {
    saisir('email', EMAIL);
    saisir('mot-de-passe', MOT_DE_PASSE);
    element().querySelector('form')?.dispatchEvent(new Event('submit'));
    await fixture.whenStable();
  }

  it('lit le jeton après #, efface aussitôt le fragment de l’adresse et garde l’état de l’historique', async () => {
    await ouvrir(`/verifier-email#jeton=${JETON}`, { navigationId: 3 });
    expect(window.location.hash).toBe('');
    expect(window.location.pathname).toBe('/verifier-email');
    expect(window.location.href).not.toContain(JETON);
    expect(window.history.state).toEqual({ navigationId: 3 });
    expect(element().textContent).not.toContain(JETON);
    // Ouvrir le lien ne vérifie rien (docs/api.md §2.7) : aucune requête avant le clic.
    controleur.expectNone(() => true);
    expect(bouton('Confirmer mon adresse').disabled).toBe(false);
  });

  it('envoie le jeton par POST dans le corps, jamais par GET ; 204 : email vérifié, lien vers la connexion', async () => {
    await ouvrir(`/verifier-email#jeton=${JETON}`);
    bouton('Confirmer mon adresse').click();
    await fixture.whenStable();
    expect(bouton('Confirmation…').disabled).toBe(true);

    controleur.expectNone({ method: 'GET', url: URL_VERIFIER });
    const requete = controleur.expectOne({ method: 'POST', url: URL_VERIFIER });
    expect(requete.request.body).toEqual({ jeton: JETON });
    expect(requete.request.urlWithParams).not.toContain(JETON);
    expect(requete.request.headers.has('Authorization')).toBe(false);
    requete.flush(null, { status: 204, statusText: 'No Content' });
    await rafraichir();

    expect(element().querySelector('[role="status"]')?.textContent).toContain(MESSAGES.emailVerifie);
    expect(element().querySelector('a[href="/connexion"]')?.textContent).toContain('Aller à la connexion');
    expect(document.activeElement?.textContent).toContain('Vérification de votre email');
  });

  it('400 lien_invalide : message, formulaire de renvoi, et le jeton est oublié', async () => {
    await arriverSurLienInvalide();
    expect(element().querySelector('[role="alert"]')?.textContent).toContain(MESSAGES.lienInvalide);
    expect(element().textContent).not.toContain('texte du serveur');
    expect(element().querySelector('#email')).not.toBeNull();
    expect(element().querySelector('#mot-de-passe')).not.toBeNull();
    // Plus de bouton de confirmation : le jeton n'est plus envoyé.
    expect([...element().querySelectorAll('button')].some((b) => b.textContent?.includes('Confirmer'))).toBe(false);
  });

  it('pas de jeton dans l’adresse : même message que lien_invalide, aucune requête', async () => {
    for (const adresse of ['/verifier-email', '/verifier-email#jeton=', '/verifier-email#autre=1']) {
      TestBed.resetTestingModule();
      await ouvrir(adresse);
      expect(element().querySelector('[role="alert"]')?.textContent).toContain(MESSAGES.lienInvalide);
      controleur.expectNone(() => true);
    }
  });

  it('erreur réseau : message, le jeton reste en mémoire et peut être renvoyé', async () => {
    await ouvrir(`/verifier-email#jeton=${JETON}`);
    bouton('Confirmer mon adresse').click();
    controleur.expectOne(URL_VERIFIER).error(new ProgressEvent('error'));
    await rafraichir();
    expect(element().textContent).toContain(MESSAGES.reseau);

    bouton('Confirmer mon adresse').click();
    controleur.expectOne(URL_VERIFIER).flush(null, { status: 204, statusText: 'No Content' });
    await rafraichir();
    expect(element().textContent).toContain(MESSAGES.emailVerifie);
  });

  it('renvoi réussi : corps email + mot de passe, message, mot de passe effacé du formulaire', async () => {
    await arriverSurLienInvalide();
    expect(element().textContent).toContain('Sans accent ni espace');
    expect(element().querySelector('#email')?.getAttribute('aria-describedby')).toBe('email-aide');
    await demanderRenvoi();
    const requete = controleur.expectOne({ method: 'POST', url: URL_RENVOYER });
    expect(requete.request.body).toEqual({ email: EMAIL, mot_de_passe: MOT_DE_PASSE });
    requete.flush(null, { status: 204, statusText: 'No Content' });
    await rafraichir();
    expect(element().querySelector('[role="status"]')?.textContent).toContain(MESSAGES.lienRenvoye);
    expect(element().querySelector<HTMLInputElement>('#mot-de-passe')?.value).toBe('');
  });

  it('renvoi refusé 401 : « Email ou mot de passe incorrect. »', async () => {
    await arriverSurLienInvalide();
    await demanderRenvoi();
    controleur
      .expectOne(URL_RENVOYER)
      .flush({ erreur: 'x', code: 'identifiants_invalides' }, { status: 401, statusText: 'Unauthorized' });
    await rafraichir();
    expect(element().querySelector('form [role="alert"]')?.textContent).toContain('Email ou mot de passe incorrect.');
  });

  it('renvoi refusé 429 : bouton désactivé pendant Retry-After, avec le décompte, puis réactivé', async () => {
    await arriverSurLienInvalide();
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'Date'] });
    await demanderRenvoi();
    controleur
      .expectOne(URL_RENVOYER)
      .flush(
        { erreur: 'x', code: 'trop_de_demandes' },
        { status: 429, statusText: 'Too Many Requests', headers: { 'Retry-After': '42' } },
      );
    await rafraichir();
    expect(element().textContent).toContain('Vous pourrez demander un nouveau lien dans 42 s.');
    expect(bouton('Renvoyer le lien').disabled).toBe(true);
    expect(bouton('Renvoyer le lien').textContent).toContain('(42 s)');

    // Un clic pendant l'attente n'envoie rien.
    await demanderRenvoi();
    controleur.expectNone(URL_RENVOYER);

    vi.advanceTimersByTime(30_000);
    await fixture.whenStable();
    expect(bouton('Renvoyer le lien').textContent).toContain('(12 s)');
    vi.advanceTimersByTime(12_000);
    await fixture.whenStable();
    expect(bouton('Renvoyer le lien').disabled).toBe(false);
    expect(element().textContent).not.toContain('Vous pourrez demander');
  });

  it('second lien ouvert dans le même onglet (routeur réel) : le nouveau jeton est lu et effacé de l’adresse', async () => {
    const AUTRE = 'fedcba9876543210'.repeat(4);
    window.history.replaceState(null, '', '/');
    TestBed.configureTestingModule({
      providers: [
        provideRouter([{ path: 'verifier-email', component: VerifierEmailPage }]),
        // TestBed remplace l'adresse par une fausse (MockPlatformLocation) : ici, le
        // routeur doit écrire dans la vraie, comme dans le navigateur.
        { provide: PlatformLocation, useClass: BrowserPlatformLocation },
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
      ],
    });
    controleur = TestBed.inject(HttpTestingController);
    const harnais = await RouterTestingHarness.create(`/verifier-email#jeton=${JETON}`);
    await harnais.fixture.whenStable();
    const page = (): HTMLElement => harnais.routeNativeElement as HTMLElement;
    const cliquerConfirmer = (): void =>
      [...page().querySelectorAll('button')].find((b) => b.textContent?.includes('Confirmer mon adresse'))?.click();
    expect(window.location.pathname).toBe('/verifier-email');
    expect(window.location.hash).toBe('');

    cliquerConfirmer();
    controleur.expectOne(URL_VERIFIER).flush(null, { status: 204, statusText: 'No Content' });
    await attendre();
    await harnais.fixture.whenStable();
    expect(page().textContent).toContain(MESSAGES.emailVerifie);

    await harnais.navigateByUrl(`/verifier-email#jeton=${AUTRE}`);
    await harnais.fixture.whenStable();
    expect(window.location.hash).toBe('');
    expect(window.location.href).not.toContain(AUTRE);
    expect(page().textContent).not.toContain(MESSAGES.emailVerifie);
    cliquerConfirmer();
    expect(controleur.expectOne(URL_VERIFIER).request.body).toEqual({ jeton: AUTRE });
  });

  it('ni le jeton, ni le mot de passe, ni l’email dans la console (journal de développement compris)', async () => {
    await arriverSurLienInvalide();
    await demanderRenvoi();
    controleur
      .expectOne(URL_RENVOYER)
      .flush(
        { erreur: 'x', code: 'trop_de_demandes' },
        { status: 429, statusText: 'Too Many Requests', headers: { 'Retry-After': '42' } },
      );
    await rafraichir();
    const journal = toutLeJournal();
    // Le journal a bien écrit : il est branché, et il ne garde que code, méthode et chemin.
    expect(journal).toContain('[API] 400 lien_invalide POST /api/professionnels/email/verifier');
    expect(journal).toContain('[API] 429 trop_de_demandes POST /api/professionnels/email/renvoyer');
    expect(journal).not.toContain(JETON);
    expect(journal).not.toContain(MOT_DE_PASSE);
    expect(journal).not.toContain(EMAIL);
  });
});
