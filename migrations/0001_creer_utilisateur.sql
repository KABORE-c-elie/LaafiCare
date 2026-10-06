-- Utilisateur : identité centrale (BF-01, section 11 et section 6 du
-- CLAUDE.md). Ne porte aucune information de rôle : les futures tables de
-- rôle (patient, agent_assurance, ...) référenceront utilisateur_id en clé
-- étrangère, avec leur propre clé primaire -- pas d'héritage à clé
-- partagée, pour permettre à une même identité de cumuler plusieurs rôles
-- (principe multi-rôles acté, CLAUDE.md section 6).

CREATE TABLE utilisateur (
    -- uuidv7() : fonction native PostgreSQL 18 (doc PostgreSQL 18,
    -- functions-uuid.html). UUID ordonné dans le temps -> meilleure
    -- localité d'index qu'un uuidv4() aléatoire, sans exposer un entier
    -- séquentiel devinable comme le ferait un SERIAL.
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),

    nom                 TEXT NOT NULL,
    prenom              TEXT NOT NULL,

    -- Identifiant de connexion des rôles non-Patient (section 11).
    -- Facultatif : un Patient n'en a pas besoin.
    email               TEXT,

    -- Identifiant de connexion du Patient (numéro qui reçoit l'OTP,
    -- section 11), obligatoire et unique pour TOUS les rôles : l'identité
    -- (une personne = un utilisateur) est portée par ce champ, jamais par
    -- le rôle.
    telephone           TEXT NOT NULL UNIQUE,

    -- Secret des rôles non-Patient (Argon2id, section 11 -- décision
    -- actée, remplace BCrypt initialement prévu en BNF-01). NULL pour un
    -- Patient : aucun mot de passe, OTP uniquement.
    mot_de_passe_hash   TEXT,

    -- Verrouillage 5 tentatives / 15 min (BF-01). Deux colonnes suffisent
    -- à ce stade, pas de table de journal séparée.
    tentatives_echouees SMALLINT NOT NULL DEFAULT 0,
    verrouille_jusqua   TIMESTAMPTZ,

    date_creation       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT telephone_format CHECK (telephone ~ '^\+[1-9][0-9]{6,14}$')
);

-- Unicité de l'email insensible à la casse. NULL autorisé en plusieurs
-- exemplaires : par défaut PostgreSQL ne considère jamais deux NULL comme
-- égaux dans une contrainte d'unicité (doc PostgreSQL 18,
-- ddl-constraints.html), donc plusieurs comptes Patient sans email
-- cohabitent sans violer cet index.
CREATE UNIQUE INDEX utilisateur_email_unique_idx ON utilisateur (lower(email));
