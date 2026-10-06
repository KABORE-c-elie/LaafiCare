-- ContratAssurance MUNASEB (section 12 du CLAUDE.md), nommé
-- contrat_assurance_munaseb -- même raison que agent_assurance_munaseb :
-- MUNASEB est câblée en dur, pas un module Assurance générique.
--
-- Champs basés uniquement sur les sources MUNASEB réelles (docs/), pas sur
-- une supposition : aucune classe "Contrat"/"ContratAssurance" n'existe
-- dans docs/diagrammes/diagramme_classes_munaseb oki11.drawio -- son
-- équivalent réel est la classe carteAdhesion (id RE4hV46S5Z5vCYrV1-wb-72,
-- champs id/nom/prenom/dateNaiss/lieuNaiss/numeroCarte/UFR/Université/
-- NumMatricule/dateEffet/dateExpiration/telephone/persoprevenir/qrCode).
-- Le dictionnaire de données de docs/Memoire_Soutenance1.docx confirme
-- numeroCarte (Caractère 30), dateEmission/dateExpiration ("1 an").
--
-- Champs déjà portés par `patient` (nom, prénom, téléphone, date/lieu de
-- naissance) non dupliqués ici -- patient_id en FK suffit, même principe
-- que patient/agent_assurance_munaseb (pas d'héritage à clé partagée).
--
-- Aucun champ statut/suspension : vérifié dans les deux sources MUNASEB,
-- absent de carteAdhesion et de Mutualiste. `statut` n'existe que sur
-- PartenaireSante ("actif, suspendu") et sur les demandes -- jamais sur la
-- carte du mutualiste. La validité du contrat se lit uniquement via
-- date_expiration, comme dans le système source.
--
-- Aucun champ montant_consomme : le mémoire décrit une "vérification
-- automatique du solde disponible" (ligne 826), calculée au moment de la
-- demande de remboursement, pas un compteur stocké sur la carte -- ce
-- calcul viendra avec DemandeRemboursement (prochaine étape, section 12).
--
-- Plafond (100 000 FCFA) et taux (80%) sont des paramètres MUNASEB
-- identiques pour tout mutualiste (mémoire, lignes 64/567/850-852 :
-- "Paramétrage du plafond de remboursement ; Définition des taux de
-- remboursement" par l'Administrateur) -- pas des colonnes par contrat,
-- des constantes dans le code (à côté du service qui les utilisera).
CREATE TABLE contrat_assurance_munaseb (
    id                  UUID PRIMARY KEY DEFAULT uuidv7(),
    patient_id          UUID NOT NULL UNIQUE REFERENCES patient (id),

    -- carteAdhesion.numeroCarte (Caractère 30, mémoire ligne 1535-1538).
    numero_carte        TEXT NOT NULL UNIQUE,

    -- carteAdhesion.UFR / Université / NumMatricule -- spécifique à
    -- MUNASEB (mutuelle étudiante), pas des champs de patient générique.
    ufr                 TEXT NOT NULL,
    universite          TEXT NOT NULL,
    num_matricule       TEXT NOT NULL,

    -- carteAdhesion.dateEffet / dateExpiration ("carte d'1 an", mémoire
    -- ligne 1545-1547).
    date_effet          DATE NOT NULL,
    date_expiration     DATE NOT NULL,

    -- carteAdhesion.persoprevenir -- personne à prévenir en cas de besoin.
    -- Nullable : pas systématiquement renseigné à l'adhésion.
    personne_a_prevenir TEXT,

    date_creation        TIMESTAMPTZ NOT NULL DEFAULT now()
);
