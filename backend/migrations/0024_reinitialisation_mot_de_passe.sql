-- Étape 3.12 (décisions R1 à R21 du porteur, 2026-10-08 et 2026-10-21) :
-- liens de réinitialisation du mot de passe d'un compte professionnel ou
-- administrateur, dans la table des jetons d'email (migration 0020).
--
-- Même méthode que la migration 0017 : la contrainte, au même nom, reprend
-- toute l'ancienne liste ; aucune ligne existante ne peut la violer.
ALTER TABLE jeton_email DROP CONSTRAINT jeton_email_objet_check;
ALTER TABLE jeton_email ADD CONSTRAINT jeton_email_objet_check
    CHECK (objet IN ('verification_email', 'reinitialisation_mot_de_passe'));

-- Liens de réinitialisation encore en attente d'un compte : comptés à chaque
-- demande (au plus 3 liens valables, R5 ; une minute au moins entre deux,
-- R13). Pas d'index unique ici, contrairement à la vérification de l'email :
-- une nouvelle demande n'annule pas les précédentes (R3).
CREATE INDEX jeton_email_reinitialisation_en_attente_idx
    ON jeton_email (compte_id, date_creation)
    WHERE objet = 'reinitialisation_mot_de_passe' AND utilise_le IS NULL AND annule_le IS NULL;
