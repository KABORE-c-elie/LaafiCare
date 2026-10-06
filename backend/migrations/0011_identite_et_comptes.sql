-- Comptes séparés d'une même identité (section 14 du CLAUDE.md) : chacun
-- a son mot de passe, son verrouillage, son statut et sa version de jeton.
-- Un blocage du compte patient ne touche jamais le compte professionnel,
-- et inversement. `utilisateur` garde l'identité : nom, prénom, téléphone
-- unique, email, OTP (réservé au compte patient).
CREATE TABLE compte (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    utilisateur_id      UUID NOT NULL REFERENCES utilisateur (id),
    type_compte         TEXT NOT NULL
                        CHECK (type_compte IN ('patient', 'professionnel', 'administrateur_laaficare')),

    -- Obligatoire : un compte n'existe qu'une fois son mot de passe défini.
    -- Une inscription patient abandonnée = une identité sans compte patient.
    mot_de_passe_hash   TEXT NOT NULL,

    -- Verrouillage 5 tentatives / 15 min (BF-01), propre à ce compte. Le
    -- compteur est augmenté par la base elle-même (décision K2) ; le CHECK
    -- garantit qu'aucune erreur ne le rend négatif.
    tentatives_echouees SMALLINT NOT NULL DEFAULT 0 CHECK (tentatives_echouees >= 0),
    verrouille_jusqua   TIMESTAMPTZ,

    statut              TEXT NOT NULL DEFAULT 'actif' CHECK (statut IN ('actif', 'desactive')),

    -- Recopiée dans le jeton. L'incrémenter invalide aussitôt les jetons
    -- déjà émis (bascule vers le patient, section 14).
    version_jeton       INTEGER NOT NULL DEFAULT 0 CHECK (version_jeton >= 0),

    date_creation       TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Au plus un compte de chaque type par personne ; l'index de cette
    -- contrainte sert aussi à retrouver un compte par identité et type.
    CONSTRAINT un_compte_par_type UNIQUE (utilisateur_id, type_compte)
);

-- Non vérifiable en SQL (l'information est dans d'autres tables), donc
-- vérifié par le code à la création du compte : un compte professionnel
-- ou administrateur exige un email sur l'identité ; un compte
-- professionnel exige d'abord un compte patient.

-- Reprise : un compte patient pour chaque patient qui a un mot de passe.
-- Les patients sans mot de passe (données de test créées en SQL) n'en ont
-- pas ; leurs contrats et demandes restent valables.
INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash, tentatives_echouees, verrouille_jusqua)
SELECT u.id, 'patient', u.mot_de_passe_hash, u.tentatives_echouees, u.verrouille_jusqua
FROM patient p JOIN utilisateur u ON u.id = p.utilisateur_id
WHERE u.mot_de_passe_hash IS NOT NULL;

-- Comptes agents de développement non repris (décision V5 a) : ils seront
-- recréés par invitation une fois la MUNASEB créée et validée. La table de
-- rôle disparaît (décision V6) : le rôle passera sur les affectations
-- (migration 0012). Rien ne la référence : l'historique des statuts pointe
-- vers l'identité.
DROP TABLE agent_assurance_munaseb;

-- Le mot de passe et le verrouillage quittent l'identité.
ALTER TABLE utilisateur
    DROP COLUMN mot_de_passe_hash,
    DROP COLUMN tentatives_echouees,
    DROP COLUMN verrouille_jusqua;
