-- Code OTP du Patient (section 11). Colonnes sur `utilisateur`, pas de
-- table séparée -- même logique que `mot_de_passe_hash`/`email` : NULL pour
-- les rôles qui n'en ont pas besoin (décision actée avec le porteur).
--
-- Un seul code actif à la fois par construction : une nouvelle demande
-- écrase ces deux colonnes (UPDATE), l'ancien code cesse simplement
-- d'exister -- pas besoin d'historique ni de statut "invalidé".

ALTER TABLE utilisateur
    ADD COLUMN otp_code_hash TEXT,
    ADD COLUMN otp_expire_a  TIMESTAMPTZ;
