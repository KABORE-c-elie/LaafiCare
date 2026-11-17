-- Module médecin, étape 2 (décisions du porteur du 2026-11-17, CLAUDE.md
-- §17) : journal des accès aux données d'un patient, et trace des essais
-- sans résultat (pour les limites d'essais).
--
-- Règles du porteur :
-- - chaque scan du NIP, chaque lecture du dossier et chaque affichage du
--   téléphone est écrit **avant** que les données soient renvoyées ; si
--   l'écriture échoue, rien n'est montré (contrôlé par le code, dans la
--   même transaction) ;
-- - le journal n'est **jamais modifié ni effacé**, même par un
--   administrateur : droits retirés au serveur et triggers (même modèle
--   que historique_statut_demande, migration 0010) ;
-- - aucune purge en attendant l'avis de la CIL (décision MD6,
--   docs/points-ouverts.md ; loi n° 001-2021/AN, article 25).
--
-- Chaque ligne désigne l'affectation qui a agi (qui, avec quel rôle, dans
-- quel établissement) : clé composite MATCH FULL vers affectation, comme
-- les décisions des agents (migration 0021).

CREATE TABLE journal_acces (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id          UUID NOT NULL REFERENCES patient (id),
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL,
    -- Liste fermée, étendue par migration.
    nature              TEXT NOT NULL CHECK (nature IN (
                            'scan', 'detail_contrat', 'recherche_identite',
                            'ouverture_prise_en_charge', 'prolongation_prise_en_charge',
                            'bris_de_glace', 'lecture_dossier', 'ecriture_dossier')),
    -- Le téléphone du patient a été montré (agent MUNASEB seulement).
    telephone_affiche   BOOLEAN NOT NULL DEFAULT false,
    -- Prise en charge sous laquelle le dossier a été lu ou écrit (clé
    -- étrangère ajoutée par la migration suivante, avec la table).
    prise_en_charge_id  UUID,
    date_acces          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT journal_acces_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);

-- Le patient lit son journal, le plus récent d'abord (curseur sur id).
CREATE INDEX journal_acces_patient_idx ON journal_acces (patient_id, id DESC);
-- Limite des scans par affectation dans une fenêtre glissante.
CREATE INDEX journal_acces_affectation_idx ON journal_acces (affectation_id, nature, date_acces);

-- Essais sans résultat : NIP inconnu ou mal formé, recherche par identité
-- sans résultat unique, code de soin faux. Rien de ce qui a été saisi n'est
-- gardé (ni NIP, ni nom, ni code) : seulement l'affectation, la nature, la
-- date, et le patient quand il est connu (code faux pour un NIP existant).
CREATE TABLE echec_acces_patient (
    id              UUID PRIMARY KEY DEFAULT uuidv7(),
    affectation_id  UUID NOT NULL,
    structure_id    UUID NOT NULL,
    role            TEXT NOT NULL,
    nature          TEXT NOT NULL CHECK (nature IN ('scan', 'recherche_identite', 'code_soin')),
    patient_id      UUID REFERENCES patient (id),
    date_echec      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT echec_acces_patient_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);

CREATE INDEX echec_acces_patient_affectation_idx
    ON echec_acces_patient (affectation_id, nature, date_echec);
CREATE INDEX echec_acces_patient_patient_idx
    ON echec_acces_patient (patient_id, nature, date_echec) WHERE patient_id IS NOT NULL;

CREATE FUNCTION acces_patient_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% : les lignes ne sont jamais modifiees ni supprimees (%)', TG_TABLE_NAME, TG_OP;
END;
$$;

CREATE TRIGGER journal_acces_sans_modification
    BEFORE UPDATE OR DELETE ON journal_acces
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER journal_acces_sans_troncature
    BEFORE TRUNCATE ON journal_acces
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER echec_acces_patient_sans_modification
    BEFORE UPDATE OR DELETE ON echec_acces_patient
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER echec_acces_patient_sans_troncature
    BEFORE TRUNCATE ON echec_acces_patient
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();

-- Droits (suite de P1, migration 0026) : le serveur lit et ajoute, jamais
-- ne modifie ni n'efface.
REVOKE UPDATE, DELETE ON journal_acces, echec_acces_patient FROM laaficare_serveur;
