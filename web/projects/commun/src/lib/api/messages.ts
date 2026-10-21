import { ErreurDecodee } from './erreur';

// Textes affichés, choisis d'après le champ `code` (docs/api.md §1.3), jamais
// d'après le code HTTP ni le message du serveur. Validés par le porteur.
export const MESSAGES = {
  identifiantsInvalides: 'Email ou mot de passe incorrect.',
  // Texte exact de docs/api.md §2.4 (403 `email_non_verifie`).
  emailNonVerifie:
    "Votre email n'est pas encore vérifié. Ouvrez le lien que nous vous avons envoyé, ou demandez-en un nouveau.",
  codeInvalide: 'Code incorrect.',
  // Vérification de l'email (docs/api.md §2.7) : textes du porteur (2026-10-21).
  emailVerifie: 'Votre email est vérifié. Vous pouvez maintenant vous connecter.',
  lienInvalide: "Ce lien n'est plus valable ou a déjà servi. Demandez-en un nouveau.",
  lienRenvoye: 'Si votre compte existe, un nouveau lien vous a été envoyé.',
  delaiDepasse: 'Le délai est dépassé. Reconnectez-vous.',
  affectationNonExercable: "Cet espace n'est pas disponible pour le moment.",
  accesRetire: 'Votre accès à cet espace a été retiré.',
  sessionTerminee: 'Votre session est terminée. Reconnectez-vous.',
  reseau: 'Impossible de joindre le serveur. Vérifiez votre connexion internet, puis réessayez.',
  erreurInterne: 'Une erreur est survenue. Réessayez dans un instant.',
} as const;

const PAR_CODE: Readonly<Record<string, string>> = {
  identifiants_invalides: MESSAGES.identifiantsInvalides,
  email_non_verifie: MESSAGES.emailNonVerifie,
  code_invalide: MESSAGES.codeInvalide,
  lien_invalide: MESSAGES.lienInvalide,
  affectation_non_exercable: MESSAGES.affectationNonExercable,
  acces_refuse: MESSAGES.accesRetire,
};

/**
 * Message par défaut d'une erreur. Un écran qui doit réagir autrement (ex.
 * `jeton_invalide` pendant le second facteur) teste le code avant d'appeler ceci.
 * Un code sans texte prévu retombe sur le message d'erreur interne : jamais le
 * message brut du serveur.
 */
export function messagePourErreur(erreur: ErreurDecodee): string {
  if (erreur.genre === 'reseau') {
    return MESSAGES.reseau;
  }
  if (erreur.genre === 'api') {
    return PAR_CODE[erreur.code] ?? MESSAGES.erreurInterne;
  }
  return MESSAGES.erreurInterne;
}
