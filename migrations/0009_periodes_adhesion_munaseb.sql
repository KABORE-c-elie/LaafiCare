-- Périodes d'adhésion MUNASEB (section 12 du CLAUDE.md, "Périodes
-- d'adhésion et renouvellement"). Chaque période est conservée : un
-- renouvellement crée une nouvelle période au lieu d'écraser les dates du
-- contrat. Appui dans les sources : le réabonnement du mémoire a ses
-- propres dateDebut/dateFin (dictionnaire l. 1555-1562), et le diagramme a
-- une classe Reabonnement.

-- Doc PostgreSQL 18, "btree_gist" : module "trusted", installable par un
-- utilisateur qui a le droit CREATE sur la base. Il permet de combiner une
-- égalité sur une colonne ordinaire (contrat_id) et le chevauchement de
-- dates dans une même contrainte d'exclusion.
CREATE EXTENSION IF NOT EXISTS btree_gist;

CREATE TABLE periode_adhesion_munaseb (
    id            UUID PRIMARY KEY DEFAULT uuidv7(),
    contrat_id    UUID NOT NULL REFERENCES contrat_assurance_munaseb (id),

    -- En V1, date du jour fixée par le code au moment de l'enregistrement,
    -- jamais fournie par l'appelant (section 12). Elle décide si un
    -- renouvellement est dans les temps ou en retard (carence).
    date_paiement DATE NOT NULL,
    date_debut    DATE NOT NULL,
    date_fin      DATE NOT NULL,
    date_creation TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT periode_dates_ordonnees CHECK (date_fin >= date_debut),

    -- Vrai dans tous les cas prévus : carence (début = paiement + 1 mois),
    -- renouvellement dans les temps (début = lendemain de la fin, après le
    -- paiement), et contrats repris ci-dessous (paiement = début).
    CONSTRAINT paiement_avant_debut CHECK (date_paiement <= date_debut),

    -- Deux périodes d'un même contrat -- donc d'un même patient, puisque
    -- contrat_assurance_munaseb.patient_id est UNIQUE -- ne se chevauchent
    -- jamais. Forme reprise de l'exemple officiel (doc PostgreSQL 18,
    -- "Range Types", section "Constraints on Ranges") :
    -- EXCLUDE USING GIST (room WITH =, during WITH &&).
    -- '[]' : un daterange exclut par défaut sa borne haute ([), même doc) ;
    -- les deux bornes incluses font compter le dernier jour de la période.
    -- Deux périodes contiguës (fin = J, début suivant = J + 1) restent
    -- autorisées.
    --
    -- Pas de CHECK sur la durée de 12 mois (décision R3) : elle est
    -- calculée par une seule fonction Rust, pour éviter qu'un écart entre
    -- l'arithmétique des mois de PostgreSQL et celle de chrono ne fasse
    -- refuser une date correcte.
    CONSTRAINT periodes_sans_chevauchement
        EXCLUDE USING gist (contrat_id WITH =, daterange(date_debut, date_fin, '[]') WITH &&)
);
-- Pas d'index séparé sur contrat_id : l'index GiST de la contrainte
-- d'exclusion commence par contrat_id et sert aussi aux recherches par
-- contrat.

-- ---------------------------------------------------------------------
-- Reprise des données : une période par contrat existant, avec ses dates
-- actuelles. La date de paiement réelle est inconnue : on prend la date
-- d'effet, ce qui respecte paiement_avant_debut.
-- ---------------------------------------------------------------------
INSERT INTO periode_adhesion_munaseb (contrat_id, date_paiement, date_debut, date_fin)
SELECT id, date_effet, date_effet, date_expiration
FROM contrat_assurance_munaseb;

-- ---------------------------------------------------------------------
-- Rattachement des demandes à leur période. À ce stade, chaque contrat n'a
-- qu'une période : la correspondance est directe.
-- ---------------------------------------------------------------------
ALTER TABLE demande_remboursement_munaseb
    ADD COLUMN periode_id UUID REFERENCES periode_adhesion_munaseb (id);

UPDATE demande_remboursement_munaseb d
SET periode_id = p.id
FROM periode_adhesion_munaseb p
WHERE p.contrat_id = d.contrat_id;

ALTER TABLE demande_remboursement_munaseb
    ALTER COLUMN periode_id SET NOT NULL;

-- periode_id remplace contrat_id (décision validée) : la période donne le
-- contrat, garder les deux ferait deux sources qui pourraient se
-- contredire. Supprimer la colonne supprime aussi son index
-- (demande_remboursement_munaseb_contrat_idx).
ALTER TABLE demande_remboursement_munaseb
    DROP COLUMN contrat_id;

-- Sert au calcul du solde d'une période (somme des demandes Validé/Payé) et
-- au verrou de la future validation.
CREATE INDEX demande_remboursement_munaseb_periode_idx
    ON demande_remboursement_munaseb (periode_id);

-- Les dates vivent désormais dans les périodes.
ALTER TABLE contrat_assurance_munaseb
    DROP COLUMN date_effet,
    DROP COLUMN date_expiration;
