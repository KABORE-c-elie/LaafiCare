-- Module médecin, étape 4 (décisions MD12 à MD17 du porteur du 2026-11-17,
-- CLAUDE.md §17) : allergies, antécédents, consultations et ordonnances.
--
-- Règles communes :
-- - chaque acte est rattaché au patient, à la **prise en charge** sous
--   laquelle il est fait, et à l'**affectation** qui l'a fait (qui, avec
--   quel rôle, dans quel établissement ; CLAUDE.md §6). Clés composites :
--   le patient de l'acte est celui de la prise en charge, et l'établissement
--   de l'affectation est celui de la prise en charge ;
-- - rien ne se modifie ni ne s'efface (CDC, BF-02 : suppression interdite) :
--   une allergie se **retire** par une ligne de retrait (désactivation
--   réversible : la redéclarer crée une nouvelle ligne) ; une consultation se
--   corrige par un **ajout** (MD14) ;
-- - textes libres bornés (garde-fous techniques, aucune source n'en donne).

-- Cible des clés composites ci-dessous.
ALTER TABLE prise_en_charge ADD CONSTRAINT prise_en_charge_id_structure_patient
    UNIQUE (id, structure_id, patient_id);

-- ---------------------------------------------------------------------
-- Allergies (MD12) : texte libre, et la DCI quand c'est un médicament du
-- catalogue national (pour le contrôle des ordonnances, MD16).
-- ---------------------------------------------------------------------
CREATE TABLE allergie (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id          UUID NOT NULL,
    substance           TEXT NOT NULL CHECK (btrim(substance) <> '' AND char_length(substance) <= 200),
    dci_id              UUID REFERENCES dci (id),
    reaction            TEXT CHECK (reaction IS NULL OR (btrim(reaction) <> '' AND char_length(reaction) <= 1000)),
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role IN ('medecin', 'infirmier')),
    date_saisie         TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT allergie_prise_en_charge
        FOREIGN KEY (prise_en_charge_id, structure_id, patient_id)
        REFERENCES prise_en_charge (id, structure_id, patient_id),
    CONSTRAINT allergie_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);
CREATE INDEX allergie_patient_idx ON allergie (patient_id);

-- Retrait d'une allergie (erreur de saisie, allergie écartée) : par un
-- médecin, avec un motif ; une seule fois.
CREATE TABLE retrait_allergie (
    allergie_id         UUID PRIMARY KEY REFERENCES allergie (id),
    motif               TEXT NOT NULL CHECK (btrim(motif) <> '' AND char_length(motif) <= 1000),
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role = 'medecin'),
    date_retrait        TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT retrait_allergie_prise_en_charge
        FOREIGN KEY (prise_en_charge_id, structure_id) REFERENCES prise_en_charge (id, structure_id),
    CONSTRAINT retrait_allergie_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);

-- ---------------------------------------------------------------------
-- Antécédents. Types : liste fermée proposée (aucune source officielle
-- lue) ; `medicamenteux` est celui que voit la pharmacie.
-- ---------------------------------------------------------------------
CREATE TABLE antecedent (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id          UUID NOT NULL,
    type_antecedent     TEXT NOT NULL CHECK (type_antecedent IN ('medical', 'chirurgical', 'familial', 'medicamenteux')),
    description         TEXT NOT NULL CHECK (btrim(description) <> '' AND char_length(description) <= 2000),
    date_debut          DATE,
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role IN ('medecin', 'infirmier')),
    date_saisie         TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT antecedent_prise_en_charge
        FOREIGN KEY (prise_en_charge_id, structure_id, patient_id)
        REFERENCES prise_en_charge (id, structure_id, patient_id),
    CONSTRAINT antecedent_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);
CREATE INDEX antecedent_patient_idx ON antecedent (patient_id);

-- ---------------------------------------------------------------------
-- Consultations (médecin seulement) : motif, examen, diagnostic en texte
-- libre (MD13). Jamais modifiée : corrigée par un ajout (MD14).
-- ---------------------------------------------------------------------
CREATE TABLE consultation (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id          UUID NOT NULL,
    motif               TEXT NOT NULL CHECK (btrim(motif) <> '' AND char_length(motif) <= 1000),
    examen              TEXT CHECK (examen IS NULL OR (btrim(examen) <> '' AND char_length(examen) <= 5000)),
    diagnostic          TEXT CHECK (diagnostic IS NULL OR (btrim(diagnostic) <> '' AND char_length(diagnostic) <= 2000)),
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role = 'medecin'),
    date_consultation   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT consultation_prise_en_charge
        FOREIGN KEY (prise_en_charge_id, structure_id, patient_id)
        REFERENCES prise_en_charge (id, structure_id, patient_id),
    CONSTRAINT consultation_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT consultation_id_patient UNIQUE (id, patient_id)
);
CREATE INDEX consultation_patient_idx ON consultation (patient_id, id);

CREATE TABLE ajout_consultation (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    consultation_id     UUID NOT NULL REFERENCES consultation (id),
    texte               TEXT NOT NULL CHECK (btrim(texte) <> '' AND char_length(texte) <= 2000),
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role = 'medecin'),
    date_ajout          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT ajout_prise_en_charge
        FOREIGN KEY (prise_en_charge_id, structure_id) REFERENCES prise_en_charge (id, structure_id),
    CONSTRAINT ajout_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);
CREATE INDEX ajout_consultation_idx ON ajout_consultation (consultation_id);

-- ---------------------------------------------------------------------
-- Ordonnances (médecin seulement) et leurs lignes. Aucune durée de
-- validité imposée (MD17) : la date est toujours rendue.
-- ---------------------------------------------------------------------
CREATE TABLE ordonnance (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id          UUID NOT NULL,
    consultation_id     UUID,
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role = 'medecin'),
    date_emission       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT ordonnance_prise_en_charge
        FOREIGN KEY (prise_en_charge_id, structure_id, patient_id)
        REFERENCES prise_en_charge (id, structure_id, patient_id),
    CONSTRAINT ordonnance_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    -- La consultation, s'il y en a une, est celle du même patient.
    CONSTRAINT ordonnance_consultation
        FOREIGN KEY (consultation_id, patient_id) REFERENCES consultation (id, patient_id)
);
CREATE INDEX ordonnance_patient_idx ON ordonnance (patient_id, id);

-- Une ligne : un médicament du catalogue (MD15 : le contrôle d'allergie
-- par DCI est alors possible) **ou** un nom hors catalogue (marqué, aucun
-- contrôle possible). Posologie en texte libre (aucun format officiel lu).
CREATE TABLE ligne_ordonnance (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    ordonnance_id       UUID NOT NULL REFERENCES ordonnance (id),
    rang                SMALLINT NOT NULL CHECK (rang BETWEEN 1 AND 50),
    medicament_id       UUID REFERENCES medicament (id),
    denomination_libre  TEXT CHECK (denomination_libre IS NULL
                            OR (btrim(denomination_libre) <> '' AND char_length(denomination_libre) <= 200)),
    posologie           TEXT NOT NULL CHECK (btrim(posologie) <> '' AND char_length(posologie) <= 500),
    quantite            INTEGER NOT NULL CHECK (quantite BETWEEN 1 AND 10000),
    duree_jours         SMALLINT CHECK (duree_jours IS NULL OR duree_jours BETWEEN 1 AND 365),

    CONSTRAINT ligne_un_seul_medicament
        CHECK ((medicament_id IS NULL) <> (denomination_libre IS NULL)),
    CONSTRAINT ligne_rang_unique UNIQUE (ordonnance_id, rang)
);

-- Dépassement d'une alerte d'allergie (MD16) : le médecin prescrit malgré
-- l'alerte, avec un motif obligatoire, tracé ligne par ligne.
CREATE TABLE depassement_alerte_allergie (
    id               UUID PRIMARY KEY DEFAULT uuidv7(),
    ordonnance_id    UUID NOT NULL REFERENCES ordonnance (id),
    ligne_id         UUID NOT NULL REFERENCES ligne_ordonnance (id),
    allergie_id      UUID NOT NULL REFERENCES allergie (id),
    motif            TEXT NOT NULL CHECK (btrim(motif) <> '' AND char_length(motif) <= 1000),
    date_depassement TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT depassement_unique UNIQUE (ligne_id, allergie_id)
);
CREATE INDEX depassement_ordonnance_idx ON depassement_alerte_allergie (ordonnance_id);

-- ---------------------------------------------------------------------
-- Rien ne se modifie ni ne s'efface (fonction de la migration 0036).
-- ---------------------------------------------------------------------
CREATE TRIGGER allergie_sans_modification BEFORE UPDATE OR DELETE ON allergie
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER allergie_sans_troncature BEFORE TRUNCATE ON allergie
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER retrait_allergie_sans_modification BEFORE UPDATE OR DELETE ON retrait_allergie
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER retrait_allergie_sans_troncature BEFORE TRUNCATE ON retrait_allergie
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER antecedent_sans_modification BEFORE UPDATE OR DELETE ON antecedent
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER antecedent_sans_troncature BEFORE TRUNCATE ON antecedent
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER consultation_sans_modification BEFORE UPDATE OR DELETE ON consultation
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER consultation_sans_troncature BEFORE TRUNCATE ON consultation
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER ajout_consultation_sans_modification BEFORE UPDATE OR DELETE ON ajout_consultation
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER ajout_consultation_sans_troncature BEFORE TRUNCATE ON ajout_consultation
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER ordonnance_sans_modification BEFORE UPDATE OR DELETE ON ordonnance
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER ordonnance_sans_troncature BEFORE TRUNCATE ON ordonnance
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER ligne_ordonnance_sans_modification BEFORE UPDATE OR DELETE ON ligne_ordonnance
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER ligne_ordonnance_sans_troncature BEFORE TRUNCATE ON ligne_ordonnance
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER depassement_sans_modification BEFORE UPDATE OR DELETE ON depassement_alerte_allergie
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER depassement_sans_troncature BEFORE TRUNCATE ON depassement_alerte_allergie
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();

REVOKE UPDATE, DELETE ON allergie, retrait_allergie, antecedent, consultation, ajout_consultation,
    ordonnance, ligne_ordonnance, depassement_alerte_allergie FROM laaficare_serveur;
