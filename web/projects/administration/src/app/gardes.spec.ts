import { TestBed } from '@angular/core/testing';
import { ActivatedRouteSnapshot, CanActivateFn, Router, RouterStateSnapshot, UrlTree, provideRouter } from '@angular/router';
import { SessionService } from 'commun';
import { exigerAdministrateur, exigerReenrolement, exigerSecondFacteur } from './gardes';

const PAIRE = { jeton_acces: 'a', expire_dans: 900, jeton_renouvellement: 'r' };

describe('gardes de l’administration', () => {
  let session: SessionService;
  let router: Router;

  beforeEach(() => {
    TestBed.configureTestingModule({ providers: [provideRouter([])] });
    session = TestBed.inject(SessionService);
    router = TestBed.inject(Router);
  });

  function executer(garde: CanActivateFn): boolean | string {
    const resultat = TestBed.runInInjectionContext(() => garde({} as ActivatedRouteSnapshot, {} as RouterStateSnapshot));
    return resultat instanceof UrlTree ? router.serializeUrl(resultat) : (resultat as boolean);
  }

  it('chaque écran n’accepte que sa sorte de jeton', () => {
    for (const garde of [exigerSecondFacteur, exigerReenrolement, exigerAdministrateur]) {
      expect(executer(garde)).toBe('/connexion');
    }
    session.ouvrirSecondFacteur('i');
    expect(executer(exigerSecondFacteur)).toBe(true);
    expect(executer(exigerAdministrateur)).toBe('/connexion');
    session.ouvrirReenrolement('r');
    expect(executer(exigerReenrolement)).toBe(true);
    expect(executer(exigerAdministrateur)).toBe('/connexion');
    session.ouvrirAdministrateur(PAIRE);
    expect(executer(exigerAdministrateur)).toBe(true);
    expect(executer(exigerReenrolement)).toBe('/connexion');
  });

  it('une session professionnelle n’ouvre jamais les pages de l’équipe', () => {
    session.ouvrirSansAffectation(PAIRE);
    expect(executer(exigerAdministrateur)).toBe('/connexion');
  });
});
