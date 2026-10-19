import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import { SessionService, intercepteurAcces } from 'commun';
import { ConnexionPage } from './connexion';

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('ConnexionPage', () => {
  let fixture: ComponentFixture<ConnexionPage>;
  let controleur: HttpTestingController;
  let router: Router;
  let session: SessionService;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [ConnexionPage],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
      ],
    });
    fixture = TestBed.createComponent(ConnexionPage);
    controleur = TestBed.inject(HttpTestingController);
    router = TestBed.inject(Router);
    session = TestBed.inject(SessionService);
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

  async function soumettre(): Promise<void> {
    element().querySelector('form')?.dispatchEvent(new Event('submit'));
    await fixture.whenStable();
  }

  it('n’envoie rien si les champs sont vides, et le dit à côté de chaque champ', async () => {
    await soumettre();
    controleur.expectNone('/api/professionnels/connexion');
    expect(element().textContent).toContain('Saisissez votre email.');
    expect(element().textContent).toContain('Saisissez votre mot de passe.');
    expect(element().querySelector('#email')?.getAttribute('aria-invalid')).toBe('true');
  });

  it('envoie email et mot de passe tels que saisis, puis va au choix de l’espace', async () => {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', ' Exemple#2026 ');
    await soumettre();
    const requete = controleur.expectOne('/api/professionnels/connexion');
    expect(requete.request.body).toEqual({ email: 'agent@example.org', mot_de_passe: ' Exemple#2026 ' });
    expect(requete.request.headers.has('Authorization')).toBe(false);
    requete.flush({ etape: 'complete', jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' });
    await attendre();
    expect(session.etat()).toBe('sans_affectation');
    expect(router.navigate).toHaveBeenCalledWith(['/espaces']);
  });

  it('va au second facteur quand etape = second_facteur', async () => {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', 'Exemple#2026');
    await soumettre();
    controleur.expectOne('/api/professionnels/connexion').flush({ etape: 'second_facteur', jeton_intermediaire: 'i' });
    await attendre();
    expect(session.etat()).toBe('second_facteur');
    expect(router.navigate).toHaveBeenCalledWith(['/second-facteur']);
  });

  it('affiche le message d’après le code, et vide le mot de passe', async () => {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', 'faux');
    await soumettre();
    controleur
      .expectOne('/api/professionnels/connexion')
      .flush({ erreur: 'texte du serveur', code: 'identifiants_invalides' }, { status: 401, statusText: 'Unauthorized' });
    await attendre();
    await fixture.whenStable();
    expect(element().querySelector('[role="alert"]')?.textContent).toContain('Email ou mot de passe incorrect.');
    expect(element().textContent).not.toContain('texte du serveur');
    expect(element().querySelector<HTMLInputElement>('#mot-de-passe')?.value).toBe('');
  });

  it('erreur réseau : message de connexion internet', async () => {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', 'Exemple#2026');
    await soumettre();
    controleur.expectOne('/api/professionnels/connexion').error(new ProgressEvent('error'));
    await attendre();
    await fixture.whenStable();
    expect(element().textContent).toContain('Impossible de joindre le serveur.');
  });

  it('le bouton afficher montre puis masque le mot de passe', async () => {
    const champ = element().querySelector<HTMLInputElement>('#mot-de-passe');
    const bouton = element().querySelector<HTMLButtonElement>('button[aria-pressed]');
    expect(champ?.type).toBe('password');
    bouton?.click();
    await fixture.whenStable();
    expect(champ?.type).toBe('text');
    expect(bouton?.getAttribute('aria-pressed')).toBe('true');
  });
});
