-- Séquence globale pour le NIP (décision actée, remplace tout schéma par
-- année) : plafonnée à 15 chiffres pour ne jamais déborder du format
-- (échec propre à l'épuisement plutôt qu'un dépassement silencieux).
CREATE SEQUENCE nip_seq AS BIGINT MINVALUE 1 MAXVALUE 999999999999999 NO CYCLE;

-- Patient : propre clé primaire + utilisateur_id en FK (principe
-- multi-rôles, section 6 CLAUDE.md) -- pas d'héritage à clé partagée, pas
-- de duplication des champs déjà sur `utilisateur` (nom, prénom,
-- téléphone).
CREATE TABLE patient (
    id             UUID PRIMARY KEY DEFAULT uuidv7(),
    utilisateur_id UUID NOT NULL UNIQUE REFERENCES utilisateur (id),
    nip            TEXT NOT NULL UNIQUE,
    date_naissance DATE NOT NULL,
    lieu_naissance TEXT NOT NULL,

    -- 16 chiffres purement numériques (payload de 15 chiffres + chiffre de
    -- contrôle Luhn), voir src/nip.rs pour la génération/validation.
    CONSTRAINT nip_format CHECK (nip ~ '^[0-9]{16}$')
);
