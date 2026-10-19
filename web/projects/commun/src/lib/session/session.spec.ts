import { TestBed } from '@angular/core/testing';
import { Affectation, PaireJetons } from '../api/types';
import { SessionService } from './session';

const PAIRE: PaireJetons = { jeton_acces: 'acces-1', expire_dans: 900, jeton_renouvellement: 'renouv-1' };
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

describe('SessionService', () => {
  let session: SessionService;

  beforeEach(() => {
    session = TestBed.inject(SessionService);
  });

  it('commence déconnectée, sans aucun jeton', () => {
    expect(session.etat()).toBe('deconnecte');
    expect(session.jetonAcces()).toBeNull();
    expect(session.jetonRenouvellement()).toBeNull();
  });

  it('second facteur : seul le jeton intermédiaire est gardé', () => {
    session.ouvrirSecondFacteur('intermediaire');
    expect(session.etat()).toBe('second_facteur');
    expect(session.jetonIntermediaire()).toBe('intermediaire');
    expect(session.connecte()).toBe(false);
  });

  it('la connexion complète efface le jeton intermédiaire', () => {
    session.ouvrirSecondFacteur('intermediaire');
    session.ouvrirSansAffectation(PAIRE);
    expect(session.etat()).toBe('sans_affectation');
    expect(session.jetonIntermediaire()).toBeNull();
    expect(session.jetonAcces()).toBe('acces-1');
  });

  it('l’activation remplace la paire et retient l’affectation', () => {
    session.ouvrirSansAffectation(PAIRE);
    session.ouvrirEnExercice({ ...PAIRE, jeton_acces: 'acces-2', jeton_renouvellement: 'renouv-2' }, AFFECTATION);
    expect(session.etat()).toBe('en_exercice');
    expect(session.jetonRenouvellement()).toBe('renouv-2');
    expect(session.affectation()?.id).toBe('aff-1');
  });

  it('le renouvellement remplace les deux jetons sans changer la sorte', () => {
    session.ouvrirEnExercice(PAIRE, AFFECTATION);
    session.remplacerPaire({ jeton_acces: 'acces-3', expire_dans: 900, jeton_renouvellement: 'renouv-3' });
    expect(session.etat()).toBe('en_exercice');
    expect(session.jetonAcces()).toBe('acces-3');
    expect(session.jetonRenouvellement()).toBe('renouv-3');
  });

  it('effacer vide tout', () => {
    session.ouvrirEnExercice(PAIRE, AFFECTATION);
    session.effacer();
    expect(session.etat()).toBe('deconnecte');
    expect(session.jetonAcces()).toBeNull();
    expect(session.jetonRenouvellement()).toBeNull();
    expect(session.affectation()).toBeNull();
  });

  it('ne touche jamais au stockage du navigateur', () => {
    session.ouvrirEnExercice(PAIRE, AFFECTATION);
    expect(localStorage.length).toBe(0);
    expect(sessionStorage.length).toBe(0);
    expect(document.cookie).toBe('');
  });
});
