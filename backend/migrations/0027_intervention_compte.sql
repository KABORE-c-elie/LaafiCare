-- Interventions de l'équipe LaafiCare sur un compte compromis (lot « avant
-- le pilote », P2, décisions IC1 à IC8 du porteur, 2026-10-22 ; J11, DS10) :
-- retrait d'une affectation, blocage d'un compte, réactivation. Outil local
-- `intervenir_compte`, jamais une route HTTP.
--
-- Une ligne par intervention, jamais modifiée ni supprimée, comme les autres
-- traces (migrations 0010, 0012, 0013, 0020).

-- La clé étrangère composite plus bas (une affectation retirée appartient
-- bien au compte visé) s'appuie sur la contrainte `affectation_id_compte_unique`
-- (id, compte_id), créée par la migration 0015 pour les sessions.

CREATE TABLE intervention_compte (
    id                          UUID PRIMARY KEY DEFAULT uuidv7(),
    -- Compte visé : patient, professionnel ou administrateur.
    compte_id                   UUID NOT NULL REFERENCES compte (id),
    -- Seulement pour un retrait d'affectation, et alors obligatoire.
    affectation_id              UUID,
    action                      TEXT NOT NULL
                                CHECK (action IN ('retrait_affectation', 'blocage_compte', 'reactivation_compte')),
    administrateur_compte_id    UUID NOT NULL,
    administrateur_type_compte  TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (administrateur_type_compte = 'administrateur_laaficare'),
    -- Obligatoire, non vide, 1 000 caractères au plus (même garde-fou que
    -- le motif d'un rejet ou d'un refus).
    motif                       TEXT NOT NULL CHECK (btrim(motif) <> '' AND char_length(motif) <= 1000),
    -- IC4 : attestation obligatoire pour une réactivation seulement ; un
    -- blocage peut être urgent, sans la personne devant soi. Le numéro de
    -- la CNIB n'est jamais enregistré (même règle que Y2).
    cnib_verifiee               BOOLEAN NOT NULL DEFAULT false,
    date_intervention           TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT intervention_compte_administrateur
        FOREIGN KEY (administrateur_compte_id, administrateur_type_compte) REFERENCES compte (id, type_compte),
    CONSTRAINT intervention_compte_affectation_du_compte
        FOREIGN KEY (affectation_id, compte_id) REFERENCES affectation (id, compte_id),
    CONSTRAINT intervention_compte_affectation_si_retrait
        CHECK ((action = 'retrait_affectation') = (affectation_id IS NOT NULL)),
    CONSTRAINT intervention_compte_cnib_pour_reactivation
        CHECK (action <> 'reactivation_compte' OR cnib_verifiee)
);

CREATE INDEX intervention_compte_compte_idx ON intervention_compte (compte_id, date_intervention);

-- Jamais sur ses propres comptes (comme Y3-bis, migration 0013) : un CHECK
-- ne peut pas lire la table `compte`, d'où un trigger qui compare
-- l'identité des deux comptes.
CREATE FUNCTION intervention_compte_par_un_autre() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM compte cible
        JOIN compte administrateur ON administrateur.utilisateur_id = cible.utilisateur_id
        WHERE cible.id = NEW.compte_id
          AND administrateur.id = NEW.administrateur_compte_id
    ) THEN
        RAISE EXCEPTION 'intervention_compte : un administrateur n''intervient sur aucun de ses propres comptes';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER intervention_compte_autre_personne
    BEFORE INSERT ON intervention_compte
    FOR EACH ROW EXECUTE FUNCTION intervention_compte_par_un_autre();

-- Même mécanisme que les autres traces (doc PostgreSQL 18, CREATE TRIGGER :
-- TRUNCATE ne déclenche pas les triggers ligne par ligne, d'où un second
-- trigger FOR EACH STATEMENT).
CREATE FUNCTION intervention_compte_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'intervention_compte : les lignes ne sont jamais modifiees ni supprimees (%)', TG_OP;
END;
$$;

CREATE TRIGGER intervention_compte_sans_modification
    BEFORE UPDATE OR DELETE ON intervention_compte
    FOR EACH ROW EXECUTE FUNCTION intervention_compte_non_modifiable();

CREATE TRIGGER intervention_compte_sans_troncature
    BEFORE TRUNCATE ON intervention_compte
    FOR EACH STATEMENT EXECUTE FUNCTION intervention_compte_non_modifiable();

-- Droits (suite de P1, migration 0026) : les droits par défaut donnent au
-- serveur lecture et écriture sur cette nouvelle table ; il ne garde que la
-- lecture. Seul le rôle des outils y écrit.
REVOKE INSERT, UPDATE, DELETE ON intervention_compte FROM laaficare_serveur;
GRANT INSERT ON intervention_compte TO laaficare_outils;
