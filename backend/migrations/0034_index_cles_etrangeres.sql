-- Index des clés étrangères et des listes paginées (remédiation de l'audit,
-- branche remediation-audit, migration 0028 renumérotée 0034 ; reprise et
-- adaptée le 2026-11-17).
--
-- Doc PostgreSQL 18, §5.5.5 (Foreign Keys) : la déclaration d'une clé
-- étrangère ne crée pas d'index sur les colonnes qui référencent ; la
-- suppression d'une ligne référencée oblige à chercher ses lignes filles,
-- d'où l'intérêt d'un index sur ces colonnes. Un index se paie à chaque
-- insertion : on n'en ajoute que là où une suppression réelle ou une
-- requête réelle du code s'en sert. Liste établie en interrogeant le
-- catalogue (pg_constraint, pg_index) d'une base à jour jusqu'à 0033.
--
-- La fonction uuidv7() de remplacement de la remédiation n'est pas reprise :
-- PostgreSQL 18 la fournit (doc PostgreSQL 18, §9.14, uuidv7).
--
-- Index non repris, car la table parente n'est jamais effacée (triggers ou
-- règle du projet) et aucune requête ne filtre sur ces colonnes :
-- autorisation_validee(structure_id), decision_structure(administrateur),
-- affectation(compte_id) (les lectures portent sur les affectations
-- actives, déjà couvertes par affectation_une_active_par_role),
-- affectation(desactivee_par_compte_id), invitation(invite_par_compte_id),
-- historique_statut_demande(agent_affectation_id),
-- intervention_compte(affectation_id) et (administrateur_compte_id).
--
-- CREATE INDEX sans CONCURRENTLY : sqlx exécute chaque migration dans une
-- transaction, où CONCURRENTLY est interdit (doc PostgreSQL 18, CREATE
-- INDEX). Les tables sont encore petites : le verrou d'écriture pendant la
-- construction est bref.

-- ---------------------------------------------------------------------
-- Suppressions réelles (lignes filles cherchées à chaque suppression)
-- ---------------------------------------------------------------------

-- La désactivation d'un TOTP efface sa ligne et ses codes en cascade
-- (décision Y1). L'index partiel code_secours_disponibles ne couvre que les
-- codes non utilisés.
CREATE INDEX code_secours_second_facteur_idx ON code_secours (second_facteur_id);

-- La purge des sessions (Q-b, session.rs) efface leurs jetons en cascade ;
-- chaque jeton effacé oblige à chercher ceux qui le désignent comme
-- successeur. Index partiel : une égalité implique une valeur non nulle,
-- PostgreSQL peut donc s'en servir pour ce contrôle.
CREATE INDEX jeton_renouvellement_remplace_par_idx
    ON jeton_renouvellement (remplace_par_id) WHERE remplace_par_id IS NOT NULL;

-- ---------------------------------------------------------------------
-- Requêtes réelles du code
-- ---------------------------------------------------------------------

-- « Mes demandes » (structure.rs) : toutes les structures d'un créateur,
-- validées comprises ; l'index unique structure_une_non_validee_par_createur
-- ne couvre que les non validées.
CREATE INDEX structure_createur_idx ON structure (createur_compte_id);

-- Liste de toutes les pièces d'une structure, anciennes comprises
-- (piece_justificative.rs) ; l'index unique existant ne couvre que les
-- pièces actives.
CREATE INDEX piece_justificative_structure_idx ON piece_justificative (structure_id);

-- Délai avant un nouvel envoi du lien de vérification (jeton_email.rs) :
-- compte et objet, sans condition sur l'état du jeton ; les index partiels
-- existants ne couvrent que les jetons en attente.
CREATE INDEX jeton_email_compte_objet_idx ON jeton_email (compte_id, objet);

-- Retrait d'un accès (membres.rs, intervention_compte.rs) : fermeture des
-- sessions ouvertes sur une affectation.
CREATE INDEX session_affectation_idx ON session (affectation_id) WHERE affectation_id IS NOT NULL;

-- Pharmacie (0030 à 0033) : journal du stock et journal des dépenses,
-- paginés par identifiant décroissant dans une pharmacie
-- (stock_pharmacie::journal, benefice_pharmacie::lister_depenses). Les
-- autres lectures de la pharmacie sont déjà couvertes (lots par pharmacie et
-- médicament, prix, seuils, clés d'idempotence, dépenses par date).
CREATE INDEX mouvement_stock_structure_id_idx ON mouvement_stock (structure_id, id);
CREATE INDEX depense_pharmacie_structure_id_idx ON depense_pharmacie (structure_id, id);
