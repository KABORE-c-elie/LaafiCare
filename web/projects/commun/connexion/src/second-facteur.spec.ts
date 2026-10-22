import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { ESPACE_APPLICATION, PARCOURS_CONNEXION, ParcoursConnexion, SessionService, intercepteurAcces } from 'commun';
import { SecondFacteurPage } from './second-facteur';

const PARCOURS: ParcoursConnexion = {
  routeConnexion: '/api/essai/connexion',
  routeSecondFacteur: '/api/essai/second-facteur',
  pageConnexion: '/entree',
  pageSecondFacteur: '/code',
  pageApresConnexion: '/accueil-connecte',
};

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('SecondFacteurPage (commun)', () => {
  let fixture: ComponentFixture<SecondFacteurPage>;
  let controleur: HttpTestingController;
  let router: Router;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [SecondFacteurPage],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
        { provide: PARCOURS_CONNEXION, useValue: PARCOURS },
        { provide: ESPACE_APPLICATION, useValue: { logo: '/logo.svg', titre: 'Espace d’essai', phrase: 'Phrase' } },
      ],
    });
    TestBed.inject(SessionService).ouvrirSecondFacteur('jeton-intermediaire');
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

  async function envoyerCode(): Promise<Event> {
    const champ = element().querySelector<HTMLInputElement>('#code');
    if (champ === null) {
      throw new Error('champ absent');
    }
    champ.value = '123456';
    champ.dispatchEvent(new Event('input'));
    const evenement = new Event('submit', { cancelable: true });
    element().querySelector('form')?.dispatchEvent(evenement);
    await fixture.whenStable();
    return evenement;
  }

  it('panneau d’accueil de l’application (ESPACE_APPLICATION), sans phrase', () => {
    expect(element().querySelector('h1')?.textContent).toContain('Espace d’essai');
    expect(element().querySelector('.accueil__phrase')).toBeNull();
  });

  it('route du parcours, jeton intermédiaire, puis page après connexion du parcours', async () => {
    const evenement = await envoyerCode();
    expect(evenement.defaultPrevented).toBe(true);
    const requete = controleur.expectOne(PARCOURS.routeSecondFacteur);
    expect(requete.request.headers.get('Authorization')).toBe('Bearer jeton-intermediaire');
    requete.flush({ etape: 'complete', jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' });
    await attendre();
    expect(router.navigate).toHaveBeenCalledWith([PARCOURS.pageApresConnexion]);
  });

  it('jeton_invalide : retour à la page de connexion du parcours', async () => {
    await envoyerCode();
    controleur
      .expectOne(PARCOURS.routeSecondFacteur)
      .flush({ erreur: 'x', code: 'jeton_invalide' }, { status: 401, statusText: 'x' });
    await attendre();
    expect(router.navigate).toHaveBeenCalledWith([PARCOURS.pageConnexion]);
  });
});
