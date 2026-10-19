import { HttpErrorResponse } from '@angular/common/http';
import { codeErreur, decoderErreur } from './erreur';
import { MESSAGES, messagePourErreur } from './messages';

function reponse(status: number, error: unknown): HttpErrorResponse {
  return new HttpErrorResponse({ status, error });
}

describe('decoderErreur', () => {
  it('lit le code du corps {erreur, code}', () => {
    const decodee = decoderErreur(reponse(401, { erreur: 'texte du serveur', code: 'identifiants_invalides' }));
    expect(decodee).toEqual({
      genre: 'api',
      statut: 401,
      code: 'identifiants_invalides',
      corps: { erreur: 'texte du serveur', code: 'identifiants_invalides' },
    });
  });

  it('garde les champs complémentaires (statut_actuel)', () => {
    const decodee = decoderErreur(
      reponse(409, { erreur: 'x', code: 'transition_invalide', statut_actuel: 'paye' }),
    );
    expect(decodee.genre === 'api' && decodee.corps.statut_actuel).toBe('paye');
  });

  it('classe le statut 0 comme une erreur réseau (délai dépassé ou réseau coupé)', () => {
    expect(decoderErreur(reponse(0, new ProgressEvent('error')))).toEqual({ genre: 'reseau' });
  });

  it('ne se fie jamais au seul code HTTP : un 401 sans corps reste « inconnue »', () => {
    expect(decoderErreur(reponse(401, null))).toEqual({ genre: 'inconnue', statut: 401 });
    expect(codeErreur(reponse(401, '<html>'))).toBeNull();
  });

  it('refuse un corps dont le code n’est pas une chaîne', () => {
    expect(decoderErreur(reponse(500, { erreur: 'x', code: 42 })).genre).toBe('inconnue');
  });

  it('traite une erreur qui ne vient pas d’HttpClient comme inconnue', () => {
    expect(decoderErreur(new Error('x'))).toEqual({ genre: 'inconnue', statut: 0 });
  });
});

describe('messagePourErreur', () => {
  it('choisit le message d’après le code, jamais d’après le message du serveur', () => {
    const decodee = decoderErreur(reponse(401, { erreur: 'autre texte', code: 'identifiants_invalides' }));
    expect(messagePourErreur(decodee)).toBe('Email ou mot de passe incorrect.');
  });

  it('reprend le texte exact de docs/api.md §2.4 pour email_non_verifie', () => {
    const decodee = decoderErreur(reponse(403, { erreur: 'x', code: 'email_non_verifie' }));
    expect(messagePourErreur(decodee)).toBe(
      "Votre email n'est pas encore vérifié. Ouvrez le lien que nous vous avons envoyé, ou demandez-en un nouveau.",
    );
  });

  it('donne le message réseau pour une erreur réseau', () => {
    expect(messagePourErreur({ genre: 'reseau' })).toBe(MESSAGES.reseau);
  });

  it('retombe sur le message d’erreur interne pour un code sans texte prévu', () => {
    const decodee = decoderErreur(reponse(500, { erreur: 'détail', code: 'erreur_interne' }));
    expect(messagePourErreur(decodee)).toBe(MESSAGES.erreurInterne);
    expect(messagePourErreur({ genre: 'inconnue', statut: 502 })).toBe(MESSAGES.erreurInterne);
  });
});
