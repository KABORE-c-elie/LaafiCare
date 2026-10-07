-- Structures sous licence, pièces justificatives, registre des
-- autorisations validées, décisions de l'équipe LaafiCare, affectations et
-- invitations (section 14 du CLAUDE.md ; décisions M1 à M9 du 2026-10-04).
-- Seule la structure des tables est posée ici : aucune route, aucune
-- donnée reprise.

-- ---------------------------------------------------------------------
-- Type de compte garanti par la base. Une clé étrangère composite
-- (compte_id, type_compte) vers cette contrainte empêche, par exemple,
-- d'affecter un compte patient à une structure. Doc PostgreSQL 18, 5.5.5
-- Foreign Keys : une clé étrangère doit référencer des colonnes qui sont
-- une clé primaire ou qui forment une contrainte unique. `id` étant déjà
-- unique, cette contrainte ne restreint rien d'autre.
-- ---------------------------------------------------------------------
ALTER TABLE compte ADD CONSTRAINT compte_id_type_unique UNIQUE (id, type_compte);

-- ---------------------------------------------------------------------
-- Structures
-- ---------------------------------------------------------------------
CREATE TABLE structure (
    id                     UUID PRIMARY KEY DEFAULT uuidv7(),
    type_structure         TEXT NOT NULL
                           CHECK (type_structure IN ('clinique', 'hopital', 'pharmacie', 'laboratoire', 'munaseb')),
    nom                    TEXT NOT NULL CHECK (btrim(nom) <> ''),
    -- Déjà normalisé par `telephone.rs` avant l'écriture.
    telephone              TEXT NOT NULL,

    -- Adresse cadastrale : parcelle, lot et section facultatifs (zones non
    -- loties). Liste des provinces non fermée faute de source officielle
    -- (point ouvert, section 14).
    commune                TEXT NOT NULL CHECK (btrim(commune) <> ''),
    province               TEXT NOT NULL CHECK (btrim(province) <> ''),
    parcelle               TEXT,
    lot                    TEXT,
    section_cadastrale     TEXT,

    -- NUMERIC(9,6) (décision M2) : 6 décimales, environ 11 cm, et la valeur
    -- saisie est conservée exactement (un DOUBLE PRECISION l'arrondirait en
    -- binaire). 3 chiffres avant la virgule suffisent pour ±180.
    latitude               NUMERIC(9,6) NOT NULL CHECK (latitude BETWEEN -90 AND 90),
    longitude              NUMERIC(9,6) NOT NULL CHECK (longitude BETWEEN -180 AND 180),

    -- Cycle (section 14) : brouillon → en_attente → validee | refusee ;
    -- validee ↔ suspendue (réactivation, M8) ; brouillon | refusee →
    -- abandonnee (M5, état final). Les passages permis sont vérifiés par le
    -- code ; la base garantit les deux limites ci-dessous.
    statut                 TEXT NOT NULL DEFAULT 'brouillon'
                           CHECK (statut IN ('brouillon', 'en_attente', 'validee', 'refusee', 'suspendue', 'abandonnee')),

    -- Facultative : la durée d'une licence est un point ouvert (illimitée
    -- ou à renouveler). Recopiée de la dernière décision qui l'a fixée.
    licence_expire_le      DATE,

    -- Toujours un compte professionnel : il est créé, s'il n'existe pas,
    -- au moment de créer la structure (section 14).
    createur_compte_id     UUID NOT NULL,
    createur_type_compte   TEXT NOT NULL DEFAULT 'professionnel' CHECK (createur_type_compte = 'professionnel'),
    date_creation          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT structure_createur_professionnel
        FOREIGN KEY (createur_compte_id, createur_type_compte) REFERENCES compte (id, type_compte)
);

-- Limite anti-abus (M5) : une seule structure non validée par personne
-- (un compte professionnel par personne). Une structure abandonnée sort
-- de l'index et libère la place, sans rien effacer.
CREATE UNIQUE INDEX structure_une_non_validee_par_createur
    ON structure (createur_compte_id)
    WHERE statut IN ('brouillon', 'en_attente', 'refusee');

-- Une seule MUNASEB validée ou suspendue (M6). Les brouillons et demandes
-- en attente restent libres : sinon n'importe qui bloquerait le vrai chef
-- de la mutuelle avec un brouillon. Toutes les lignes de l'index ont la
-- même valeur ('munaseb') : il en admet donc au plus une.
CREATE UNIQUE INDEX structure_une_seule_munaseb_validee
    ON structure (type_structure)
    WHERE type_structure = 'munaseb' AND statut IN ('validee', 'suspendue');

-- ---------------------------------------------------------------------
-- Pièces justificatives
-- ---------------------------------------------------------------------
CREATE TABLE piece_justificative (
    id                 UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id       UUID NOT NULL REFERENCES structure (id),
    -- Liste fermée ; les pièces exigées par type de structure sont dans le
    -- code. Étendue par migration quand les listes hôpital, pharmacie,
    -- laboratoire et MUNASEB seront connues.
    type_piece         TEXT NOT NULL
                       CHECK (type_piece IN ('autorisation_creation', 'autorisation_ouverture_exploitation')),

    -- Conservé tel que saisi : l'équipe le compare au document.
    numero             TEXT NOT NULL CHECK (btrim(numero) <> ''),
    -- Forme de comparaison (M3, règle complétée le 2026-10-05), dans cet
    -- ordre :
    -- 1. majuscules, lettres ASCII seulement ;
    -- 2. espaces retirées : les 25 caractères Unicode White_Space
    --    (PropList.txt), la définition de `char::is_whitespace` en Rust ;
    -- 3. tirets convertis en tiret simple : les 30 autres caractères de la
    --    propriété Unicode Dash, liste reprise de `telephone.rs`. Les
    --    tirets restent : ils font partie du numéro officiel
    --    (2018-628/MS/CAB) ;
    -- 4. préfixe retiré : un « N » suivi de « ° », « º », « O » ou « . »,
    --    au début seulement. Les espaces ayant disparu à l'étape 2,
    --    « N ° 2018 » est couvert. Sans risque : un numéro d'arrêté commence
    --    par l'année, jamais par une lettre.
    --
    -- Résultat identique sur tous les serveurs (décisions N1 et du
    -- 2026-10-05) :
    -- - COLLATE "C" sur `upper` : la doc PostgreSQL 18 (24.2, Collation
    --   Support) dit que `upper` dépend de la collation, et que la collation
    --   C ne traite comme lettres que l'ASCII, avec un comportement « stable
    --   across all versions for a given database encoding ». `º` et `°`
    --   n'ont pas de majuscule et restent tels quels. La collation est
    --   ensuite héritée par les `regexp_replace`.
    -- - Listes explicites au lieu de `\s` : la doc (9.7.3.2) dit que
    --   l'appartenance d'un caractère non ASCII à [[:space:]] dépend de la
    --   collation et « can vary across platforms ».
    -- - Échappements longs `\UXXXXXXXX` (doc 9.7.3.3, « always taken as
    --   ordinary characters » entre crochets) : aucun caractère invisible ou
    --   trompeur dans ce fichier, qu'un éditeur pourrait modifier sans que
    --   personne le voie ; chaque caractère est listé un par un, sans plage.
    --   Pas la forme courte `\uXXXX` : certains outils d'édition la
    --   remplacent d'office par le caractère lui-même.
    --   Les listes sont coupées en plusieurs chaînes : deux chaînes
    --   séparées par un retour à la ligne sont concaténées (doc 4.1.2.1).
    --
    -- Doc 5.4 Generated Columns : la colonne ne peut pas être écrite
    -- directement, elle ne peut donc jamais diverger de `numero` ;
    -- l'expression ne peut utiliser que des fonctions immutables, faute de
    -- quoi la création de la table échoue. STORED explicite : VIRTUAL est
    -- le défaut en 18, et la doc ne dit rien de l'indexation d'une colonne
    -- virtuelle ; une colonne STORED est écrite sur disque comme une autre.
    numero_normalise   TEXT NOT NULL GENERATED ALWAYS AS (
                           regexp_replace(
                               regexp_replace(
                                   regexp_replace(
                                       upper(numero COLLATE "C"),
                                       '[\U00000009\U0000000A\U0000000B\U0000000C\U0000000D\U00000020\U00000085\U000000A0\U00001680'
                                       '\U00002000\U00002001\U00002002\U00002003\U00002004\U00002005\U00002006\U00002007\U00002008\U00002009\U0000200A'
                                       '\U00002028\U00002029\U0000202F\U0000205F\U00003000]',
                                       '', 'g'),
                                   '[\U0000058A\U000005BE\U00001400\U00001806\U00002010\U00002011\U00002012\U00002013\U00002014\U00002015'
                                   '\U00002053\U0000207B\U0000208B\U00002212\U00002E17\U00002E1A\U00002E3A\U00002E3B\U00002E40\U00002E5D'
                                   '\U0000301C\U00003030\U000030A0\U0000FE31\U0000FE32\U0000FE58\U0000FE63\U0000FF0D\U00010D6E\U00010EAD]',
                                   '-', 'g'),
                               '^N[°ºO.]', '')
                       ) STORED,
    date_piece         DATE NOT NULL,

    -- Type déterminé par les premiers octets du fichier, jamais par ce que
    -- le client envoie (section 14, OWASP File Upload).
    type_mime          TEXT NOT NULL CHECK (type_mime IN ('application/pdf', 'image/jpeg', 'image/png')),
    -- 5 Mio au plus (section 14).
    taille             INTEGER NOT NULL CHECK (taille BETWEEN 1 AND 5242880),
    -- Empreinte SHA-256 : toujours 32 octets.
    sha256             BYTEA NOT NULL CHECK (octet_length(sha256) = 32),

    -- Une pièce ne se modifie jamais : un remplacement (pendant le
    -- brouillon) ajoute une ligne et passe l'ancienne à faux. Rien n'est
    -- effacé, même après un refus ou un abandon.
    active             BOOLEAN NOT NULL DEFAULT true,
    date_depot         TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- N2 : un numéro réduit au seul préfixe (« N° ») n'est pas un numéro.
    CONSTRAINT piece_justificative_numero_normalise_non_vide CHECK (numero_normalise <> '')
);

-- Une seule pièce active par type et par structure ; l'index sert aussi à
-- lister les pièces d'une structure.
CREATE UNIQUE INDEX piece_justificative_une_active_par_type
    ON piece_justificative (structure_id, type_piece)
    WHERE active;

-- Avertissement à l'administrateur : même numéro dans une autre demande.
CREATE INDEX piece_justificative_numero_idx
    ON piece_justificative (type_piece, numero_normalise);

-- Contenu dans une table à part (M1) : une requête sur
-- `piece_justificative`, même un SELECT *, ne peut pas lire le fichier.
-- Seule la route de téléchargement lit cette table (section 14).
CREATE TABLE piece_contenu (
    piece_id   UUID PRIMARY KEY REFERENCES piece_justificative (id),
    contenu    BYTEA NOT NULL CHECK (octet_length(contenu) BETWEEN 1 AND 5242880)
);

-- ---------------------------------------------------------------------
-- Registre des autorisations validées : la clé primaire garantit qu'un
-- même numéro (par type de pièce) ne peut être validé qu'une fois. Rempli
-- dans la transaction de validation, à partir des pièces actives ; les
-- brouillons ne s'y trouvent jamais, ils ne peuvent donc bloquer personne.
-- ---------------------------------------------------------------------
CREATE TABLE autorisation_validee (
    type_piece         TEXT NOT NULL,
    numero_normalise   TEXT NOT NULL,
    structure_id       UUID NOT NULL REFERENCES structure (id),
    piece_id           UUID NOT NULL UNIQUE REFERENCES piece_justificative (id),
    date_enregistrement TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (type_piece, numero_normalise)
);

-- ---------------------------------------------------------------------
-- Décisions de l'équipe LaafiCare (validation, refus, suspension,
-- réactivation). Jamais modifiées ni supprimées (triggers plus bas).
-- ---------------------------------------------------------------------
CREATE TABLE decision_structure (
    id                         UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id               UUID NOT NULL REFERENCES structure (id),
    administrateur_compte_id   UUID NOT NULL,
    administrateur_type_compte TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                               CHECK (administrateur_type_compte = 'administrateur_laaficare'),
    decision                   TEXT NOT NULL CHECK (decision IN ('validee', 'refusee', 'suspendue', 'reactivee')),
    -- Obligatoire pour un refus ou une suspension, absent sinon. Longueur
    -- maximale contrôlée par le code, comme le motif de rejet d'une demande.
    motif                      TEXT,
    -- Fixée à la validation, ou redéfinie à la réactivation (M8).
    licence_expire_le          DATE,
    date_decision              TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT decision_structure_administrateur
        FOREIGN KEY (administrateur_compte_id, administrateur_type_compte) REFERENCES compte (id, type_compte),
    CONSTRAINT decision_structure_motif
        CHECK ((decision IN ('refusee', 'suspendue')) = (motif IS NOT NULL AND btrim(motif) <> '')),
    CONSTRAINT decision_structure_licence
        CHECK (licence_expire_le IS NULL OR decision IN ('validee', 'reactivee'))
);

CREATE INDEX decision_structure_structure_idx
    ON decision_structure (structure_id, date_decision);

-- Même mécanisme que la migration 0010 (doc PostgreSQL 18, CREATE
-- TRIGGER : TRUNCATE ne déclenche pas les triggers ligne par ligne, d'où
-- un second trigger FOR EACH STATEMENT). Protège contre une erreur, pas
-- contre un administrateur de la base (section 12).
CREATE FUNCTION decision_structure_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'decision_structure : les lignes ne sont jamais modifiées ni supprimées (%)', TG_OP;
END;
$$;

CREATE TRIGGER decision_structure_sans_modification
    BEFORE UPDATE OR DELETE ON decision_structure
    FOR EACH ROW EXECUTE FUNCTION decision_structure_non_modifiable();

CREATE TRIGGER decision_structure_sans_troncature
    BEFORE TRUNCATE ON decision_structure
    FOR EACH STATEMENT EXECUTE FUNCTION decision_structure_non_modifiable();

-- ---------------------------------------------------------------------
-- Affectations : compte professionnel + structure + rôle. Les rôles permis
-- par type de structure sont une liste fermée dans le code (section 14).
-- ---------------------------------------------------------------------
CREATE TABLE affectation (
    id                        UUID PRIMARY KEY DEFAULT uuidv7(),
    compte_id                 UUID NOT NULL,
    compte_type               TEXT NOT NULL DEFAULT 'professionnel' CHECK (compte_type = 'professionnel'),
    structure_id              UUID NOT NULL REFERENCES structure (id),
    role                      TEXT NOT NULL CHECK (role IN (
                                  'responsable', 'directeur', 'chef_de_service', 'major', 'medecin',
                                  'infirmier', 'agent_accueil', 'pharmacien', 'biologiste',
                                  'agent_assurance_munaseb')),
    statut                    TEXT NOT NULL DEFAULT 'active' CHECK (statut IN ('active', 'desactivee')),
    date_creation             TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Une désactivation est toujours datée et attribuée. Réactiver = une
    -- nouvelle ligne (M7) : celle-ci reste désactivée, rien ne s'efface.
    desactivee_le             TIMESTAMPTZ,
    desactivee_par_compte_id  UUID REFERENCES compte (id),

    CONSTRAINT affectation_compte_professionnel
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte),
    CONSTRAINT affectation_desactivation_tracee
        CHECK ((statut = 'active') = (desactivee_le IS NULL AND desactivee_par_compte_id IS NULL)),
    CONSTRAINT affectation_desactivation_complete
        CHECK ((desactivee_le IS NULL) = (desactivee_par_compte_id IS NULL)),
    -- Cible de la clé étrangère composite de `invitation` : une invitation
    -- acceptée ne peut désigner qu'une affectation de sa structure et de
    -- son rôle (même principe que compte_id_type_unique).
    CONSTRAINT affectation_id_structure_role_unique UNIQUE (id, structure_id, role)
);

-- Au plus une affectation active pour un même rôle dans une même
-- structure ; plusieurs rôles différents restent possibles (section 14).
-- L'index sert aussi au choix de l'affectation après connexion.
CREATE UNIQUE INDEX affectation_une_active_par_role
    ON affectation (compte_id, structure_id, role)
    WHERE statut = 'active';

CREATE INDEX affectation_structure_idx ON affectation (structure_id);

-- ---------------------------------------------------------------------
-- Invitations, enregistrées au numéro sans rien chercher (section 14).
-- Soit à un rôle dans une structure, soit à devenir administrateur
-- LaafiCare (sans structure).
-- ---------------------------------------------------------------------
CREATE TABLE invitation (
    id                     UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id           UUID REFERENCES structure (id),
    role                   TEXT CHECK (role IN (
                               'responsable', 'directeur', 'chef_de_service', 'major', 'medecin',
                               'infirmier', 'agent_accueil', 'pharmacien', 'biologiste',
                               'agent_assurance_munaseb')),
    pour_administrateur    BOOLEAN NOT NULL DEFAULT false,
    -- Déjà normalisé par `telephone.rs`.
    telephone              TEXT NOT NULL,
    -- Compte professionnel (responsable, chef de service, major) ou
    -- administrateur : clé étrangère simple.
    invite_par_compte_id   UUID NOT NULL REFERENCES compte (id),
    -- `expiree` n'est jamais écrit (M4) : une invitation en attente dont
    -- `expire_le` est passé est expirée, déduit à la lecture.
    statut                 TEXT NOT NULL DEFAULT 'en_attente'
                           CHECK (statut IN ('en_attente', 'acceptee', 'refusee', 'annulee')),
    date_creation          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Fixée à la création (7 jours, constante du code) : changer la
    -- constante ne modifie jamais les invitations existantes.
    expire_le              TIMESTAMPTZ NOT NULL,
    date_reponse           TIMESTAMPTZ,
    -- Affectation créée par l'acceptation : avec `invite_par_compte_id` et
    -- `date_reponse`, elle dit toujours qui a fait entrer un professionnel
    -- dans une structure, et quand (décision du 2026-10-04). UNIQUE : une
    -- affectation naît d'une seule invitation.
    affectation_id         UUID UNIQUE,

    -- La clé composite garantit que l'affectation liée porte la structure
    -- et le rôle de l'invitation. Doc PostgreSQL 18, 5.5.5 : sans MATCH
    -- FULL, « a referencing row need not satisfy the foreign key
    -- constraint if any of its referencing columns are null » -- le cas
    -- avant l'acceptation, et pour une invitation administrateur.
    CONSTRAINT invitation_affectation
        FOREIGN KEY (affectation_id, structure_id, role) REFERENCES affectation (id, structure_id, role),
    -- Présente si et seulement si une invitation à un rôle est acceptée.
    -- Une invitation administrateur acceptée crée un compte, pas une
    -- affectation.
    CONSTRAINT invitation_affectation_si_acceptee
        CHECK ((statut = 'acceptee' AND NOT pour_administrateur) = (affectation_id IS NOT NULL)),
    CONSTRAINT invitation_cible
        CHECK ((pour_administrateur AND structure_id IS NULL AND role IS NULL)
            OR (NOT pour_administrateur AND structure_id IS NOT NULL AND role IS NOT NULL)),
    CONSTRAINT invitation_expiration CHECK (expire_le > date_creation),
    CONSTRAINT invitation_reponse_datee CHECK ((statut = 'en_attente') = (date_reponse IS NULL))
);

-- La personne invitée retrouve ses invitations depuis son compte patient.
CREATE INDEX invitation_telephone_idx ON invitation (telephone, statut);
-- Plafond d'invitations par structure et par jour : un comptage.
CREATE INDEX invitation_structure_idx ON invitation (structure_id, date_creation);
