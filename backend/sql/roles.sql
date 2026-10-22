-- Rôles PostgreSQL de LaafiCare (lot « avant le pilote », P1, décisions 1 et
-- 2 du porteur, 2026-10-22). À lancer UNE fois par serveur PostgreSQL, par
-- un rôle qui a le droit de créer des rôles (en développement : l'utilisateur
-- `laaficare` du docker-compose). Les rôles sont communs à toutes les bases
-- du serveur : c'est pourquoi ils ne sont pas créés par une migration, qui
-- est rejouée dans chaque base de test (`#[sqlx::test]`).
--
-- AUCUN MOT DE PASSE ICI : ce fichier est dans le dépôt. Les mots de passe
-- se posent ensuite avec `\password` dans psql (voir le README), qui ne les
-- fait apparaître ni à l'écran ni dans l'historique.
--
-- Sans danger s'il est relancé : chaque rôle n'est créé que s'il manque.
--
-- - laaficare_serveur : le serveur. Ni superuser, ni propriétaire des
--   tables, ni droit de créer des bases ou des rôles (exigence de
--   production, CLAUDE.md §12). Ses droits sur les tables sont donnés par la
--   migration 0026.
-- - laaficare_outils : les outils locaux de l'équipe (`src/bin/`). Il reçoit
--   les droits du serveur par héritage, plus le seul droit d'ouvrir la
--   fenêtre de réenrôlement d'un administrateur (migration 0026).
--
-- Les migrations sont appliquées par le rôle propriétaire des tables
-- (`cargo run --bin migrer`, avec DATABASE_URL) : en développement,
-- l'utilisateur `laaficare` ; en production, un rôle propriétaire sans droits
-- de superuser, créé par l'hébergeur.
--
-- Source : doc PostgreSQL 18, CREATE ROLE et « Role Membership » : un membre
-- d'un rôle avec INHERIT utilise ses droits sans SET ROLE.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'laaficare_serveur') THEN
        CREATE ROLE laaficare_serveur LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'laaficare_outils') THEN
        CREATE ROLE laaficare_outils LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS INHERIT;
    END IF;
END
$$;

GRANT laaficare_serveur TO laaficare_outils;
