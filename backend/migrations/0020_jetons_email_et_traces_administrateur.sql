-- Étape 3.6.5 (décisions B1 à B10 du 2026-10-18) : jetons envoyés par
-- email, motif de fermeture des sessions de `desactiver_totp`, traces de la
-- création et de la reprise des administrateurs.

-- ---------------------------------------------------------------------
-- Jetons envoyés par email (B1). Objet `verification_email` (R0) ; la 3.12
-- ajoutera `reinitialisation_mot_de_passe`. 32 octets aléatoires, seule
-- l'empreinte SHA-256 est gardée : au-delà de 112 bits, NIST SP 800-63B
-- rév. 4 §3.1.2 admet un hachage simple (décision R1). Durée : 24 h pour la
-- vérification (NIST SP 800-63A rév. 4 §3.8), calculée par PostgreSQL à
-- l'écriture depuis une constante du code (une seule horloge).
-- ---------------------------------------------------------------------
CREATE TABLE jeton_email (
    id             UUID PRIMARY KEY DEFAULT uuidv7(),
    compte_id      UUID NOT NULL,
    compte_type    TEXT NOT NULL,
    objet          TEXT NOT NULL CHECK (objet IN ('verification_email')),
    empreinte      BYTEA NOT NULL UNIQUE CHECK (octet_length(empreinte) = 32),
    -- Adresse à laquelle le jeton a été envoyé : il ne vérifie qu'elle. Un
    -- jeton envoyé à l'ancienne adresse ne vérifie jamais la nouvelle (R0).
    email_vise     TEXT NOT NULL CHECK (email_vise = btrim(email_vise) AND email_vise <> ''),
    date_creation  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expire_le      TIMESTAMPTZ NOT NULL,
    utilise_le     TIMESTAMPTZ,
    -- Remplacé par un jeton plus récent du même compte et du même objet
    -- (décision du porteur, 2026-10-18). Gardé, pas effacé : trace.
    annule_le      TIMESTAMPTZ,

    -- Seuls les comptes professionnel et administrateur ont un email
    -- vérifié (R0) : la base refuse un jeton pour un compte patient.
    CONSTRAINT jeton_email_compte
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte),
    CONSTRAINT jeton_email_type_compte
        CHECK (compte_type IN ('professionnel', 'administrateur_laaficare')),
    CONSTRAINT jeton_email_expiration CHECK (expire_le > date_creation),
    CONSTRAINT jeton_email_utilisation CHECK (utilise_le IS NULL OR utilise_le >= date_creation),
    CONSTRAINT jeton_email_annulation CHECK (annule_le IS NULL OR annule_le >= date_creation),
    -- Un jeton n'est jamais à la fois utilisé et annulé.
    CONSTRAINT jeton_email_utilise_ou_annule CHECK (utilise_le IS NULL OR annule_le IS NULL)
);

-- Au plus un jeton de vérification en attente par compte : l'émission d'un
-- nouveau jeton annule d'abord les précédents, et la base refuse un second
-- jeton en attente si le code l'oubliait. Limité à `verification_email` :
-- la réinitialisation (3.12) garde jusqu'à 3 liens en attente (R3, R5).
CREATE UNIQUE INDEX jeton_email_une_verification_en_attente
    ON jeton_email (compte_id)
    WHERE objet = 'verification_email' AND utilise_le IS NULL AND annule_le IS NULL;

-- ---------------------------------------------------------------------
-- Motif de fermeture des sessions par `desactiver_totp` (même étape).
-- Même méthode que la migration 0017 : la contrainte, au même nom, reprend
-- toute l'ancienne liste, aucune ligne existante ne peut la violer.
-- ---------------------------------------------------------------------
ALTER TABLE session DROP CONSTRAINT session_motif_revocation_check;
ALTER TABLE session ADD CONSTRAINT session_motif_revocation_check
    CHECK (motif_revocation IN (
        'deconnexion', 'deconnexion_partout',
        'reinitialisation_mot_de_passe', 'rejeu_detecte',
        'changement_affectation', 'desactivation_compte',
        'email_non_verifie', 'desactivation_totp'));

-- ---------------------------------------------------------------------
-- Trace de la création de chaque administrateur (décision du porteur,
-- 2026-10-18) : date, compte créé, administrateur qui l'a créé. Ce dernier
-- est vide pour le premier administrateur seulement, créé par l'outil
-- local `creer_premier_administrateur`.
-- ---------------------------------------------------------------------
CREATE TABLE creation_administrateur (
    compte_id                   UUID PRIMARY KEY,
    compte_type                 TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (compte_type = 'administrateur_laaficare'),
    cree_par_compte_id          UUID,
    cree_par_type_compte        TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (cree_par_type_compte = 'administrateur_laaficare'),
    date_creation               TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT creation_administrateur_compte
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte),
    -- MATCH SIMPLE (par défaut) : la clé n'est pas vérifiée quand
    -- `cree_par_compte_id` est vide (doc PostgreSQL 18, CREATE TABLE).
    CONSTRAINT creation_administrateur_cree_par
        FOREIGN KEY (cree_par_compte_id, cree_par_type_compte) REFERENCES compte (id, type_compte),
    CONSTRAINT creation_administrateur_pas_soi_meme
        CHECK (cree_par_compte_id IS NULL OR cree_par_compte_id <> compte_id)
);

-- Un seul premier administrateur, jamais deux : deux outils lancés en même
-- temps ne peuvent pas en créer deux (le second INSERT est refusé).
CREATE UNIQUE INDEX creation_administrateur_un_seul_premier
    ON creation_administrateur ((true)) WHERE cree_par_compte_id IS NULL;

-- ---------------------------------------------------------------------
-- Trace de la reprise d'un administrateur qui n'a jamais eu de TOTP actif
-- (outil interrompu, ou fenêtre de réenrôlement expirée ; décision du
-- porteur, 2026-10-18) : date, administrateur repris, administrateur qui a
-- repris. Ce dernier est vide pour la reprise du premier administrateur par
-- lui-même, quand aucun autre n'existe (décision B6).
-- ---------------------------------------------------------------------
CREATE TABLE reprise_administrateur (
    id                          UUID PRIMARY KEY DEFAULT uuidv7(),
    compte_id                   UUID NOT NULL,
    compte_type                 TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (compte_type = 'administrateur_laaficare'),
    repris_par_compte_id        UUID,
    repris_par_type_compte      TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (repris_par_type_compte = 'administrateur_laaficare'),
    date_reprise                TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT reprise_administrateur_compte
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte),
    CONSTRAINT reprise_administrateur_repris_par
        FOREIGN KEY (repris_par_compte_id, repris_par_type_compte) REFERENCES compte (id, type_compte),
    CONSTRAINT reprise_administrateur_pas_soi_meme
        CHECK (repris_par_compte_id IS NULL OR repris_par_compte_id <> compte_id)
);

CREATE INDEX reprise_administrateur_compte_idx
    ON reprise_administrateur (compte_id, date_reprise);

-- Les deux traces ne sont jamais modifiées ni supprimées : même mécanisme
-- que les migrations 0010, 0012 et 0013 (doc PostgreSQL 18, CREATE TRIGGER :
-- TRUNCATE ne déclenche pas les triggers ligne par ligne, d'où un second
-- trigger FOR EACH STATEMENT). Protège contre une erreur, pas contre un
-- administrateur de la base (section 12).
CREATE FUNCTION trace_administrateur_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% : les lignes ne sont jamais modifiées ni supprimées (%)', TG_TABLE_NAME, TG_OP;
END;
$$;

CREATE TRIGGER creation_administrateur_sans_modification
    BEFORE UPDATE OR DELETE ON creation_administrateur
    FOR EACH ROW EXECUTE FUNCTION trace_administrateur_non_modifiable();

CREATE TRIGGER creation_administrateur_sans_troncature
    BEFORE TRUNCATE ON creation_administrateur
    FOR EACH STATEMENT EXECUTE FUNCTION trace_administrateur_non_modifiable();

CREATE TRIGGER reprise_administrateur_sans_modification
    BEFORE UPDATE OR DELETE ON reprise_administrateur
    FOR EACH ROW EXECUTE FUNCTION trace_administrateur_non_modifiable();

CREATE TRIGGER reprise_administrateur_sans_troncature
    BEFORE TRUNCATE ON reprise_administrateur
    FOR EACH STATEMENT EXECUTE FUNCTION trace_administrateur_non_modifiable();
