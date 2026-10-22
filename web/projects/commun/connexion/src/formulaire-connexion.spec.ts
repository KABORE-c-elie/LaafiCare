import { provideHttpClient, withInterceptors } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Router, provideRouter } from '@angular/router';
import {
  PARCOURS_ADMINISTRATEUR,
  PARCOURS_CONNEXION,
  ParcoursConnexion,
  SessionService,
  intercepteurAcces,
} from 'commun';
import { FormulaireConnexionComponent } from './formulaire-connexion';

// Parcours d'une autre application (ex. l'administration) : les routes et pages viennent du jeton.
const PARCOURS: ParcoursConnexion = {
  sorte: 'professionnel',
  pageReenrolement: null,
  routeConnexion: '/api/essai/connexion',
  routeSecondFacteur: '/api/essai/second-facteur',
  pageConnexion: '/entree',
  pageSecondFacteur: '/code',
  pageApresConnexion: '/accueil-connecte',
};

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('FormulaireConnexionComponent', () => {
  let fixture: ComponentFixture<FormulaireConnexionComponent>;
  let controleur: HttpTestingController;
  let router: Router;
  let refus: (string | null)[];
  let envois: number;

  beforeEach(async () => {
    TestBed.configureTestingModule({
      imports: [FormulaireConnexionComponent],
      providers: [
        provideRouter([]),
        provideHttpClient(withInterceptors([intercepteurAcces])),
        provideHttpClientTesting(),
        { provide: PARCOURS_CONNEXION, useValue: PARCOURS },
      ],
    });
    fixture = TestBed.createComponent(FormulaireConnexionComponent);
    fixture.componentRef.setInput('garderMotDePasse', ['email_non_verifie']);
    controleur = TestBed.inject(HttpTestingController);
    router = TestBed.inject(Router);
    vi.spyOn(router, 'navigate').mockResolvedValue(true);
    refus = [];
    envois = 0;
    fixture.componentInstance.refus.subscribe((code) => refus.push(code));
    fixture.componentInstance.envoi.subscribe(() => envois++);
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

  async function soumettre(): Promise<Event> {
    const evenement = new Event('submit', { cancelable: true });
    element().querySelector('form')?.dispatchEvent(evenement);
    await fixture.whenStable();
    return evenement;
  }

  async function connecterAvecRefus(code: string, status: number): Promise<void> {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', 'Exemple#2026');
    await soumettre();
    controleur.expectOne(PARCOURS.routeConnexion).flush({ erreur: 'x', code }, { status, statusText: 'x' });
    await attendre();
    await fixture.whenStable();
  }

  it('route de l’API et pages lues dans PARCOURS_CONNEXION ; pas de soumission native', async () => {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', 'Exemple#2026');
    const evenement = await soumettre();
    expect(evenement.defaultPrevented).toBe(true);
    expect(envois).toBe(1);
    controleur
      .expectOne(PARCOURS.routeConnexion)
      .flush({ etape: 'complete', jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' });
    await attendre();
    expect(TestBed.inject(SessionService).etat()).toBe('sans_affectation');
    expect(router.navigate).toHaveBeenCalledWith([PARCOURS.pageApresConnexion]);
  });

  it('second facteur : page du parcours', async () => {
    saisir('email', 'agent@example.org');
    saisir('mot-de-passe', 'Exemple#2026');
    await soumettre();
    controleur.expectOne(PARCOURS.routeConnexion).flush({ etape: 'second_facteur', jeton_intermediaire: 'i' });
    await attendre();
    expect(router.navigate).toHaveBeenCalledWith([PARCOURS.pageSecondFacteur]);
  });

  it('refus : le code est émis ; mot de passe effacé, sauf pour les codes de `garderMotDePasse`', async () => {
    await connecterAvecRefus('identifiants_invalides', 401);
    expect(refus).toEqual(['identifiants_invalides']);
    expect(element().querySelector<HTMLInputElement>('#mot-de-passe')?.value).toBe('');
    expect(element().querySelector('[role="alert"]')?.textContent).toContain('Email ou mot de passe incorrect.');

    await connecterAvecRefus('email_non_verifie', 403);
    expect(refus).toEqual(['identifiants_invalides', 'email_non_verifie']);
    expect(element().querySelector<HTMLInputElement>('#mot-de-passe')?.value).toBe('Exemple#2026');
  });

  describe('parcours administrateur (§2.5)', () => {
    async function configurerAdministrateur(messagesParCode: Record<string, string> = {}): Promise<SessionService> {
      TestBed.resetTestingModule();
      TestBed.configureTestingModule({
        imports: [FormulaireConnexionComponent],
        providers: [
          provideRouter([]),
          provideHttpClient(withInterceptors([intercepteurAcces])),
          provideHttpClientTesting(),
          { provide: PARCOURS_CONNEXION, useValue: PARCOURS_ADMINISTRATEUR },
        ],
      });
      fixture = TestBed.createComponent(FormulaireConnexionComponent);
      fixture.componentRef.setInput('messagesParCode', messagesParCode);
      controleur = TestBed.inject(HttpTestingController);
      router = TestBed.inject(Router);
      vi.spyOn(router, 'navigate').mockResolvedValue(true);
      await fixture.whenStable();
      saisir('email', 'admin@example.org');
      saisir('mot-de-passe', 'Exemple#2026');
      await soumettre();
      return TestBed.inject(SessionService);
    }

    it('etape reenrolement : jeton de réenrôlement gardé, page de réenrôlement', async () => {
      const session = await configurerAdministrateur();
      controleur
        .expectOne(PARCOURS_ADMINISTRATEUR.routeConnexion)
        .flush({ etape: 'reenrolement', jeton_reenrolement: 'jeton-reenrolement' });
      await attendre();
      expect(session.etat()).toBe('reenrolement');
      expect(session.jetonReenrolement()).toBe('jeton-reenrolement');
      expect(router.navigate).toHaveBeenCalledWith(['/reenrolement']);
    });

    it('etape complete : jamais pour un administrateur, aucune session, erreur interne', async () => {
      const session = await configurerAdministrateur();
      controleur
        .expectOne(PARCOURS_ADMINISTRATEUR.routeConnexion)
        .flush({ etape: 'complete', jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' });
      await attendre();
      await fixture.whenStable();
      expect(session.etat()).toBe('deconnecte');
      expect(router.navigate).not.toHaveBeenCalled();
      expect(element().querySelector('[role="alert"]')?.textContent).toContain('Une erreur est survenue.');
    });

    it('messagesParCode : texte propre à l’application pour un code', async () => {
      await configurerAdministrateur({ email_non_verifie: 'Texte de l’administration.' });
      controleur
        .expectOne(PARCOURS_ADMINISTRATEUR.routeConnexion)
        .flush({ erreur: 'x', code: 'email_non_verifie' }, { status: 403, statusText: 'x' });
      await attendre();
      await fixture.whenStable();
      expect(element().querySelector('[role="alert"]')?.textContent).toContain('Texte de l’administration.');
    });
  });

  it('champs vides : rien n’est envoyé, aucun envoi émis', async () => {
    await soumettre();
    controleur.expectNone(PARCOURS.routeConnexion);
    expect(envois).toBe(0);
    expect(fixture.componentInstance.lireSaisie()).toBeNull();
  });
});
