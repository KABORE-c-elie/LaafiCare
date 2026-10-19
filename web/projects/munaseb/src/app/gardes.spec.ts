import { TestBed } from '@angular/core/testing';
import { ActivatedRouteSnapshot, CanActivateFn, Router, RouterStateSnapshot, UrlTree, provideRouter } from '@angular/router';
import { Affectation, SessionService } from 'commun';
import { exigerConnecte, exigerEnExercice, exigerSecondFacteur } from './gardes';

const PAIRE = { jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' };
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

describe('gardes de route', () => {
  let session: SessionService;
  let router: Router;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    session = TestBed.inject(SessionService);
    router = TestBed.inject(Router);
  });

  function executer(garde: CanActivateFn): boolean | string {
    const resultat = TestBed.runInInjectionContext(() =>
      garde({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot),
    );
    return resultat instanceof UrlTree ? router.serializeUrl(resultat) : (resultat as boolean);
  }

  it('aucune page métier sans jeton en exercice', () => {
    expect(executer(exigerEnExercice)).toBe('/connexion');
    session.ouvrirSecondFacteur('i');
    expect(executer(exigerEnExercice)).toBe('/connexion');
    session.ouvrirSansAffectation(PAIRE);
    expect(executer(exigerEnExercice)).toBe('/espaces');
    session.ouvrirEnExercice(PAIRE, AFFECTATION);
    expect(executer(exigerEnExercice)).toBe(true);
  });

  it('le choix de l’espace exige une session ouverte', () => {
    expect(executer(exigerConnecte)).toBe('/connexion');
    session.ouvrirSecondFacteur('i');
    expect(executer(exigerConnecte)).toBe('/connexion');
    session.ouvrirSansAffectation(PAIRE);
    expect(executer(exigerConnecte)).toBe(true);
    session.ouvrirEnExercice(PAIRE, AFFECTATION);
    expect(executer(exigerConnecte)).toBe(true);
  });

  it('le second facteur exige un jeton intermédiaire', () => {
    expect(executer(exigerSecondFacteur)).toBe('/connexion');
    session.ouvrirSecondFacteur('i');
    expect(executer(exigerSecondFacteur)).toBe(true);
  });
});
