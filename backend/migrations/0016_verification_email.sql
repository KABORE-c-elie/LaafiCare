-- Vérification de l'email d'un compte professionnel ou administrateur
-- (décision R0, 2026-10-08). L'email est sur l'identité (`utilisateur`) :
-- sa vérification aussi. Un compte professionnel ou administrateur dont
-- l'email n'est pas vérifié reste inutilisable (connexion : 403
-- `email_non_verifie`). La façon de vérifier (lien à usage unique valable
-- 24 h, envoyé à l'adresse : NIST SP 800-63A rév. 4 §3.8) est construite
-- là où naissent les comptes (étape 3.7).
ALTER TABLE utilisateur
    ADD COLUMN email_verifie_le TIMESTAMPTZ;

-- Une date de vérification sans email n'a pas de sens.
ALTER TABLE utilisateur
    ADD CONSTRAINT utilisateur_email_verifie_avec_email
        CHECK (email_verifie_le IS NULL OR email IS NOT NULL);

-- Une identité ne naît jamais vérifiée : la vérification arrive toujours
-- après, par un UPDATE, une fois le lien ouvert et confirmé. Doc
-- PostgreSQL 18, CREATE TRIGGER : un trigger BEFORE peut « change the row
-- being inserted (for INSERT and UPDATE operations only) ».
CREATE FUNCTION email_jamais_verifie_a_la_creation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.email_verifie_le := NULL;
    RETURN NEW;
END;
$$;

CREATE TRIGGER email_jamais_verifie_a_la_creation
    BEFORE INSERT ON utilisateur
    FOR EACH ROW EXECUTE FUNCTION email_jamais_verifie_a_la_creation();

-- Tout changement d'email annule la vérification : la nouvelle adresse
-- doit être vérifiée à son tour. Garanti par la base, pour tout appelant
-- (code, futur outil d'administration). Toute différence compte, majuscules
-- comprises : rien ne garantit que deux écritures désignent la même boîte.
-- Doc PostgreSQL 18, CREATE TRIGGER : un trigger UPDATE OF « will only fire
-- if at least one of the listed columns is mentioned as a target of the
-- UPDATE command », même si la valeur ne change pas ; d'où IS DISTINCT
-- FROM, qui ignore une réécriture à l'identique et traite correctement
-- NULL. Une mise à jour qui change l'email et pose la vérification en même
-- temps voit la vérification annulée : elle doit toujours suivre le
-- changement, jamais l'accompagner.
CREATE FUNCTION email_change_annule_verification() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.email IS DISTINCT FROM OLD.email THEN
        NEW.email_verifie_le := NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER email_change_annule_verification
    BEFORE UPDATE OF email ON utilisateur
    FOR EACH ROW EXECUTE FUNCTION email_change_annule_verification();
