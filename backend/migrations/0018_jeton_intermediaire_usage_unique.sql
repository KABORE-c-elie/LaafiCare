-- Jeton intermédiaire du second facteur à usage unique (décision F5 b,
-- 2026-10-15), sans table de jetons : le compte garde le `iat` (date
-- d'émission, en secondes) du dernier jeton intermédiaire consommé par un
-- second facteur réussi.
--
-- On enregistre le `iat` du jeton lui-même, et non une date de la base :
-- `iat` vient de l'horloge du serveur Rust, `now()` de celle de PostgreSQL ;
-- un écart d'une ou deux secondes entre les deux casserait la règle (jeton
-- réutilisable, ou refus à tort). Ici, une seule horloge est comparée à
-- elle-même.
--
-- Règles, dans `auth_professionnel.rs` et `extracteur_jwt.rs` :
-- - consommation, en une seule requête conditionnelle après un code
--   correct : la colonne ne prend la valeur `iat` que si elle est vide ou
--   plus petite ; sinon le jeton a déjà servi ;
-- - un jeton intermédiaire dont `iat` est inférieur ou égal à la valeur
--   enregistrée est refusé.
-- Limite connue : un nouveau jeton émis dans la même seconde que celui qui
-- vient d'être consommé est refusé à tort ; la personne se reconnecte.
ALTER TABLE compte
    ADD COLUMN second_facteur_dernier_iat BIGINT;

-- Une date d'émission est un nombre de secondes positif depuis 1970.
ALTER TABLE compte
    ADD CONSTRAINT compte_second_facteur_dernier_iat_positif
        CHECK (second_facteur_dernier_iat IS NULL OR second_facteur_dernier_iat > 0);
