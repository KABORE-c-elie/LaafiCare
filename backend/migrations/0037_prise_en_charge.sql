-- Module médecin, étape 3 (décisions MD1, MD2, MD3 du porteur du
-- 2026-11-17, CLAUDE.md §17) : code de soin du patient, prise en charge
-- par un établissement, bris de glace et son contrôle.
--
-- Modèle d'accès retenu (MD1) : le dossier ne s'ouvre que par une **prise
-- en charge** ouverte par l'établissement qui soigne, avec le code de soin
-- du patient (moyen préféré) ou après vérification de sa CNIB (tracée), ou
-- par un **bris de glace** en urgence, tracé et contrôlé ensuite par le
-- responsable de l'établissement (MD2).

-- ---------------------------------------------------------------------
-- Code de soin : un TOTP à 6 chiffres (RFC 6238) propre au patient, que son
-- téléphone calcule sans réseau. Secret chiffré comme celui du second
-- facteur (migration 0013 : AES-256-GCM, nonce de 96 bits, version de clé).
-- Une ligne par compte patient ; une nouvelle génération remplace l'ancienne
-- (un secret dont personne ne se sert n'est pas gardé, comme Y1).
-- ---------------------------------------------------------------------
CREATE TABLE code_soin_patient (
    compte_id       UUID PRIMARY KEY,
    compte_type     TEXT NOT NULL DEFAULT 'patient' CHECK (compte_type = 'patient'),
    -- 20 octets de secret + 16 octets d'étiquette GCM.
    secret_chiffre  BYTEA NOT NULL CHECK (octet_length(secret_chiffre) = 36),
    nonce           BYTEA NOT NULL CHECK (octet_length(nonce) = 12),
    version_cle     SMALLINT NOT NULL,
    -- Dernier pas de temps accepté : un code ne sert qu'une fois (RFC 6238 :
    -- « MUST NOT accept the second attempt »).
    dernier_pas     BIGINT,
    date_creation   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT code_soin_patient_compte
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte)
);

-- ---------------------------------------------------------------------
-- Prise en charge : ce patient, cet établissement, pour une durée limitée
-- (24 h, MD3). Ouverte par une affectation de l'établissement (clé
-- composite : l'établissement est toujours celui de l'affectation).
-- ---------------------------------------------------------------------
CREATE TABLE prise_en_charge (
    id                 UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id         UUID NOT NULL REFERENCES patient (id),
    affectation_id     UUID NOT NULL,
    structure_id       UUID NOT NULL,
    role               TEXT NOT NULL,
    moyen              TEXT NOT NULL CHECK (moyen IN ('code_soin', 'cnib_verifiee', 'bris_de_glace')),
    -- Bris de glace seulement : catégorie (liste fermée proposée, sans
    -- source officielle) et texte libre, tous deux obligatoires (MD2).
    motif_categorie    TEXT CHECK (motif_categorie IN (
                           'patient_inconscient', 'urgence_vitale',
                           'patient_incapable_de_communiquer', 'autre')),
    motif              TEXT CHECK (motif IS NULL OR (btrim(motif) <> '' AND char_length(motif) <= 1000)),
    debut              TIMESTAMPTZ NOT NULL DEFAULT now(),
    fin                TIMESTAMPTZ NOT NULL,

    CONSTRAINT prise_en_charge_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT prise_en_charge_motif_si_bris_de_glace
        CHECK ((moyen = 'bris_de_glace') = (motif_categorie IS NOT NULL AND motif IS NOT NULL)),
    -- MD2 : le bris de glace est ouvert au médecin et à l'infirmier (sage-
    -- femme comprise), jamais au pharmacien ni à un autre rôle.
    CONSTRAINT prise_en_charge_bris_de_glace_soignant
        CHECK (moyen <> 'bris_de_glace' OR role IN ('medecin', 'infirmier')),
    CONSTRAINT prise_en_charge_duree CHECK (fin > debut),
    CONSTRAINT prise_en_charge_id_structure UNIQUE (id, structure_id)
);

CREATE INDEX prise_en_charge_patient_idx ON prise_en_charge (patient_id, structure_id);
CREATE INDEX prise_en_charge_structure_idx ON prise_en_charge (structure_id, id);

-- Prolongation pendant une hospitalisation (MD3), par un médecin ou un
-- infirmier du même établissement ; chacune tracée. La fin effective d'une
-- prise en charge est la plus tardive de sa fin et de ses prolongations.
CREATE TABLE prolongation_prise_en_charge (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    prise_en_charge_id  UUID NOT NULL,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role IN ('medecin', 'infirmier')),
    motif               TEXT NOT NULL CHECK (motif = 'hospitalisation'),
    nouvelle_fin        TIMESTAMPTZ NOT NULL,
    date_prolongation   TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT prolongation_de_son_etablissement
        FOREIGN KEY (prise_en_charge_id, structure_id) REFERENCES prise_en_charge (id, structure_id),
    CONSTRAINT prolongation_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT prolongation_apres CHECK (nouvelle_fin > date_prolongation)
);

CREATE INDEX prolongation_prise_en_charge_idx ON prolongation_prise_en_charge (prise_en_charge_id);

-- Contrôle d'un bris de glace par le responsable de l'établissement (MD2),
-- une seule fois.
CREATE TABLE controle_bris_de_glace (
    prise_en_charge_id  UUID PRIMARY KEY,
    affectation_id      UUID NOT NULL,
    structure_id        UUID NOT NULL,
    role                TEXT NOT NULL CHECK (role = 'responsable'),
    conclusion          TEXT NOT NULL CHECK (conclusion IN ('justifie', 'injustifie')),
    commentaire         TEXT CHECK (commentaire IS NULL OR (btrim(commentaire) <> '' AND char_length(commentaire) <= 1000)),
    date_controle       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT controle_de_son_etablissement
        FOREIGN KEY (prise_en_charge_id, structure_id) REFERENCES prise_en_charge (id, structure_id),
    CONSTRAINT controle_affectation
        FOREIGN KEY (affectation_id, structure_id, role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);

-- Seul un bris de glace se contrôle (un CHECK ne peut pas lire une autre
-- table : trigger).
CREATE FUNCTION controle_seulement_bris_de_glace() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM prise_en_charge
                   WHERE id = NEW.prise_en_charge_id AND moyen = 'bris_de_glace') THEN
        RAISE EXCEPTION 'controle_bris_de_glace : la prise en charge n''est pas un bris de glace';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER controle_seulement_bris_de_glace
    BEFORE INSERT ON controle_bris_de_glace
    FOR EACH ROW EXECUTE FUNCTION controle_seulement_bris_de_glace();

-- Traces jamais modifiées ni effacées (fonction de la migration 0036).
CREATE TRIGGER prise_en_charge_sans_modification
    BEFORE UPDATE OR DELETE ON prise_en_charge
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER prise_en_charge_sans_troncature
    BEFORE TRUNCATE ON prise_en_charge
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER prolongation_sans_modification
    BEFORE UPDATE OR DELETE ON prolongation_prise_en_charge
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER prolongation_sans_troncature
    BEFORE TRUNCATE ON prolongation_prise_en_charge
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER controle_sans_modification
    BEFORE UPDATE OR DELETE ON controle_bris_de_glace
    FOR EACH ROW EXECUTE FUNCTION acces_patient_non_modifiable();
CREATE TRIGGER controle_sans_troncature
    BEFORE TRUNCATE ON controle_bris_de_glace
    FOR EACH STATEMENT EXECUTE FUNCTION acces_patient_non_modifiable();

REVOKE UPDATE, DELETE ON prise_en_charge, prolongation_prise_en_charge, controle_bris_de_glace
    FROM laaficare_serveur;
-- Le code de soin se remplace (suppression puis insertion) et son dernier
-- pas se met à jour : droits ordinaires (0026).

-- Le journal désigne la prise en charge sous laquelle il a lu.
ALTER TABLE journal_acces ADD CONSTRAINT journal_acces_prise_en_charge
    FOREIGN KEY (prise_en_charge_id) REFERENCES prise_en_charge (id);
CREATE INDEX journal_acces_prise_en_charge_idx ON journal_acces (prise_en_charge_id)
    WHERE prise_en_charge_id IS NOT NULL;

-- Le patient est averti d'un bris de glace par une notification (MD2).
ALTER TABLE notification DROP CONSTRAINT notification_type_check;
ALTER TABLE notification ADD CONSTRAINT notification_type_check
    CHECK (type IN ('remboursement_statut', 'acces_urgence'));
