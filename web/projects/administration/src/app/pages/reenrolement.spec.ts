import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ESPACE_APPLICATION, MESSAGES, MessageNavigation, SessionService, intercepteurAcces, intercepteurJournalDev } from 'commun';
import { ESPACE_ADMINISTRATION } from '../espace';
import { MESSAGES_REENROLEMENT, ReenrolementPage, grouperCle } from './reenrolement';

const COMMENCER = '/api/administrateurs/reenrolement/commencer';
const CONFIRMER = '/api/administrateurs/reenrolement/confirmer';
// Valeurs fictives : secret inventé, codes de secours inventés.
const CLE = 'JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP';
const URL = `otpauth://totp/LaafiCare:Administrateur?secret=${CLE}&issuer=LaafiCare`;
const CODES = Array.from({ length: 10 }, (_, i) => `AAAA${i}-BBBB${i}`);
const MOT_DE_PASSE = 'Exemple#2026';

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('ReenrolementPage', () => {
  let fixture: ComponentFixture<ReenrolementPage>;
  let controleur: HttpTestingController;
  let router: Router;
  let session: SessionService;
  let consoles: ReturnType<typeof vi.spyOn>[];
  let stockage: ReturnType<typeof vi.spyOn>;

  beforeEach(async () => {
    consoles = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) =>
      vi.spyOn(console, m).mockImplementation(() => undefined),
    );
    // Aucun stockage du navigateur (localStorage, sessionStorage) : jamais.
    stockage = vi.spyOn(Storage.prototype, 'setItem');
    TestBed.configureTestingModule({
      imports: [ReenrolementPage],
      providers: [
        provideRouter([]),
        // Journal de développement branché, comme avec `ng serve`.
        provideHttpClient(withInterceptors([intercepteurAcces, intercepteurJournalDev])),
        provideHttpClientTesting(),
        { provide: ESPACE_APPLICATION, useValue: ESPACE_ADMINISTRATION },
      ],
    });
    session = TestBed.inject(SessionService);
    session.ouvrirReenrolement('jeton-reenrolement');
    fixture = TestBed.createComponent(ReenrolementPage);
    controleur = TestBed.inject(HttpTestingController);
    router = TestBed.inject(Router);
    vi.spyOn(router, 'navigate').mockResolvedValue(true);
    await fixture.whenStable();
  });

  afterEach(() => {
    controleur.verify();
    consoles.forEach((e) => e.mockRestore());
    stockage.mockRestore();
  });

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function stabiliser(): Promise<void> {
    await attendre();
    await fixture.whenStable();
  }

  function saisir(id: string, valeur: string): void {
    const champ = element().querySelector<HTMLInputElement>(`#${id}`);
    if (champ === null) {
      throw new Error(`champ ${id} absent`);
    }
    champ.value = valeur;
    champ.dispatchEvent(new Event('input'));
  }

  async function soumettre(): Promise<Event> {
    const evenement = new Event('submit', { cancelable: true });
    element().querySelector('form')?.dispatchEvent(evenement);
    await fixture.whenStable();
    return evenement;
  }

  function bouton(texte: string): HTMLButtonElement | undefined {
    return [...element().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes(texte));
  }

  async function arriverAuQr(): Promise<void> {
    saisir('mot-de-passe', MOT_DE_PASSE);
    await soumettre();
    const requete = controleur.expectOne(COMMENCER);
    expect(requete.request.body).toEqual({ mot_de_passe: MOT_DE_PASSE });
    expect(requete.request.headers.get('Authorization')).toBe('Bearer jeton-reenrolement');
    requete.flush({ url_otpauth: URL, cle_base32: CLE });
    await stabiliser();
  }

  async function arriverAuxCodes(): Promise<void> {
    await arriverAuQr();
    saisir('code', '123456');
    await soumettre();
    const requete = controleur.expectOne(CONFIRMER);
    expect(requete.request.body).toEqual({ code: '123456' });
    requete.flush({ codes_secours: CODES });
    await stabiliser();
  }

  function toutLeJournal(): string {
    return consoles.flatMap((e) => e.mock.calls.map((appel: unknown[]) => appel.map(String).join(' '))).join('\n');
  }

  it('mot de passe redemandé, avec le jeton de réenrôlement ; QR code dessiné, clé masquée', async () => {
    await arriverAuQr();
    expect(element().querySelector('svg.qr path')?.getAttribute('d')).not.toBe('');
    expect(element().textContent).not.toContain(CLE);
    expect(element().querySelector<HTMLInputElement>('#mot-de-passe')).toBeNull();
  });

  it('la clé ne s’affiche qu’à la demande, par groupes de 4', async () => {
    await arriverAuQr();
    bouton('Afficher la clé')?.click();
    await fixture.whenStable();
    expect(element().querySelector('.cle')?.textContent).toBe(grouperCle(CLE));
    expect(grouperCle('ABCDEFGHIJ')).toBe('ABCD EFGH IJ');
    expect(bouton('Masquer la clé')?.getAttribute('aria-pressed')).toBe('true');
  });

  it('mot de passe faux : « Mot de passe incorrect. », champ vidé', async () => {
    saisir('mot-de-passe', 'faux');
    await soumettre();
    controleur.expectOne(COMMENCER).flush({ erreur: 'x', code: 'identifiants_invalides' }, { status: 401, statusText: 'x' });
    await stabiliser();
    expect(element().querySelector('[role="alert"]')?.textContent).toContain(MESSAGES_REENROLEMENT.motDePasseIncorrect);
    expect(element().querySelector<HTMLInputElement>('#mot-de-passe')?.value).toBe('');
  });

  it('code faux : « Code incorrect. », champ vidé, QR toujours là', async () => {
    await arriverAuQr();
    saisir('code', '000000');
    await soumettre();
    controleur.expectOne(CONFIRMER).flush({ erreur: 'x', code: 'code_invalide' }, { status: 401, statusText: 'x' });
    await stabiliser();
    expect(element().querySelector('[role="alert"]')?.textContent).toContain(MESSAGES.codeInvalide);
    expect(element().querySelector<HTMLInputElement>('#code')?.value).toBe('');
    expect(element().querySelector('svg.qr')).not.toBeNull();
  });

  for (const [etape, route] of [
    ['commencer', COMMENCER],
    ['confirmer', CONFIRMER],
  ] as const) {
    it(`jeton_invalide sur ${etape} (fenêtre expirée) : session effacée, retour à la connexion`, async () => {
      const poser = vi.spyOn(TestBed.inject(MessageNavigation), 'poser');
      if (etape === 'confirmer') {
        await arriverAuQr();
        saisir('code', '123456');
      } else {
        saisir('mot-de-passe', MOT_DE_PASSE);
      }
      await soumettre();
      controleur.expectOne(route).flush({ erreur: 'x', code: 'jeton_invalide' }, { status: 401, statusText: 'x' });
      await stabiliser();
      expect(session.etat()).toBe('deconnecte');
      expect(poser).toHaveBeenCalledWith({ genre: 'erreur', texte: MESSAGES.delaiDepasse });
      expect(router.navigate).toHaveBeenCalledWith(['/connexion']);
    });
  }

  it('les 10 codes de secours, puis plus de QR ni de clé ; case obligatoire avant de partir', async () => {
    await arriverAuxCodes();
    const affiches = [...element().querySelectorAll('.codes-secours li')].map((li) => li.textContent?.trim());
    expect(affiches).toEqual(CODES);
    expect(element().querySelector('svg.qr')).toBeNull();
    expect(element().textContent).not.toContain(CLE);
    // Aucun moyen de copie, de téléchargement ou d'impression (porteur, B3).
    for (const mot of ['Copier', 'Télécharger', 'Imprimer']) {
      expect(bouton(mot)).toBeUndefined();
    }
    const partir = bouton('Revenir à la connexion');
    expect(partir?.disabled).toBe(true);
    partir?.click();
    await fixture.whenStable();
    expect(router.navigate).not.toHaveBeenCalled();
  });

  it('case cochée : codes effacés, aucune session, retour à la connexion avec un message', async () => {
    const poser = vi.spyOn(TestBed.inject(MessageNavigation), 'poser');
    await arriverAuxCodes();
    element().querySelector<HTMLInputElement>('input[type="checkbox"]')?.click();
    await fixture.whenStable();
    bouton('Revenir à la connexion')?.click();
    await stabiliser();
    expect(element().querySelectorAll('.codes-secours li').length).toBe(0);
    expect(session.etat()).toBe('deconnecte');
    expect(session.jetonReenrolement()).toBeNull();
    expect(poser).toHaveBeenCalledWith({ genre: 'info', texte: MESSAGES_REENROLEMENT.termine });
    expect(router.navigate).toHaveBeenCalledWith(['/connexion']);
  });

  it('codes montrés une seule fois : une nouvelle page ne les a plus', async () => {
    await arriverAuxCodes();
    fixture.destroy();
    const autre = TestBed.createComponent(ReenrolementPage);
    await autre.whenStable();
    expect((autre.nativeElement as HTMLElement).querySelectorAll('.codes-secours li').length).toBe(0);
    expect((autre.nativeElement as HTMLElement).querySelector('#mot-de-passe')).not.toBeNull();
  });

  it('double envoi impossible : deux soumissions, une seule requête', async () => {
    saisir('mot-de-passe', MOT_DE_PASSE);
    await soumettre();
    await soumettre();
    const requetes = controleur.match(COMMENCER);
    expect(requetes.length).toBe(1);
    requetes[0].flush({ url_otpauth: URL, cle_base32: CLE });
    await stabiliser();
  });

  it('ni l’adresse otpauth, ni la clé, ni les codes, ni le mot de passe : jamais stockés ni écrits dans la console', async () => {
    await arriverAuQr();
    bouton('Afficher la clé')?.click();
    saisir('code', '000000');
    await soumettre();
    controleur.expectOne(CONFIRMER).flush({ erreur: 'x', code: 'code_invalide' }, { status: 401, statusText: 'x' });
    await stabiliser();
    saisir('code', '123456');
    await soumettre();
    controleur.expectOne(CONFIRMER).flush({ codes_secours: CODES });
    await stabiliser();
    expect(stockage).not.toHaveBeenCalled();
    const journal = toutLeJournal();
    // Le journal de développement est bien branché : il a écrit le refus du code.
    expect(journal).toContain('[API] 401 code_invalide POST /api/administrateurs/reenrolement/confirmer');
    for (const secret of [CLE, URL, MOT_DE_PASSE, '000000', '123456', ...CODES]) {
      expect(journal).not.toContain(secret);
    }
  });
});
