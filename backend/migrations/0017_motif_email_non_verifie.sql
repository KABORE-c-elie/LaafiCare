-- Nouveau motif de fermeture d'une session (décision du 2026-10-15) :
-- l'email d'un compte professionnel ou administrateur n'est plus vérifié
-- (le trigger de la migration 0016 efface la vérification à chaque
-- changement d'email). `renouveler` ferme alors la session, qui pourrait
-- sinon continuer jusqu'à 12 h (professionnel) ou 24 h (administrateur).
--
-- La contrainte avait reçu de PostgreSQL le nom automatique
-- `session_motif_revocation_check` (vérifié dans la base de développement
-- le 2026-10-15, et sur une base neuve par tests/migration_0017.rs). Elle
-- est remplacée par une contrainte au même nom, posé explicitement cette
-- fois. La nouvelle liste contient toute l'ancienne : aucune ligne
-- existante ne peut la violer.
ALTER TABLE session DROP CONSTRAINT session_motif_revocation_check;
ALTER TABLE session ADD CONSTRAINT session_motif_revocation_check
    CHECK (motif_revocation IN (
        'deconnexion', 'deconnexion_partout',
        'reinitialisation_mot_de_passe', 'rejeu_detecte',
        'changement_affectation', 'desactivation_compte',
        'email_non_verifie'));
