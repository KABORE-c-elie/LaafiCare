-- Étape 3.11 réduite (décision DS5 du porteur, 2026-10-21) : motif de
-- fermeture des sessions ouvertes sur une affectation désactivée.
--
-- Même méthode que les migrations 0017 et 0020 : la contrainte, au même
-- nom, reprend toute l'ancienne liste ; aucune ligne existante ne peut la
-- violer.
ALTER TABLE session DROP CONSTRAINT session_motif_revocation_check;
ALTER TABLE session ADD CONSTRAINT session_motif_revocation_check
    CHECK (motif_revocation IN (
        'deconnexion', 'deconnexion_partout',
        'reinitialisation_mot_de_passe', 'rejeu_detecte',
        'changement_affectation', 'desactivation_compte',
        'email_non_verifie', 'desactivation_totp',
        'desactivation_affectation'));
