-- Étape 3.7, réduite à la MUNASEB (décisions E1 à E13 et du porteur des
-- 2026-10-20 et 2026-10-21) : numéro d'urgence, région et email de la
-- structure ; emails sans espaces autour, en ASCII, de 254 octets au plus,
-- garantis par la base.

-- La comparaison ASCII ci-dessous (octets = caractères) ne vaut qu'en
-- UTF-8 : RFC 3629 §3, tout caractère hors ASCII y occupe 2 à 4 octets. La
-- migration s'arrête plutôt que de poser une contrainte qui ne garantirait
-- rien (doc PostgreSQL 18, « Character Set Support » ; paramètre
-- `server_encoding`).
DO $$
BEGIN
    IF current_setting('server_encoding') <> 'UTF8' THEN
        RAISE EXCEPTION 'migration 0022 : base en UTF8 attendue, trouvé %', current_setting('server_encoding');
    END IF;
END
$$;

-- ---------------------------------------------------------------------
-- Champs obligatoires de la structure (décision du porteur, 2026-10-20) :
-- le numéro de service existe déjà (`telephone`) ; s'ajoutent le numéro
-- d'urgence, la région et l'email. NOT NULL dès cette migration : aucune
-- base de production n'existe, et la base de développement repart de zéro
-- (décision du porteur, 2026-10-21). Sur une table qui contient déjà une
-- structure, la migration échoue et n'applique rien (doc PostgreSQL 18,
-- ALTER TABLE, SET NOT NULL : « the column must not contain any null
-- values ») ; SQLx exécute chaque migration dans une transaction.
-- ---------------------------------------------------------------------

-- Déjà normalisé par `telephone.rs` avant l'écriture, comme `telephone`.
-- Il peut être identique au numéro de service (décision du porteur).
ALTER TABLE structure ADD COLUMN telephone_urgence TEXT;
ALTER TABLE structure ALTER COLUMN telephone_urgence SET NOT NULL;

-- Texte libre, comme la province et la commune : aucun texte officiel du
-- découpage en vigueur n'a été trouvé le 2026-10-20 (seul le compte rendu
-- du Conseil des ministres du 2 juillet 2025 donne les 17 régions ; ni le
-- décret, ni la liste des 47 provinces). Point ouvert : docs/points-ouverts.md.
ALTER TABLE structure ADD COLUMN region TEXT;
ALTER TABLE structure ALTER COLUMN region SET NOT NULL;
ALTER TABLE structure ADD CONSTRAINT structure_region_non_vide CHECK (btrim(region) <> '');

-- Email de la structure (décision du porteur, 2026-10-20) : coordonnée de
-- l'établissement, distincte de l'email de connexion de son responsable
-- (compte professionnel). Pas vérifié par lien, pas unique (plusieurs
-- établissements d'un même groupe peuvent partager une adresse). Mêmes
-- règles de base que l'email d'une identité, ci-dessous.
ALTER TABLE structure ADD COLUMN email TEXT;
ALTER TABLE structure ALTER COLUMN email SET NOT NULL;
ALTER TABLE structure ADD CONSTRAINT structure_email_non_vide CHECK (email = btrim(email) AND email <> '');
ALTER TABLE structure ADD CONSTRAINT structure_email_ascii CHECK (octet_length(email) = char_length(email));
ALTER TABLE structure ADD CONSTRAINT structure_email_254_octets CHECK (octet_length(email) <= 254);

-- ---------------------------------------------------------------------
-- Email d'une identité.
--
-- Sans espaces autour (décision du 2026-10-15). La connexion compare
-- `lower(u.email) = lower(btrim($1))` (J6) : un compte n'est trouvé, et
-- l'index unique sur `lower(email)` (migration 0001) n'est sûr, que si les
-- emails sont enregistrés sans espaces autour. `btrim` sans second argument
-- ne retire que l'espace ordinaire, comme la comparaison de la connexion.
--
-- ASCII seulement (décision du porteur, 2026-10-21 ; contrôle complet dans
-- `adresse_email.rs`) : en ASCII, `lower` donne le même résultat quels que
-- soient les réglages de langue de la base (doc PostgreSQL 18, fonctions de
-- chaînes : `lower` « Converts the string to all lower case, according to
-- the rules of the database's locale »).
--
-- 254 octets au plus : RFC 5321 §4.5.3.1.3, chemin de 256 octets moins les
-- `<` `>` (§4.1.2). La partie locale (64 octets, §4.5.3.1.1) est contrôlée
-- par le code seulement (décision du porteur, 2026-10-21).
--
-- Base de développement vérifiée le 2026-10-20 : 3 emails, aucun ne viole
-- ces contraintes. NULL passe (doc PostgreSQL 18, 5.5.1 : une contrainte
-- CHECK est satisfaite si l'expression vaut vrai ou NULL) : l'email d'un
-- patient reste facultatif.
-- ---------------------------------------------------------------------
ALTER TABLE utilisateur ADD CONSTRAINT utilisateur_email_sans_espaces_autour
    CHECK (email = btrim(email));
ALTER TABLE utilisateur ADD CONSTRAINT utilisateur_email_ascii
    CHECK (octet_length(email) = char_length(email));
ALTER TABLE utilisateur ADD CONSTRAINT utilisateur_email_254_octets
    CHECK (octet_length(email) <= 254);
