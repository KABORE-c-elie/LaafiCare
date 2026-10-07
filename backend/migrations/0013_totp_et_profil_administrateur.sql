-- Second facteur TOTP, codes de secours, désactivations par l'équipe
-- LaafiCare et profil administrateur (section 14 du CLAUDE.md ; décisions
-- Z1 à Z10, Y1 à Y4, Y3-bis et K1 à K3, du 2026-10-05 au 2026-10-06).
-- Seule la structure des tables est posée ici : aucune route, aucun outil.

-- ---------------------------------------------------------------------
-- Second facteur TOTP (RFC 6238). Au plus un actif et un en attente par
-- compte : pendant un remplacement, l'ancien reste valable jusqu'à la
-- confirmation du nouveau (Z7). Un TOTP désactivé ou remplacé est
-- supprimé, secret compris (Y1) : la trace reste dans desactivation_totp.
-- ---------------------------------------------------------------------
CREATE TABLE second_facteur_totp (
    id               UUID PRIMARY KEY DEFAULT uuidv7(),
    compte_id        UUID NOT NULL REFERENCES compte (id),
    -- Secret de 160 bits (20 octets, RFC 4226) chiffré en AES-256-GCM
    -- (Z4) : texte chiffré de même taille que le secret, suivi de
    -- l'étiquette d'authentification de 16 octets, soit 36 octets. À
    -- confirmer dans la doc de `aes-gcm` et par un test unitaire avant
    -- l'écriture de `totp.rs` ; en cas d'écart, cette migration est
    -- corrigée avant toute application sur une vraie base.
    secret_chiffre   BYTEA NOT NULL CHECK (octet_length(secret_chiffre) = 36),
    -- Nonce aléatoire de 96 bits par chiffrement (Z4).
    nonce            BYTEA NOT NULL CHECK (octet_length(nonce) = 12),
    -- Clé de chiffrement utilisée : prépare un futur changement de clé (Z4).
    version_cle      SMALLINT NOT NULL CHECK (version_cle >= 1),
    statut           TEXT NOT NULL DEFAULT 'en_attente' CHECK (statut IN ('en_attente', 'actif')),
    -- Dernier pas de temps accepté : un code n'est accepté que si son pas
    -- est strictement plus grand (RFC 6238 « MUST NOT accept the second
    -- attempt », NIST SP 800-63B rév. 4 §3.1.4 « SHALL accept a given OTP
    -- only once »). Renseigné dès l'activation, avec le pas du code de
    -- confirmation, pour que ce code ne serve pas une seconde fois.
    dernier_pas      BIGINT CHECK (dernier_pas >= 0),
    date_creation    TIMESTAMPTZ NOT NULL DEFAULT now(),
    date_activation  TIMESTAMPTZ,

    CONSTRAINT second_facteur_totp_activation_datee
        CHECK ((statut = 'actif') = (date_activation IS NOT NULL)),
    CONSTRAINT second_facteur_totp_pas_si_actif
        CHECK (dernier_pas IS NULL OR statut = 'actif')
);

-- Au remplacement, le code supprime l'ancien TOTP actif avant d'activer
-- le nouveau, dans la même transaction ; sinon le premier index refuse.
CREATE UNIQUE INDEX second_facteur_totp_un_actif
    ON second_facteur_totp (compte_id) WHERE statut = 'actif';
CREATE UNIQUE INDEX second_facteur_totp_un_en_attente
    ON second_facteur_totp (compte_id) WHERE statut = 'en_attente';

-- ---------------------------------------------------------------------
-- Codes de secours (Z6) : 10 par TOTP, à usage unique. Rattachés au TOTP
-- et non au compte : la suppression du TOTP (Y1) les emporte, sans code
-- à écrire pour ça. Qu'ils n'existent que pour un TOTP actif est garanti
-- par le code, au moment de la confirmation.
-- ---------------------------------------------------------------------
CREATE TABLE code_secours (
    id                 UUID PRIMARY KEY DEFAULT uuidv7(),
    second_facteur_id  UUID NOT NULL REFERENCES second_facteur_totp (id) ON DELETE CASCADE,
    -- Argon2id au format PHC, sel aléatoire inclus : environ 50 bits
    -- d'entropie, sous les 112 bits au-delà desquels NIST SP 800-63B rév. 4
    -- §3.1.2 dispense d'un hachage de mot de passe salé.
    hash               TEXT NOT NULL,
    utilise_le         TIMESTAMPTZ,
    date_creation      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX code_secours_disponibles
    ON code_secours (second_facteur_id) WHERE utilise_le IS NULL;

-- ---------------------------------------------------------------------
-- Désactivations par l'équipe LaafiCare (Z9). Jamais modifiées ni
-- supprimées (triggers plus bas).
-- ---------------------------------------------------------------------
CREATE TABLE desactivation_totp (
    id                          UUID PRIMARY KEY DEFAULT uuidv7(),
    -- Compte dont le TOTP est désactivé : patient, professionnel ou
    -- administrateur.
    compte_id                   UUID NOT NULL REFERENCES compte (id),
    administrateur_compte_id    UUID NOT NULL,
    administrateur_type_compte  TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (administrateur_type_compte = 'administrateur_laaficare'),
    motif                       TEXT NOT NULL CHECK (btrim(motif) <> ''),
    -- Y2 : attestation obligatoire que la CNIB de la personne a été
    -- vérifiée ; son numéro n'est jamais enregistré.
    cnib_verifiee               BOOLEAN NOT NULL CHECK (cnib_verifiee),
    date_desactivation          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT desactivation_totp_administrateur
        FOREIGN KEY (administrateur_compte_id, administrateur_type_compte) REFERENCES compte (id, type_compte)
);

CREATE INDEX desactivation_totp_compte_idx
    ON desactivation_totp (compte_id, date_desactivation);

-- Y3 et Y3-bis : un administrateur ne désactive le TOTP d'aucun de ses
-- propres comptes. Un CHECK ne peut pas lire une autre table, d'où un
-- trigger : il compare l'utilisateur des deux comptes. Si l'un des comptes
-- n'existe pas, la jointure ne trouve rien et la clé étrangère refuse la
-- ligne ensuite (les triggers BEFORE passent avant les contraintes).
CREATE FUNCTION desactivation_totp_par_un_autre() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM compte cible
        JOIN compte administrateur ON administrateur.utilisateur_id = cible.utilisateur_id
        WHERE cible.id = NEW.compte_id
          AND administrateur.id = NEW.administrateur_compte_id
    ) THEN
        RAISE EXCEPTION 'desactivation_totp : un administrateur ne désactive pas le TOTP de ses propres comptes';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER desactivation_totp_autre_personne
    BEFORE INSERT ON desactivation_totp
    FOR EACH ROW EXECUTE FUNCTION desactivation_totp_par_un_autre();

-- Même mécanisme que les migrations 0010 et 0012 (doc PostgreSQL 18,
-- CREATE TRIGGER : TRUNCATE ne déclenche pas les triggers ligne par ligne,
-- d'où un second trigger FOR EACH STATEMENT). Protège contre une erreur,
-- pas contre un administrateur de la base (section 12).
CREATE FUNCTION desactivation_totp_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'desactivation_totp : les lignes ne sont jamais modifiées ni supprimées (%)', TG_OP;
END;
$$;

CREATE TRIGGER desactivation_totp_sans_modification
    BEFORE UPDATE OR DELETE ON desactivation_totp
    FOR EACH ROW EXECUTE FUNCTION desactivation_totp_non_modifiable();

CREATE TRIGGER desactivation_totp_sans_troncature
    BEFORE TRUNCATE ON desactivation_totp
    FOR EACH STATEMENT EXECUTE FUNCTION desactivation_totp_non_modifiable();

-- ---------------------------------------------------------------------
-- Profil administrateur (K1 à K3). Table à part et non colonne de
-- `compte` : le numéro de CNIB n'apparaît dans aucune requête de
-- connexion. Obligatoire pour tout administrateur, garanti par le code
-- (compte et profil créés dans la même transaction, K3).
-- ---------------------------------------------------------------------
CREATE TABLE profil_administrateur (
    compte_id              UUID PRIMARY KEY,
    compte_type            TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                           CHECK (compte_type = 'administrateur_laaficare'),
    -- Conservé tel que saisi.
    numero_cnib            TEXT NOT NULL,
    -- Forme de comparaison : majuscules sur les lettres ASCII seulement,
    -- puis retrait des 25 caractères Unicode White_Space (PropList.txt),
    -- aucune autre transformation. Mêmes choix que numero_normalise de la
    -- migration 0012 (doc PostgreSQL 18, 24.2 et 9.7.3.2 : COLLATE "C" et
    -- liste explicite au lieu de `\s`, pour un résultat identique sur tous
    -- les serveurs). Chaque caractère est écrit en échappement long
    -- `\UXXXXXXXX` (doc 9.7.3.3), jamais tapé tel quel : aucun caractère
    -- invisible dans ce fichier, qu'un éditeur pourrait modifier sans que
    -- personne le voie. Pas la forme courte `\uXXXX` : certains outils
    -- d'édition la remplacent d'office par le caractère lui-même.
    numero_cnib_normalise  TEXT NOT NULL GENERATED ALWAYS AS (
                               regexp_replace(
                                   upper(numero_cnib COLLATE "C"),
                                   '[\U00000009\U0000000A\U0000000B\U0000000C\U0000000D\U00000020\U00000085\U000000A0\U00001680'
                                   '\U00002000\U00002001\U00002002\U00002003\U00002004\U00002005\U00002006\U00002007\U00002008\U00002009\U0000200A'
                                   '\U00002028\U00002029\U0000202F\U0000205F\U00003000]',
                                   '', 'g')
                           ) STORED,
    date_enregistrement    TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT profil_administrateur_compte
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte),
    -- K3 : unicité garantie par la base, sur le numéro normalisé.
    CONSTRAINT profil_administrateur_cnib_unique UNIQUE (numero_cnib_normalise),
    -- Format de la CNIB actuelle, fourni par le porteur (pas de source
    -- officielle) : B suivi de 8 chiffres. Chiffres listés un par un, sans
    -- plage : la doc (9.7.3.2) dit que les plages dépendent de l'ordre de
    -- tri. Couvre aussi le numéro vide. À élargir quand la carte de l'AES
    -- arrivera, sans retirer ce format (section 14).
    CONSTRAINT profil_administrateur_cnib_format
        CHECK (numero_cnib_normalise ~ '^B[0123456789]{8}$')
);
