-- Migration 0028 : Indexation exhaustive des cles etrangeres, index composites
-- de performance et polyfill de compatibilite cloud PostgreSQL 15 a 18 (RFC 9562).
--
-- Exigences d'architecture DBA :
-- 1. Eliminer les sequential scans sur les tables filles lors des verifications de cles
--    etrangeres, des jointures et des suppressions en cascade (ON DELETE CASCADE).
-- 2. Couvrir les filtres composites frequents (compte_id + objet dans jeton_email).
-- 3. Assurer la compatibilite universelle d'uuidv7() sur les bases managees cloud
--    (AWS RDS, GCP Cloud SQL, Scaleway) ou PostgreSQL 18 n'est pas encore disponible.

-- ---------------------------------------------------------------------
-- 1. Polyfill uuidv7() si absent (PostgreSQL 15, 16, 17)
-- ---------------------------------------------------------------------
DO $$
BEGIN
    IF to_regprocedure('pg_catalog.uuidv7()') IS NULL AND to_regprocedure('public.uuidv7()') IS NULL THEN
        CREATE OR REPLACE FUNCTION public.uuidv7() RETURNS uuid AS $func$
        DECLARE
            unix_ts_ms bytea;
            rand_bytes bytea;
        BEGIN
            unix_ts_ms := substring(int8send(floor(extract(epoch FROM clock_timestamp()) * 1000)::bigint) from 3 for 6);
            rand_bytes := gen_random_uuid()::bytea;
            RETURN encode(
                unix_ts_ms ||
                set_byte(substring(rand_bytes from 7 for 2), 0, (get_byte(substring(rand_bytes from 7 for 2), 0) & 15) | 112) ||
                set_byte(substring(rand_bytes from 9 for 8), 0, (get_byte(substring(rand_bytes from 9 for 8), 0) & 63) | 128),
                'hex'
            )::uuid;
        END;
        $func$ LANGUAGE plpgsql VOLATILE;

        GRANT EXECUTE ON FUNCTION public.uuidv7() TO laaficare_serveur;
        GRANT EXECUTE ON FUNCTION public.uuidv7() TO laaficare_outils;
    END IF;
END;
$$;

-- ---------------------------------------------------------------------
-- 2. Indexation des cles etrangeres et completion des index partiels
-- ---------------------------------------------------------------------

-- Structure : index complet sur le createur (l'index existant ne couvrait que les statuts non valides)
CREATE INDEX IF NOT EXISTS structure_createur_compte_id_idx
    ON structure (createur_compte_id);

-- Piece justificative : index complet sur la structure (l'index existant filtrait WHERE active)
CREATE INDEX IF NOT EXISTS piece_justificative_structure_id_idx
    ON piece_justificative (structure_id);

-- Code de secours : index complet sur le second facteur pour accelerer ON DELETE CASCADE
-- (l'index existant code_secours_disponibles ne couvrait que WHERE utilise_le IS NULL)
CREATE INDEX IF NOT EXISTS code_secours_second_facteur_id_idx
    ON code_secours (second_facteur_id);

-- Jeton email : index composite pour reinitialisation de mot de passe et verification
CREATE INDEX IF NOT EXISTS jeton_email_compte_objet_idx
    ON jeton_email (compte_id, objet);

-- Autorisation validee : cle etrangere vers structure
CREATE INDEX IF NOT EXISTS autorisation_validee_structure_id_idx
    ON autorisation_validee (structure_id);

-- Decision de structure : cle etrangere vers l'administrateur
CREATE INDEX IF NOT EXISTS decision_structure_administrateur_compte_id_idx
    ON decision_structure (administrateur_compte_id);

-- Affectation : cle etrangere vers compte (statuts inactifs non couverts par l'index unique partiel)
CREATE INDEX IF NOT EXISTS affectation_compte_id_idx
    ON affectation (compte_id);

-- Affectation : auteur de la desactivation
CREATE INDEX IF NOT EXISTS affectation_desactivee_par_compte_id_idx
    ON affectation (desactivee_par_compte_id)
    WHERE desactivee_par_compte_id IS NOT NULL;

-- Invitation : auteur de l'invitation
CREATE INDEX IF NOT EXISTS invitation_invite_par_compte_id_idx
    ON invitation (invite_par_compte_id);

-- Historique statut demande : cle etrangere composite vers affectation
CREATE INDEX IF NOT EXISTS historique_statut_demande_agent_affectation_idx
    ON historique_statut_demande (agent_affectation_id, agent_structure_id)
    WHERE agent_affectation_id IS NOT NULL;

-- Intervention compte : cle etrangere vers affectation et administrateur
CREATE INDEX IF NOT EXISTS intervention_compte_affectation_id_idx
    ON intervention_compte (affectation_id)
    WHERE affectation_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS intervention_compte_administrateur_compte_id_idx
    ON intervention_compte (administrateur_compte_id);
