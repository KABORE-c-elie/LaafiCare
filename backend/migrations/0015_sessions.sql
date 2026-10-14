-- Sessions et jetons de renouvellement (option B, décidée le 2026-10-12).
-- Sources : RFC 9700 §2.2.2 et §4.14.2 (une application installée utilise
-- la rotation des jetons de renouvellement ; un jeton déjà utilisé qui
-- revient est un rejeu, la session est révoquée ; expiration après
-- inactivité) ; NIST SP 800-63B rév. 4 §2.1.3 et §2.2.3 (durées). Durées
-- et règles de purge : décisions du porteur, CLAUDE.md section 14. Aucune
-- donnée sur l'appareil n'est enregistrée.

-- Cible de la clé composite `session_affectation_du_compte` ci-dessous ;
-- elle doit exister avant elle. Ajouter une contrainte UNIQUE construit un
-- index : aucune ligne n'est modifiée, le trigger qui protège les
-- affectations (migration 0012, sur UPDATE) ne se déclenche pas.
ALTER TABLE affectation
    ADD CONSTRAINT affectation_id_compte_unique UNIQUE (id, compte_id);

-- Une session par connexion, donc par appareil. Elle porte la durée
-- absolue et la révocation ; le `sid` du jeton d'accès la désigne, et les
-- extracteurs la vérifient à chaque requête.
CREATE TABLE session (
    id                UUID PRIMARY KEY DEFAULT uuidv7(),
    -- Sans ON DELETE CASCADE (Q-a) : un compte ne s'efface jamais, une
    -- suppression bloquée avertirait.
    compte_id         UUID NOT NULL REFERENCES compte (id),
    -- Affectation active d'un professionnel en exercice ; vide pour un
    -- patient, un administrateur ou un professionnel sans affectation.
    affectation_id    UUID,
    creee_le          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Durée absolue : 30 jours (patient), 12 h (professionnel), 24 h
    -- (administrateur). Au-delà, le mot de passe est redemandé.
    expire_le         TIMESTAMPTZ NOT NULL,
    revoquee_le       TIMESTAMPTZ,
    motif_revocation  TEXT CHECK (motif_revocation IN (
                          'deconnexion', 'deconnexion_partout',
                          'reinitialisation_mot_de_passe', 'rejeu_detecte',
                          'changement_affectation', 'desactivation_compte')),

    CONSTRAINT session_duree_positive CHECK (expire_le > creee_le),
    CONSTRAINT session_revocation_complete
        CHECK ((revoquee_le IS NULL) = (motif_revocation IS NULL)),
    -- L'affectation appartient au compte de la session (voir la contrainte
    -- ajoutée à `affectation` ci-dessus). MATCH SIMPLE par défaut : une
    -- affectation vide n'est pas contrôlée.
    CONSTRAINT session_affectation_du_compte
        FOREIGN KEY (affectation_id, compte_id) REFERENCES affectation (id, compte_id)
);

-- Révocation de toutes les sessions d'un compte (déconnexion partout,
-- réinitialisation du mot de passe).
CREATE INDEX session_compte_idx ON session (compte_id);
-- Purge (Q-b) : sessions terminées depuis plus de 30 jours, ou 6 mois pour
-- `rejeu_detecte`.
CREATE INDEX session_expire_idx ON session (expire_le);
CREATE INDEX session_revoquee_idx ON session (revoquee_le) WHERE revoquee_le IS NOT NULL;

-- Jetons de renouvellement : jamais en clair, seulement l'empreinte SHA-256
-- de 32 octets aléatoires. Au-delà de 112 bits, NIST SP 800-63B rév. 4
-- §3.1.2 admet un hachage simple (même choix que les liens de
-- réinitialisation, R1). La purge d'une session efface ses jetons.
CREATE TABLE jeton_renouvellement (
    id               UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id       UUID NOT NULL REFERENCES session (id) ON DELETE CASCADE,
    empreinte        BYTEA NOT NULL UNIQUE CHECK (octet_length(empreinte) = 32),
    cree_le          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Inactivité : 30 jours (patient) ou 1 h (professionnel,
    -- administrateur). Jamais au-delà de session.expire_le : la base ne
    -- peut pas le garantir (un CHECK ne lit pas une autre table), le code
    -- le borne avec LEAST et un test le vérifie.
    expire_le        TIMESTAMPTZ NOT NULL,
    -- Rotation : date d'usage, et jeton qui l'a remplacé. Après une
    -- réutilisation tolérée, pointe vers le dernier jeton émis, pour que la
    -- chaîne reste traçable. Sans ON DELETE : action par défaut NO ACTION,
    -- qui ne refuse que si la suppression « would create a foreign key
    -- constraint violation », contrairement à RESTRICT, qui refuse même
    -- quand l'état final serait valide (doc PostgreSQL 18, CREATE TABLE).
    -- Supprimer en cascade tous les jetons d'une session ne laisse aucun
    -- lien cassé : ce n'est donc pas bloqué (vérifié par un test).
    utilise_le       TIMESTAMPTZ,
    remplace_par_id  UUID REFERENCES jeton_renouvellement (id),
    -- Tolérance de 30 s : date de la réutilisation unique de ce jeton déjà
    -- utilisé, quand son successeur n'avait jamais servi.
    reutilise_le     TIMESTAMPTZ,
    -- Successeur jamais reçu, annulé par la tolérance. S'il revient, c'est
    -- un rejeu : la session est révoquée.
    revoque_le       TIMESTAMPTZ,

    CONSTRAINT jeton_duree_positive CHECK (expire_le > cree_le),
    CONSTRAINT jeton_rotation_complete
        CHECK ((utilise_le IS NULL) = (remplace_par_id IS NULL)),
    CONSTRAINT jeton_reutilise_apres_usage
        CHECK (reutilise_le IS NULL OR utilise_le IS NOT NULL),
    -- Un jeton annulé par la tolérance n'a jamais servi.
    CONSTRAINT jeton_revoque_jamais_utilise
        CHECK (revoque_le IS NULL OR utilise_le IS NULL)
);
CREATE INDEX jeton_renouvellement_session_idx ON jeton_renouvellement (session_id);
