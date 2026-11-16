-- Contrat MUNASEB tenu par les agents (étape 5 du CLAUDE.md, section 12 ;
-- « Modèle retenu pour l'étape 5 », docs/points-ouverts.md, accord du
-- porteur du 2026-10-22). Remplace le contrat posé à la main en SQL.
--
-- Trois ajouts :
-- 1. `parametre_munaseb` : cotisation, fiche, délai de grâce, plafond par
--    période et durée de la carence, en base, modifiables par les agents et
--    historisés (une ligne par version, jamais modifiée) ; la constante
--    Rust `PLAFOND_ANNUEL_FCFA` disparaît ;
-- 2. sur `periode_adhesion_munaseb` : numéro de carte (il change chaque
--    année, réponse 2 du porteur), antenne et niveau d'études (facultatifs),
--    plafond de la période, figé à sa création ;
-- 3. `paiement_cotisation_munaseb` : le paiement de chaque période
--    enregistrée par un agent (mode, montants, référence mobile money,
--    affectation de l'agent), jamais modifié.
--
-- Hors de cette migration, en attente de la MUNASEB : suspension d'un
-- contrat, ayants droit, tarifs par circuit (docs/points-ouverts.md).

-- ---------------------------------------------------------------------
-- 1. Paramètres
-- ---------------------------------------------------------------------

CREATE TABLE parametre_munaseb (
    id                    UUID PRIMARY KEY DEFAULT uuidv7(),
    -- Montants en FCFA entiers (pas de sous-unité en circulation, §12).
    cotisation_fcfa       INTEGER  NOT NULL CHECK (cotisation_fcfa >= 0),
    fiche_fcfa            INTEGER  NOT NULL CHECK (fiche_fcfa >= 0),
    -- Bornes : garde-fous techniques contre une faute de frappe, pas des
    -- règles de la MUNASEB (même principe que le motif de 1 000 caractères).
    -- 60 jours de grâce au plus ; une carence de 12 mois au plus, pour que
    -- le début d'une période reste avant sa fin (fin = adhésion + 12 mois).
    delai_grace_jours     SMALLINT NOT NULL CHECK (delai_grace_jours BETWEEN 0 AND 60),
    carence_mois          SMALLINT NOT NULL CHECK (carence_mois BETWEEN 0 AND 12),
    plafond_periode_fcfa  INTEGER  NOT NULL CHECK (plafond_periode_fcfa > 0),
    -- Auteur : l'affectation de l'agent (même clé que J8, migration 0021).
    -- Vide seulement pour les valeurs initiales posées ci-dessous.
    agent_affectation_id  UUID,
    agent_structure_id    UUID,
    agent_role            TEXT CHECK (agent_role IS NULL OR agent_role = 'agent_assurance_munaseb'),
    date_effet            TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT parametre_munaseb_agent
        FOREIGN KEY (agent_affectation_id, agent_structure_id, agent_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL
);

-- Une seule version sans auteur : celle de cette migration.
CREATE UNIQUE INDEX parametre_munaseb_une_seule_initiale
    ON parametre_munaseb ((true)) WHERE agent_affectation_id IS NULL;

CREATE INDEX parametre_munaseb_date_idx ON parametre_munaseb (date_effet DESC, id DESC);

-- Valeurs initiales, chacune avec sa source :
-- - cotisation 5 000 FCFA et fiche 100 FCFA : carte de membre MUNASEB 2024
--   (documents transmis par le porteur, 2026-10-22) ;
-- - délai de grâce de 10 jours : réponse 2 du porteur (2026-10-22), à
--   confirmer par la MUNASEB ;
-- - carence d'un mois : CLAUDE.md §12 et carte 2024 (date d'effet = date
--   d'adhésion + 1 mois) ;
-- - plafond de 100 000 FCFA par période : mémoire MUNASEB (l. 64, 569,
--   826), absent des documents de 2024, à confirmer.
INSERT INTO parametre_munaseb (cotisation_fcfa, fiche_fcfa, delai_grace_jours, carence_mois, plafond_periode_fcfa)
VALUES (5000, 100, 10, 1, 100000);

-- Lignes jamais modifiées ni supprimées : une modification est une
-- nouvelle ligne, l'historique est la suite des lignes (même mécanisme que
-- les autres traces ; doc PostgreSQL 18, CREATE TRIGGER : TRUNCATE ne
-- déclenche pas les triggers ligne par ligne).
CREATE FUNCTION parametre_munaseb_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'parametre_munaseb : les lignes ne sont jamais modifiees ni supprimees (%)', TG_OP;
END;
$$;

CREATE TRIGGER parametre_munaseb_sans_modification
    BEFORE UPDATE OR DELETE ON parametre_munaseb
    FOR EACH ROW EXECUTE FUNCTION parametre_munaseb_non_modifiable();

CREATE TRIGGER parametre_munaseb_sans_troncature
    BEFORE TRUNCATE ON parametre_munaseb
    FOR EACH STATEMENT EXECUTE FUNCTION parametre_munaseb_non_modifiable();

-- Plafond en vigueur : la dernière version. Sert de valeur par défaut au
-- plafond d'une période (ci-dessous) : doc PostgreSQL 18, CREATE TABLE,
-- DEFAULT n'admet pas de sous-requête, mais admet l'appel d'une fonction.
-- SECURITY INVOKER (par défaut) : le rôle qui insère doit pouvoir lire la
-- table, ce qui est le cas du serveur.
CREATE FUNCTION plafond_munaseb_en_vigueur() RETURNS INTEGER
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT plafond_periode_fcfa FROM parametre_munaseb ORDER BY date_effet DESC, id DESC LIMIT 1
$$;

-- ---------------------------------------------------------------------
-- 2. Périodes
-- ---------------------------------------------------------------------

-- Plafond figé à la création de la période (décision de l'étape 5) : un
-- changement du paramètre ne touche que les périodes suivantes, comme un
-- changement de tarif ne touche jamais une demande existante (§12). Les
-- périodes existantes reçoivent la valeur initiale (100 000), celle de
-- l'ancienne constante : PostgreSQL calcule la valeur par défaut pour
-- chaque ligne existante quand elle n'est pas constante.
ALTER TABLE periode_adhesion_munaseb
    ADD COLUMN plafond_fcfa INTEGER NOT NULL DEFAULT plafond_munaseb_en_vigueur()
        CONSTRAINT periode_plafond_positif CHECK (plafond_fcfa > 0),
    ADD COLUMN numero_carte  TEXT,
    ADD COLUMN antenne       TEXT,
    ADD COLUMN niveau_etudes TEXT;

-- Numéro de carte des périodes existantes : celui du contrat, posé sur la
-- première période de chaque contrat (la carte d'origine). Les périodes
-- suivantes d'avant cette migration n'ont pas de numéro propre ; le code
-- lit alors celui du contrat.
UPDATE periode_adhesion_munaseb p
SET numero_carte = c.numero_carte
FROM contrat_assurance_munaseb c
WHERE c.id = p.contrat_id
  AND p.id = (SELECT p2.id FROM periode_adhesion_munaseb p2
              WHERE p2.contrat_id = p.contrat_id
              ORDER BY p2.date_debut, p2.id LIMIT 1);

-- Un numéro de carte ne désigne qu'une période (réponse 2 du porteur : un
-- nouveau numéro chaque année). Les numéros des contrats sont déjà uniques
-- (migration 0006) et chaque contrat a au moins une période (0009) :
-- l'index couvre aussi les cartes d'origine.
CREATE UNIQUE INDEX periode_numero_carte_unique
    ON periode_adhesion_munaseb (numero_carte) WHERE numero_carte IS NOT NULL;

-- Obligatoire pour toute nouvelle période, sans toucher aux anciennes :
-- NOT VALID (doc PostgreSQL 18, ALTER TABLE) : la contrainte n'est pas
-- vérifiée sur les lignes existantes, mais l'est sur chaque insertion et
-- mise à jour. À ne jamais valider avec VALIDATE CONSTRAINT (anciennes
-- périodes sans numéro). Format : garde-fous techniques seulement (sans
-- espace autour, 1 à 50 caractères, ni retour à la ligne ni caractère de
-- contrôle) ; le format exact (« 0552/2024 » sur la carte de 2024) reste à
-- confirmer par la MUNASEB.
ALTER TABLE periode_adhesion_munaseb
    ADD CONSTRAINT periode_numero_carte_obligatoire CHECK (numero_carte IS NOT NULL) NOT VALID,
    ADD CONSTRAINT periode_numero_carte_format CHECK (
        numero_carte = btrim(numero_carte)
        AND char_length(numero_carte) BETWEEN 1 AND 50
        AND numero_carte !~ '[[:cntrl:]]'
    ) NOT VALID,
    ADD CONSTRAINT periode_antenne_format CHECK (
        antenne IS NULL OR (antenne = btrim(antenne) AND char_length(antenne) BETWEEN 1 AND 100
                            AND antenne !~ '[[:cntrl:]]')
    ),
    ADD CONSTRAINT periode_niveau_etudes_format CHECK (
        niveau_etudes IS NULL OR (niveau_etudes = btrim(niveau_etudes) AND char_length(niveau_etudes) BETWEEN 1 AND 100
                                  AND niveau_etudes !~ '[[:cntrl:]]')
    );

-- ---------------------------------------------------------------------
-- 3. Paiements de la cotisation
-- ---------------------------------------------------------------------

CREATE TABLE paiement_cotisation_munaseb (
    id                       UUID PRIMARY KEY DEFAULT uuidv7(),
    -- Un paiement par période : la période naît de son paiement.
    periode_id               UUID NOT NULL UNIQUE REFERENCES periode_adhesion_munaseb (id),
    -- Réponse 7 du porteur (2026-10-22) : espèces aujourd'hui, mobile money
    -- aussi ; aucune API d'opérateur avant le pilote.
    mode                     TEXT NOT NULL CHECK (mode IN ('especes', 'mobile_money')),
    -- Montants en vigueur au moment du paiement, figés (paramètres).
    montant_cotisation_fcfa  INTEGER NOT NULL CHECK (montant_cotisation_fcfa >= 0),
    montant_fiche_fcfa       INTEGER NOT NULL CHECK (montant_fiche_fcfa >= 0),
    -- Obligatoire pour le mobile money, absente pour les espèces.
    reference_transaction    TEXT,
    -- Date déclarée par l'agent (par exemple celle de la transaction mobile
    -- money), pour information seulement : la date qui sert au calcul
    -- (carence, délai de grâce) est `periode_adhesion_munaseb.date_paiement`,
    -- fixée par le système (§12). Jamais postérieure au jour de
    -- l'enregistrement, au Burkina (décision D9).
    date_declaree            DATE,
    agent_affectation_id     UUID NOT NULL,
    agent_structure_id       UUID NOT NULL,
    agent_role               TEXT NOT NULL CHECK (agent_role = 'agent_assurance_munaseb'),
    date_enregistrement      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT paiement_cotisation_agent
        FOREIGN KEY (agent_affectation_id, agent_structure_id, agent_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT paiement_reference_selon_mode
        CHECK ((mode = 'mobile_money') = (reference_transaction IS NOT NULL)),
    CONSTRAINT paiement_reference_format CHECK (
        reference_transaction IS NULL
        OR (reference_transaction = btrim(reference_transaction)
            AND char_length(reference_transaction) BETWEEN 1 AND 100
            AND reference_transaction !~ '[[:cntrl:]]')
    ),
    CONSTRAINT paiement_date_declaree_passee CHECK (
        date_declaree IS NULL OR date_declaree <= (date_enregistrement AT TIME ZONE 'Africa/Ouagadougou')::date
    )
);

-- Une transaction mobile money ne paie qu'une cotisation : une référence
-- reprise est une erreur de saisie ou une fraude.
CREATE UNIQUE INDEX paiement_reference_transaction_unique
    ON paiement_cotisation_munaseb (reference_transaction) WHERE reference_transaction IS NOT NULL;

CREATE FUNCTION paiement_cotisation_munaseb_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'paiement_cotisation_munaseb : les lignes ne sont jamais modifiees ni supprimees (%)', TG_OP;
END;
$$;

CREATE TRIGGER paiement_cotisation_munaseb_sans_modification
    BEFORE UPDATE OR DELETE ON paiement_cotisation_munaseb
    FOR EACH ROW EXECUTE FUNCTION paiement_cotisation_munaseb_non_modifiable();

CREATE TRIGGER paiement_cotisation_munaseb_sans_troncature
    BEFORE TRUNCATE ON paiement_cotisation_munaseb
    FOR EACH STATEMENT EXECUTE FUNCTION paiement_cotisation_munaseb_non_modifiable();

-- ---------------------------------------------------------------------
-- Droits (suite de P1, migration 0026)
-- ---------------------------------------------------------------------

-- Les droits par défaut donnent au serveur lecture et écriture sur les
-- deux nouvelles tables ; ce sont des traces : il ne garde que la lecture
-- et l'insertion.
REVOKE UPDATE, DELETE ON parametre_munaseb, paiement_cotisation_munaseb FROM laaficare_serveur;
