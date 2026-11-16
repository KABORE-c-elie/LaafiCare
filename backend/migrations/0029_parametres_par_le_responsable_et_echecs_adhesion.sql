-- Suite de l'étape 5 (demandes du porteur du 2026-11-16, avant la pull
-- request). La migration 0028 n'est pas modifiée : celle-ci l'adapte.
--
-- 1. Les paramètres de la MUNASEB (cotisation, plafond, carence…) ne sont
--    plus modifiés par un agent, mais par le **responsable** de la structure
--    MUNASEB ; les agents gardent la lecture. Les colonnes de l'auteur
--    prennent un nom qui ne dit plus « agent ».
-- 2. Chaque échec d'adhésion (NIP et date de naissance qui ne désignent
--    aucun patient) est tracé avec l'affectation de l'agent : la trace sert
--    à la limite d'échecs (OWASP API4:2023, Authentication Cheat Sheet).

-- ---------------------------------------------------------------------
-- 1. Auteur d'une version des paramètres : le responsable
-- ---------------------------------------------------------------------

-- Doc PostgreSQL 18, ALTER TABLE : RENAME COLUMN met à jour les index et
-- les contraintes qui utilisent la colonne (dont l'index de la version
-- initiale et la clé étrangère MATCH FULL).
ALTER TABLE parametre_munaseb RENAME COLUMN agent_affectation_id TO auteur_affectation_id;
ALTER TABLE parametre_munaseb RENAME COLUMN agent_structure_id TO auteur_structure_id;
ALTER TABLE parametre_munaseb RENAME COLUMN agent_role TO auteur_role;
ALTER TABLE parametre_munaseb RENAME CONSTRAINT parametre_munaseb_agent TO parametre_munaseb_auteur;

-- Seul le rôle `responsable` peut désormais être l'auteur d'une version.
-- NOT VALID (doc PostgreSQL 18, ALTER TABLE) : les versions déjà écrites
-- par un agent, s'il y en a, restent telles quelles (elles ne se modifient
-- jamais) ; toute nouvelle version est contrôlée. À ne jamais valider avec
-- VALIDATE CONSTRAINT.
ALTER TABLE parametre_munaseb DROP CONSTRAINT parametre_munaseb_agent_role_check;
ALTER TABLE parametre_munaseb ADD CONSTRAINT parametre_munaseb_auteur_responsable
    CHECK (auteur_role IS NULL OR auteur_role = 'responsable') NOT VALID;

-- ---------------------------------------------------------------------
-- 2. Échecs d'adhésion
-- ---------------------------------------------------------------------

-- Une ligne par échec, jamais modifiée ni supprimée. Ni le NIP ni la date
-- de naissance essayés ne sont enregistrés : ce peuvent être ceux d'un
-- autre patient, et la trace n'a besoin que de qui et quand.
CREATE TABLE echec_adhesion_munaseb (
    id                    UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_affectation_id  UUID NOT NULL,
    agent_structure_id    UUID NOT NULL,
    agent_role            TEXT NOT NULL CHECK (agent_role = 'agent_assurance_munaseb'),
    date_echec            TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT echec_adhesion_agent
        FOREIGN KEY (agent_affectation_id, agent_structure_id, agent_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);

-- Comptage par affectation dans la fenêtre glissante.
CREATE INDEX echec_adhesion_munaseb_affectation_date_idx
    ON echec_adhesion_munaseb (agent_affectation_id, date_echec);

CREATE FUNCTION echec_adhesion_munaseb_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'echec_adhesion_munaseb : les lignes ne sont jamais modifiees ni supprimees (%)', TG_OP;
END;
$$;

CREATE TRIGGER echec_adhesion_munaseb_sans_modification
    BEFORE UPDATE OR DELETE ON echec_adhesion_munaseb
    FOR EACH ROW EXECUTE FUNCTION echec_adhesion_munaseb_non_modifiable();

CREATE TRIGGER echec_adhesion_munaseb_sans_troncature
    BEFORE TRUNCATE ON echec_adhesion_munaseb
    FOR EACH STATEMENT EXECUTE FUNCTION echec_adhesion_munaseb_non_modifiable();

-- Droits (suite de P1, migration 0026) : le serveur lit et ajoute, jamais ne
-- modifie ni n'efface.
REVOKE UPDATE, DELETE ON echec_adhesion_munaseb FROM laaficare_serveur;
