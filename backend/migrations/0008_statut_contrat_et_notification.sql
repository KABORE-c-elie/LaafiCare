-- ---------------------------------------------------------------------
-- Statut du contrat MUNASEB. Décision propre à LaafiCare (porteur,
-- 2026-09-24), pas issue des sources MUNASEB : la carte d'adhésion du
-- mémoire n'a qu'une date d'expiration (voir migration 0006). Nécessaire
-- pour que creer_depuis_acte renvoie NonCouvert sur un contrat suspendu.
-- Les contrats existants restent actifs (valeur par défaut). Aucune route
-- de suspension n'existe encore.
-- ---------------------------------------------------------------------
ALTER TABLE contrat_assurance_munaseb
    ADD COLUMN statut TEXT NOT NULL DEFAULT 'actif'
        CHECK (statut IN ('actif', 'suspendu'));

-- ---------------------------------------------------------------------
-- Notification enregistrée en base, affichée par l'app patient (BF-11,
-- avec l'exception assumée de la section 12 du CLAUDE.md : pas de SMS pour
-- les notifications de remboursement). Table générique : brique de la
-- plateforme, réutilisable par les futurs modules (résultats, CPN).
-- Champs repris de la classe Notification du diagramme MUNASEB : id,
-- user_id, titre, message, type, dateEnvoi, lu.
--
-- Push futur : s'ajoutera par-dessus (colonne push_envoye_le + traitement
-- qui lit les notifications non poussées) sans changer les appelants.
-- ---------------------------------------------------------------------
CREATE TABLE notification (
    id                       UUID PRIMARY KEY DEFAULT uuidv7(),

    -- Identité du destinataire (utilisateur.id), pas patient.id : c'est le
    -- `sub` du JWT, donc la clé avec laquelle l'app filtrera ses
    -- notifications.
    utilisateur_id           UUID NOT NULL REFERENCES utilisateur (id),

    -- Liste fermée, étendue par migration à chaque nouvelle source.
    type                     TEXT NOT NULL CHECK (type IN ('remboursement_statut')),

    -- Texte minimal, sans information de santé (type d'acte, nom du
    -- partenaire) : un push futur s'afficherait sur l'écran verrouillé.
    titre                    TEXT NOT NULL,
    message                  TEXT NOT NULL,

    -- Lien vers l'objet concerné : une clé étrangère facultative par type
    -- d'objet, pour que la base garantisse son existence.
    demande_remboursement_id UUID REFERENCES demande_remboursement_munaseb (id),

    lu                       BOOLEAN NOT NULL DEFAULT false,
    date_creation            TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Requête attendue de l'app : les notifications d'un utilisateur, les plus
-- récentes d'abord.
CREATE INDEX notification_utilisateur_idx
    ON notification (utilisateur_id, date_creation DESC);
