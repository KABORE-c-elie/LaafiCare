-- Stock d'une pharmacie (étape 4 ; CLAUDE.md §15 ; CDC BF-04). Le stock
-- n'est jamais une colonne qu'on écrase : c'est la **somme d'un journal de
-- mouvements** jamais modifiés ni supprimés. Une erreur se corrige par une
-- écriture d'annulation tracée (même principe que les autres traces).
--
-- Contenu :
-- - `lot_stock` : chaque entrée en stock crée un lot (BF-04 : fournisseur,
--   numéro de lot, date d'expiration, prix d'achat) ;
-- - `vente` et `mouvement_stock` : entrées, ventes, pertes, écarts
--   d'inventaire, annulations ; chaque ligne garde le prix d'achat du lot
--   et, pour une vente, le prix de vente, **figés** (besoin du porteur du
--   2026-11-16, bénéfice net) ;
-- - `prix_vente_pharmacie` : prix de vente par pharmacie (D7), une version
--   par changement ;
-- - `seuil_stock` et `reglage_pharmacie` : seuils minimal et maximal
--   (BF-04), délai d'alerte de péremption (D9, sans valeur par défaut) ;
-- - `operation_idempotente` : clés d'idempotence (draft IETF « The
--   Idempotency-Key HTTP Header Field ») : une même requête renvoyée après
--   une coupure ne crée jamais deux lignes.
--
-- Toute écriture est faite par une affectation `pharmacien` ou
-- `responsable` **de la même pharmacie** : clé étrangère composite MATCH
-- FULL vers `affectation (id, structure_id, role)` (comme J8, migration
-- 0021) et CHECK sur la structure.

-- ---------------------------------------------------------------------
-- Lots
-- ---------------------------------------------------------------------

CREATE TABLE lot_stock (
    id                        UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id              UUID NOT NULL REFERENCES structure (id),
    medicament_id             UUID NOT NULL REFERENCES medicament (id),
    numero_lot                TEXT NOT NULL CHECK (numero_lot = btrim(numero_lot) AND char_length(numero_lot) BETWEEN 1 AND 100
                                                   AND numero_lot !~ '[[:cntrl:]]'),
    date_expiration           DATE NOT NULL,
    -- FCFA entiers (pas de sous-unité en circulation, §12) ; 0 permis
    -- (don, échantillon).
    prix_achat_unitaire_fcfa  INTEGER NOT NULL CHECK (prix_achat_unitaire_fcfa >= 0),
    fournisseur               TEXT NOT NULL CHECK (fournisseur = btrim(fournisseur) AND char_length(fournisseur) BETWEEN 1 AND 200
                                                   AND fournisseur !~ '[[:cntrl:]]'),
    date_creation             TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Cible des clés étrangères composites : un mouvement ne vise que les
    -- lots de sa propre pharmacie.
    CONSTRAINT lot_stock_id_structure UNIQUE (id, structure_id)
);

CREATE INDEX lot_stock_structure_medicament_idx ON lot_stock (structure_id, medicament_id, date_expiration);

-- ---------------------------------------------------------------------
-- Ventes (en-tête ; les lignes sont des mouvements)
-- ---------------------------------------------------------------------

CREATE TABLE vente (
    id                     UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id           UUID NOT NULL REFERENCES structure (id),
    -- D10 : la décision reste au pharmacien ; la case est gardée.
    sur_ordonnance_papier  BOOLEAN NOT NULL,
    auteur_affectation_id  UUID NOT NULL,
    auteur_structure_id    UUID NOT NULL,
    auteur_role            TEXT NOT NULL CHECK (auteur_role IN ('pharmacien', 'responsable')),
    date_vente             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT vente_auteur FOREIGN KEY (auteur_affectation_id, auteur_structure_id, auteur_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT vente_par_sa_pharmacie CHECK (auteur_structure_id = structure_id),
    CONSTRAINT vente_id_structure UNIQUE (id, structure_id)
);

CREATE INDEX vente_structure_date_idx ON vente (structure_id, date_vente);

-- ---------------------------------------------------------------------
-- Journal des mouvements
-- ---------------------------------------------------------------------

CREATE TABLE mouvement_stock (
    id                        UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id              UUID NOT NULL,
    lot_id                    UUID NOT NULL,
    nature                    TEXT NOT NULL CHECK (nature IN ('entree', 'vente', 'perte', 'inventaire', 'annulation')),
    -- Positive : le stock augmente ; négative : il baisse. Jamais 0.
    quantite                  INTEGER NOT NULL CHECK (quantite <> 0),
    -- Prix d'achat du lot, recopié et figé sur chaque mouvement : il
    -- valorise le coût des ventes et les pertes.
    prix_achat_unitaire_fcfa  INTEGER NOT NULL CHECK (prix_achat_unitaire_fcfa >= 0),
    -- Pour une vente (et son annulation) seulement : prix de vente figé.
    prix_vente_unitaire_fcfa  INTEGER CHECK (prix_vente_unitaire_fcfa >= 0),
    vente_id                  UUID,
    -- Pour un écart d'inventaire seulement : les lignes d'un même
    -- inventaire partagent cet identifiant.
    inventaire_id             UUID,
    motif_perte               TEXT CHECK (motif_perte IN ('peremption', 'casse', 'autre')),
    -- Mouvement annulé (pour une annulation seulement) ; une seule
    -- annulation par mouvement.
    annule_mouvement_id       UUID UNIQUE REFERENCES mouvement_stock (id),
    -- Libellé d'une perte, motif d'une annulation : garde-fou de longueur.
    commentaire               TEXT CHECK (commentaire IS NULL OR (btrim(commentaire) <> '' AND char_length(commentaire) <= 1000)),
    auteur_affectation_id     UUID NOT NULL,
    auteur_structure_id       UUID NOT NULL,
    auteur_role               TEXT NOT NULL CHECK (auteur_role IN ('pharmacien', 'responsable')),
    date_mouvement            TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT mouvement_lot_de_sa_pharmacie FOREIGN KEY (lot_id, structure_id) REFERENCES lot_stock (id, structure_id),
    CONSTRAINT mouvement_vente_de_sa_pharmacie FOREIGN KEY (vente_id, structure_id) REFERENCES vente (id, structure_id),
    CONSTRAINT mouvement_auteur FOREIGN KEY (auteur_affectation_id, auteur_structure_id, auteur_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT mouvement_par_sa_pharmacie CHECK (auteur_structure_id = structure_id),
    -- Sens de chaque nature.
    CONSTRAINT mouvement_sens CHECK (
        (nature = 'entree' AND quantite > 0)
        OR (nature IN ('vente', 'perte') AND quantite < 0)
        OR nature IN ('inventaire', 'annulation')
    ),
    CONSTRAINT mouvement_vente_complete CHECK (
        (nature = 'vente') <= (vente_id IS NOT NULL AND prix_vente_unitaire_fcfa IS NOT NULL)
    ),
    CONSTRAINT mouvement_prix_vente_seulement_vente CHECK (
        (prix_vente_unitaire_fcfa IS NULL AND vente_id IS NULL) OR nature IN ('vente', 'annulation')
    ),
    CONSTRAINT mouvement_motif_perte CHECK ((nature = 'perte') = (motif_perte IS NOT NULL)),
    CONSTRAINT mouvement_inventaire CHECK ((nature = 'inventaire') = (inventaire_id IS NOT NULL)),
    CONSTRAINT mouvement_annulation CHECK ((nature = 'annulation') = (annule_mouvement_id IS NOT NULL))
);

CREATE INDEX mouvement_stock_lot_idx ON mouvement_stock (lot_id);
CREATE INDEX mouvement_stock_structure_date_idx ON mouvement_stock (structure_id, date_mouvement);
CREATE INDEX mouvement_stock_vente_idx ON mouvement_stock (vente_id) WHERE vente_id IS NOT NULL;
CREATE INDEX mouvement_stock_inventaire_idx ON mouvement_stock (inventaire_id) WHERE inventaire_id IS NOT NULL;

-- Le stock d'un lot n'est jamais négatif. Le code verrouille le lot avant
-- d'écrire (deux sorties simultanées passent l'une après l'autre) ; ce
-- trigger est la garantie de la base.
CREATE FUNCTION mouvement_stock_jamais_negatif() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF (SELECT COALESCE(sum(quantite), 0) FROM mouvement_stock WHERE lot_id = NEW.lot_id) < 0 THEN
        RAISE EXCEPTION 'mouvement_stock : le stock du lot ne peut pas devenir negatif'
            USING ERRCODE = 'check_violation', CONSTRAINT = 'mouvement_stock_jamais_negatif';
    END IF;
    RETURN NULL;
END;
$$;

CREATE TRIGGER mouvement_stock_jamais_negatif AFTER INSERT ON mouvement_stock
    FOR EACH ROW EXECUTE FUNCTION mouvement_stock_jamais_negatif();

-- ---------------------------------------------------------------------
-- Prix de vente, seuils, réglages
-- ---------------------------------------------------------------------

-- Une version par changement ; le prix en vigueur est la dernière.
CREATE TABLE prix_vente_pharmacie (
    id                     UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id           UUID NOT NULL REFERENCES structure (id),
    medicament_id          UUID NOT NULL REFERENCES medicament (id),
    prix_unitaire_fcfa     INTEGER NOT NULL CHECK (prix_unitaire_fcfa >= 0),
    auteur_affectation_id  UUID NOT NULL,
    auteur_structure_id    UUID NOT NULL,
    auteur_role            TEXT NOT NULL CHECK (auteur_role = 'responsable'),
    date_effet             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT prix_vente_auteur FOREIGN KEY (auteur_affectation_id, auteur_structure_id, auteur_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT prix_vente_par_sa_pharmacie CHECK (auteur_structure_id = structure_id)
);

CREATE INDEX prix_vente_pharmacie_idx ON prix_vente_pharmacie (structure_id, medicament_id, date_effet DESC, id DESC);

-- Réglage de gestion, modifiable (ce n'est ni de l'argent ni une trace) :
-- l'auteur et la date du dernier changement sont gardés.
CREATE TABLE seuil_stock (
    structure_id           UUID NOT NULL REFERENCES structure (id),
    medicament_id          UUID NOT NULL REFERENCES medicament (id),
    seuil_min              INTEGER NOT NULL CHECK (seuil_min >= 0),
    seuil_max              INTEGER CHECK (seuil_max IS NULL OR seuil_max >= seuil_min),
    auteur_affectation_id  UUID NOT NULL,
    auteur_structure_id    UUID NOT NULL,
    auteur_role            TEXT NOT NULL CHECK (auteur_role IN ('pharmacien', 'responsable')),
    modifie_le             TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (structure_id, medicament_id),
    CONSTRAINT seuil_auteur FOREIGN KEY (auteur_affectation_id, auteur_structure_id, auteur_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT seuil_par_sa_pharmacie CHECK (auteur_structure_id = structure_id)
);

-- D9 : délai d'alerte avant péremption, sans valeur par défaut. Borne :
-- garde-fou technique (un an).
CREATE TABLE reglage_pharmacie (
    structure_id                    UUID PRIMARY KEY REFERENCES structure (id),
    delai_alerte_peremption_jours   SMALLINT CHECK (delai_alerte_peremption_jours BETWEEN 1 AND 365),
    auteur_affectation_id           UUID NOT NULL,
    auteur_structure_id             UUID NOT NULL,
    auteur_role                     TEXT NOT NULL CHECK (auteur_role = 'responsable'),
    modifie_le                      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT reglage_auteur FOREIGN KEY (auteur_affectation_id, auteur_structure_id, auteur_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT reglage_par_sa_pharmacie CHECK (auteur_structure_id = structure_id)
);

-- ---------------------------------------------------------------------
-- Clés d'idempotence
-- ---------------------------------------------------------------------

-- Une clé par pharmacie (le draft IETF laisse le serveur fixer la portée
-- d'une clé ; une clé produite par l'application est unique en pratique,
-- un UUID v4 par exemple). `empreinte` : SHA-256 de la requête, pour
-- refuser une clé réutilisée avec une autre requête (draft, §2.2 et §2.7).
CREATE TABLE operation_idempotente (
    structure_id   UUID NOT NULL REFERENCES structure (id),
    cle            TEXT NOT NULL CHECK (cle ~ '^[\x21-\x7E]{1,255}$'),
    operation      TEXT NOT NULL CHECK (operation IN ('entree', 'vente', 'perte', 'inventaire', 'annulation_mouvement',
                                                       'annulation_vente', 'depense', 'annulation_depense')),
    empreinte      BYTEA NOT NULL CHECK (octet_length(empreinte) = 32),
    -- Identifiant de ce qui a été créé (lot, vente, mouvement, inventaire,
    -- dépense) : la requête rejouée renvoie la même chose.
    resultat_id    UUID NOT NULL,
    date_creation  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (structure_id, cle)
);

-- ---------------------------------------------------------------------
-- Rien ne se modifie ni ne s'efface (sauf les deux tables de réglage)
-- ---------------------------------------------------------------------

CREATE FUNCTION stock_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '%: les lignes ne sont jamais modifiees ni supprimees (%)', TG_TABLE_NAME, TG_OP;
END;
$$;

CREATE TRIGGER lot_stock_sans_modification BEFORE UPDATE OR DELETE ON lot_stock
    FOR EACH ROW EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER lot_stock_sans_troncature BEFORE TRUNCATE ON lot_stock
    FOR EACH STATEMENT EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER vente_sans_modification BEFORE UPDATE OR DELETE ON vente
    FOR EACH ROW EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER vente_sans_troncature BEFORE TRUNCATE ON vente
    FOR EACH STATEMENT EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER mouvement_stock_sans_modification BEFORE UPDATE OR DELETE ON mouvement_stock
    FOR EACH ROW EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER mouvement_stock_sans_troncature BEFORE TRUNCATE ON mouvement_stock
    FOR EACH STATEMENT EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER prix_vente_sans_modification BEFORE UPDATE OR DELETE ON prix_vente_pharmacie
    FOR EACH ROW EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER prix_vente_sans_troncature BEFORE TRUNCATE ON prix_vente_pharmacie
    FOR EACH STATEMENT EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER operation_idempotente_sans_modification BEFORE UPDATE OR DELETE ON operation_idempotente
    FOR EACH ROW EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER operation_idempotente_sans_troncature BEFORE TRUNCATE ON operation_idempotente
    FOR EACH STATEMENT EXECUTE FUNCTION stock_non_modifiable();

-- Droits (suite de P1, migration 0026) : le serveur lit et ajoute ; il ne
-- modifie ni n'efface les traces. Seuils et réglages restent modifiables.
REVOKE UPDATE, DELETE ON lot_stock, vente, mouvement_stock, prix_vente_pharmacie, operation_idempotente
    FROM laaficare_serveur;
REVOKE DELETE ON seuil_stock, reglage_pharmacie FROM laaficare_serveur;
-- Verrou des lots pendant une vente, une perte ou un inventaire
-- (`SELECT ... FOR UPDATE`) : la doc PostgreSQL 18 (SELECT, « The Locking
-- Clause ») exige le droit UPDATE sur au moins une colonne de la table
-- verrouillée. On le donne sur la seule colonne `id`, qui reste figée par le
-- trigger `stock_non_modifiable` (toute modification est refusée).
GRANT UPDATE (id) ON lot_stock TO laaficare_serveur;
