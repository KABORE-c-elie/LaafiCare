-- Droits des rôles du serveur et des outils (lot « avant le pilote », P1,
-- décisions 3 et 4 du porteur, 2026-10-22). Les rôles sont créés par
-- `sql/roles.sql` avant les migrations : sans eux, cette migration échoue,
-- volontairement (le message de PostgreSQL nomme le rôle manquant).
--
-- Exigence (CLAUDE.md §12) : le serveur ne doit être ni superuser ni
-- propriétaire des tables. Sinon, il pourrait désactiver les triggers qui
-- rendent les traces non modifiables (historique des demandes, décisions,
-- désactivations de TOTP, créations et reprises d'administrateurs).
--
-- Sources : doc PostgreSQL 18, GRANT (notes sur les droits par colonne),
-- ALTER DEFAULT PRIVILEGES, CREATE FUNCTION (« Writing SECURITY DEFINER
-- Functions Safely »).

-- Le serveur lit et écrit les tables, sans pouvoir en créer (depuis
-- PostgreSQL 15, PUBLIC n'a plus le droit CREATE sur le schéma public), ni
-- les vider d'un coup (TRUNCATE n'est pas donné).
GRANT USAGE ON SCHEMA public TO laaficare_serveur;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO laaficare_serveur;
-- nextval() de la séquence du NIP demande USAGE.
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO laaficare_serveur;

-- Les tables des migrations suivantes, créées par le même rôle propriétaire,
-- reçoivent les mêmes droits d'elles-mêmes (la commande ne touche que les
-- objets créés ensuite par le rôle qui la lance, ici celui des migrations).
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO laaficare_serveur;
ALTER DEFAULT PRIVILEGES IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO laaficare_serveur;

-- Le journal des migrations : le serveur le lit pour vérifier au démarrage
-- que la base est à jour, mais ne l'écrit jamais (seule la commande `migrer`
-- applique les migrations, avec le rôle propriétaire).
REVOKE INSERT, UPDATE, DELETE ON _sqlx_migrations FROM laaficare_serveur;

-- Traces jamais modifiées ni supprimées : en plus de leurs triggers, le
-- serveur n'a aucun droit de les modifier ou de les supprimer.
REVOKE UPDATE, DELETE ON historique_statut_demande, decision_structure, desactivation_totp,
    creation_administrateur, reprise_administrateur FROM laaficare_serveur;

-- Fenêtre de réenrôlement d'un administrateur (migration 0019) : seuls les
-- outils locaux l'ouvrent. Doc PostgreSQL 18, GRANT : « Granting the
-- privilege at the table level and then revoking it for one column will not
-- do what one might wish: the table-level grant is unaffected by a
-- column-level operation. » Le droit de modifier `compte` est donc retiré à
-- la table, puis rendu colonne par colonne, sans cette colonne.
-- RÈGLE POUR LES MIGRATIONS SUIVANTES : une colonne ajoutée à `compte` que
-- le serveur doit modifier reçoit son propre GRANT UPDATE (colonne).
REVOKE UPDATE ON compte FROM laaficare_serveur;
GRANT UPDATE (mot_de_passe_hash, tentatives_echouees, verrouille_jusqua, statut, version_jeton,
              second_facteur_dernier_iat)
    ON compte TO laaficare_serveur;
GRANT UPDATE (reenrolement_autorise_jusqu_au) ON compte TO laaficare_outils;

-- Le serveur doit pouvoir VIDER la fenêtre (confirmation du réenrôlement),
-- jamais l'OUVRIR : une fonction qui ne fait que la vider, exécutée avec les
-- droits de son propriétaire. Doc PostgreSQL 18, CREATE FUNCTION : le
-- search_path est fixé, avec pg_temp en dernier, pour qu'aucun objet
-- temporaire ne masque la table ; l'exécution par PUBLIC, donnée par
-- défaut, est retirée dans la même transaction que la création (sqlx
-- applique chaque migration dans une transaction).
-- Renvoie l'identifiant du compte si la fenêtre était ouverte, NULL sinon.
CREATE FUNCTION vider_fenetre_reenrolement(p_compte_id UUID) RETURNS UUID
    LANGUAGE sql
    SECURITY DEFINER
    SET search_path = public, pg_temp
AS $$
    UPDATE compte SET reenrolement_autorise_jusqu_au = NULL
    WHERE id = p_compte_id AND reenrolement_autorise_jusqu_au > now()
    RETURNING id
$$;
REVOKE ALL ON FUNCTION vider_fenetre_reenrolement(UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION vider_fenetre_reenrolement(UUID) TO laaficare_serveur;
