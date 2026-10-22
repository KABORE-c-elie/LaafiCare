import { EspaceApplication } from 'commun';

/** Panneau d'accueil de l'application d'administration (connexion, second facteur, réenrôlement). */
export const ESPACE_ADMINISTRATION: EspaceApplication = {
  logo: '/logo-laaficare.svg',
  titre: 'Administration LaafiCare',
  phrase: 'Réservé à l’équipe LaafiCare : décisions sur les demandes de structure.',
};

/**
 * Texte du 403 `email_non_verifie` pour un administrateur (validé par le
 * porteur, B3) : aucune route ne lui permet de renvoyer le lien (§2.7 ne vaut
 * que pour les professionnels).
 */
export const MESSAGE_EMAIL_NON_VERIFIE_ADMINISTRATEUR =
  "Votre email n'est pas encore vérifié. Contactez l'équipe LaafiCare.";
