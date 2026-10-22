import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ESPACE_APPLICATION, MESSAGES, SessionService, intercepteurAcces } from 'commun';
import { SecondFacteurPage } from 'commun/connexion';
import { ESPACE_MUNASEB } from '../espace';
import { attendre } from './donnees-test';

// Écran commun (commun/connexion), essayé avec le parcours professionnel par défaut.

const ROUTE = '/api/professionnels/connexion/second-facteur';
const PAIRE = { etape: 'complete', jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' };

describe('SecondFacteurPage', () => {
  let fixture: ComponentFixture<SecondFacteurPage>;
  let controleur: HttpTestingController;
  let router: Router;
  let session: SessionService;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [SecondFacteurPage],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
        { provide: ESPACE_APPLICATION, useValue: ESPACE_MUNASEB },
      ],
    });
    session = TestBed.inject(SessionService);
    session.ouvrirSecondFacteur('jeton-intermediaire');
    fixture = TestBed.createComponent(SecondFacteurPage);
    controleur = TestBed.inject(HttpTestingController);
    router = TestBed.inject(Router);
    vi.spyOn(router, 'navigate').mockResolvedValue(true);
    await fixture.whenStable();
  });

  afterEach(() => controleur.verify());

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  function saisir(id: string, valeur: string): void {
    const champ = element().querySelector<HTMLInputElement>(`#${id}`);
    if (champ === null) {
      throw new Error(`champ ${id} absent`);
    }
    champ.value = valeur;
    champ.dispatchEvent(new Event('input'));
  }

  /** Soumission comme le navigateur : l'évènement doit être annulé, sinon la page se recharge. */
  async function soumettre(): Promise<Event> {
    const evenement = new Event('submit', { cancelable: true });
    element().querySelector('form')?.dispatchEvent(evenement);
    await fixture.whenStable();
    return evenement;
  }

  it('« Valider » envoie le code, sans soumission native du navigateur (pas de rechargement)', async () => {
    saisir('code', '123456');
    const evenement = await soumettre();
    expect(evenement.defaultPrevented).toBe(true);
    const requete = controleur.expectOne(ROUTE);
    expect(requete.request.body).toEqual({ code: '123456' });
    expect(requete.request.headers.get('Authorization')).toBe('Bearer jeton-intermediaire');
    requete.flush(PAIRE);
    await attendre();
    expect(session.etat()).toBe('sans_affectation');
    expect(router.navigate).toHaveBeenCalledWith(['/espaces']);
  });

  it('code de secours : seul le champ code_secours part', async () => {
    [...element().querySelectorAll('button')].find((b) => b.textContent?.includes('code de secours'))?.click();
    await fixture.whenStable();
    saisir('code-secours', 'ABCDE-FGHIJ');
    await soumettre();
    expect(controleur.expectOne(ROUTE).request.body).toEqual({ code_secours: 'ABCDE-FGHIJ' });
  });

  it('401 code_invalide : « Code incorrect. », champ vidé', async () => {
    saisir('code', '000000');
    await soumettre();
    controleur.expectOne(ROUTE).flush({ erreur: 'x', code: 'code_invalide' }, { status: 401, statusText: 'x' });
    await attendre();
    await fixture.whenStable();
    expect(element().querySelector('[role="alert"]')?.textContent).toContain(MESSAGES.codeInvalide);
    expect(element().querySelector<HTMLInputElement>('#code')?.value).toBe('');
  });

  it('401 jeton_invalide : retour à la connexion, session effacée', async () => {
    saisir('code', '123456');
    await soumettre();
    controleur.expectOne(ROUTE).flush({ erreur: 'x', code: 'jeton_invalide' }, { status: 401, statusText: 'x' });
    await attendre();
    expect(session.etat()).toBe('deconnecte');
    expect(router.navigate).toHaveBeenCalledWith(['/connexion']);
  });
});
