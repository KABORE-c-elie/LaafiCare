-- Historique des demandes MUNASEB rattaché à l'affectation de l'agent
-- (étape 3.9, décision J8 du 2026-10-08, D6 du 2026-10-18) : qui a fait la
-- transition, et dans quel établissement, avec quel rôle. Les actes de la
-- plateforme sont liés à une affectation, qui ne s'efface jamais (section
-- 14, Q1).
--
-- Les lignes existantes ne sont jamais modifiées (triggers de la migration
-- 0010) : elles gardent leur `agent_utilisateur_id`, qui ne sert plus que
-- pour elles. Les nouvelles transitions portent l'affectation.

-- Clé étrangère composite MATCH FULL (doc PostgreSQL 18, CREATE TABLE,
-- « MATCH FULL will not allow one column of a multicolumn foreign key to be
-- null unless all foreign key columns are null ») : les trois colonnes sont
-- toutes vides (création, anciennes lignes) ou toutes remplies, et désignent
-- alors une affectation existante, de cet établissement et de ce rôle. Cible :
-- la contrainte UNIQUE (id, structure_id, role) de la migration 0012.
ALTER TABLE historique_statut_demande
    ADD COLUMN agent_affectation_id UUID,
    ADD COLUMN agent_structure_id   UUID,
    ADD COLUMN agent_role           TEXT
        CONSTRAINT historique_agent_role_munaseb
        CHECK (agent_role IS NULL OR agent_role = 'agent_assurance_munaseb'),
    ADD CONSTRAINT historique_agent_affectation
        FOREIGN KEY (agent_affectation_id, agent_structure_id, agent_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL;

-- Ancienne règle (0010) : une transition a un `agent_utilisateur_id`. Les
-- nouvelles lignes n'en ont plus. Nouvelle écriture : une création n'a aucun
-- agent, ni ancien ni nouveau ; une transition en a un des deux. Les lignes
-- existantes la respectent (création sans agent, transition avec
-- `agent_utilisateur_id`) : elle est vérifiée sur toute la table.
ALTER TABLE historique_statut_demande DROP CONSTRAINT agent_si_transition;
ALTER TABLE historique_statut_demande ADD CONSTRAINT agent_si_transition
    CHECK ((statut_precedent IS NULL) = (agent_utilisateur_id IS NULL AND agent_affectation_id IS NULL));

-- Toute nouvelle transition porte une affectation. NOT VALID (doc
-- PostgreSQL 18, ALTER TABLE : la contrainte est appliquée aux lignes
-- insérées ou modifiées ensuite, sans parcourir les lignes existantes) : les
-- anciennes transitions, qui n'ont que `agent_utilisateur_id`, ne sont pas
-- contrôlées. Ne jamais lancer VALIDATE CONSTRAINT sur cette contrainte
-- (J8) : elle échouerait sur ces anciennes lignes.
ALTER TABLE historique_statut_demande ADD CONSTRAINT transition_par_affectation
    CHECK (statut_precedent IS NULL OR agent_affectation_id IS NOT NULL) NOT VALID;
