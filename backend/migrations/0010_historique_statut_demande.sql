-- Historique des statuts des demandes de remboursement MUNASEB (décision
-- H, 2026-09-25). Répond à l'objectif du mémoire MUNASEB : "une
-- traçabilité complète de toutes les opérations effectuées dans le
-- système" (l. 689). Une ligne par changement de statut, création
-- comprise ; les lignes ne sont jamais modifiées ni supprimées.
CREATE TABLE historique_statut_demande (
    id                   UUID PRIMARY KEY DEFAULT uuidv7(),

    -- Sans ON DELETE : une demande qui a un historique ne peut plus être
    -- supprimée.
    demande_id           UUID NOT NULL REFERENCES demande_remboursement_munaseb (id),

    -- NULL uniquement pour la création de la demande.
    statut_precedent     TEXT CHECK (statut_precedent IN ('en_attente', 'en_cours', 'valide', 'rejete', 'paye')),
    statut_nouveau       TEXT NOT NULL CHECK (statut_nouveau IN ('en_attente', 'en_cours', 'valide', 'rejete', 'paye')),

    -- Agent qui a fait la transition (utilisateur_id fourni par
    -- l'extracteur JWT).
    agent_utilisateur_id UUID REFERENCES utilisateur (id),
    date_changement      TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Création : aucun agent, l'acte vient du partenaire. Transition :
    -- toujours un agent.
    CONSTRAINT agent_si_transition CHECK ((statut_precedent IS NULL) = (agent_utilisateur_id IS NULL))
);

-- Lecture attendue : l'historique d'une demande, dans l'ordre.
CREATE INDEX historique_statut_demande_demande_idx
    ON historique_statut_demande (demande_id, date_changement);

-- Reprise : une ligne de création pour chaque demande existante. Aucune
-- transition n'existe encore dans le code, toutes les demandes sont donc
-- en_attente (sauf modification manuelle en SQL).
INSERT INTO historique_statut_demande (demande_id, statut_precedent, statut_nouveau, agent_utilisateur_id, date_changement)
SELECT id, NULL, 'en_attente', NULL, date_demande
FROM demande_remboursement_munaseb;

-- ---------------------------------------------------------------------
-- Lignes non modifiables. Doc PostgreSQL 18, CREATE TRIGGER : un TRUNCATE
-- ne déclenche pas les triggers de suppression ligne par ligne ; les
-- triggers sur TRUNCATE existent, "though only FOR EACH STATEMENT". D'où
-- deux triggers sur la même fonction.
--
-- Protège contre une erreur ou un bug, pas contre un administrateur, qui
-- peut désactiver un trigger. En production, l'application doit utiliser
-- un rôle PostgreSQL sans droits d'administration (section 12 du
-- CLAUDE.md).
-- ---------------------------------------------------------------------
CREATE FUNCTION historique_statut_demande_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'historique_statut_demande : les lignes ne sont jamais modifiées ni supprimées (%)', TG_OP;
END;
$$;

CREATE TRIGGER historique_statut_demande_sans_modification
    BEFORE UPDATE OR DELETE ON historique_statut_demande
    FOR EACH ROW EXECUTE FUNCTION historique_statut_demande_non_modifiable();

CREATE TRIGGER historique_statut_demande_sans_troncature
    BEFORE TRUNCATE ON historique_statut_demande
    FOR EACH STATEMENT EXECUTE FUNCTION historique_statut_demande_non_modifiable();
