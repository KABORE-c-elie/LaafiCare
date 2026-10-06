-- DemandeRemboursement MUNASEB, circuit tiers payant (section 12 du
-- CLAUDE.md, "Décisions actées -- DemandeRemboursement (2026-09-24)").
-- Sources : docs/Memoire_Soutenance1.docx et
-- docs/diagrammes/diagramme_classes_munaseb oki11.drawio uniquement.

-- ---------------------------------------------------------------------
-- Partenaire de santé : version minimale (précision de périmètre
-- 2026-09-24 -- les agents gèreront les partenaires avec les modules
-- Pharmacie/Laboratoire). Champs du dictionnaire de données du mémoire
-- (l. 1490-1513) : nomPartenaire, typePartenaire, villePartenaire,
-- telephone, statutPartenaire. Aucune donnée insérée ici : une migration
-- s'exécute aussi en production, les partenaires de test sont créés par
-- les tests eux-mêmes.
-- ---------------------------------------------------------------------
CREATE TABLE partenaire_sante_munaseb (
    id        UUID PRIMARY KEY DEFAULT uuidv7(),
    nom       TEXT NOT NULL,
    -- Pas de CHECK : le mémoire donne une liste ouverte ("pharmacie,
    -- hôpital…", l. 1496), pas une énumération fermée.
    type      TEXT NOT NULL,
    ville     TEXT NOT NULL,
    -- Même format que utilisateur.telephone (migration 0001).
    telephone TEXT NOT NULL CHECK (telephone ~ '^\+[1-9][0-9]{6,14}$'),
    -- "État du partenariat (actif, suspendu)", mémoire l. 1510-1511.
    statut    TEXT NOT NULL DEFAULT 'actif' CHECK (statut IN ('actif', 'suspendu'))
);

-- ---------------------------------------------------------------------
-- Grille tarifaire par type d'acte. La LISTE des types est fermée et
-- définie par le code (ce CHECK + l'enum Rust) ; ajouter un type = une
-- migration. Les VALEURS sont des données, modifiables plus tard par les
-- agents : elles n'existent qu'ici, jamais en constante Rust (précision de
-- périmètre 2026-09-24).
-- ---------------------------------------------------------------------
CREATE TABLE tarif_acte_munaseb (
    type_acte    TEXT PRIMARY KEY CHECK (type_acte IN (
                     'consultation', 'hospitalisation', 'pharmacie', 'laboratoire', 'lunetterie')),
    taux_percent SMALLINT CHECK (taux_percent BETWEEN 0 AND 100),
    forfait_fcfa INTEGER  CHECK (forfait_fcfa > 0),
    -- Un tarif est soit un taux, soit un forfait, jamais les deux ni aucun.
    CONSTRAINT taux_ou_forfait CHECK ((taux_percent IS NULL) <> (forfait_fcfa IS NULL))
);

-- Seuls les cinq types dont le mémoire donne la valeur. Les autres soins
-- couverts (dentaire/prothèses, accouchement/CPN, chirurgie, frais
-- funéraires, radiologie) n'ont ni montant de forfait ni taux dans les
-- sources -- ajoutés par migration quand les valeurs seront connues.
INSERT INTO tarif_acte_munaseb (type_acte, taux_percent, forfait_fcfa) VALUES
    -- "gratuité de consultation" (l. 489) -> prise en charge à 100 %.
    ('consultation',    100, NULL),
    -- "prise en charge à 100% de frais d'hospitalisations" (l. 489).
    ('hospitalisation', 100, NULL),
    -- "subvention à hauteur de 80% des frais de pharmacie, d'analyses
    -- médicales et de laboratoire" (l. 489).
    ('pharmacie',        80, NULL),
    ('laboratoire',      80, NULL),
    -- "pour les soins lunetteries la mutuelle ne rembourse que 15000
    -- francs CFA" (l. 502).
    ('lunetterie',     NULL, 15000);

-- ---------------------------------------------------------------------
-- Demande de remboursement. Champs propres du diagramme (classe
-- DemandeRemboursement) : montantDemande, montantRembourse, dateDemande,
-- statut, + motif de rejet (rejeter(motif)). Les champs d'identité du
-- mutualiste (nom, NumCarte, UFR...) ne sont pas dupliqués : ils sont déjà
-- sur patient / contrat_assurance_munaseb.
-- ---------------------------------------------------------------------
CREATE TABLE demande_remboursement_munaseb (
    id                     UUID PRIMARY KEY DEFAULT uuidv7(),
    contrat_id             UUID NOT NULL REFERENCES contrat_assurance_munaseb (id),
    partenaire_id          UUID NOT NULL REFERENCES partenaire_sante_munaseb (id),
    type_acte              TEXT NOT NULL REFERENCES tarif_acte_munaseb (type_acte),

    -- Identifiant de l'acte chez le partenaire : rend creer_depuis_acte
    -- idempotent (contrainte UNIQUE plus bas) si le module appelant
    -- rejoue l'appel après un timeout.
    reference_acte         TEXT NOT NULL,
    date_acte              DATE NOT NULL,

    -- Coût total de l'acte, en FCFA entiers (pas de sous-unité en
    -- circulation).
    montant_acte_fcfa      INTEGER NOT NULL CHECK (montant_acte_fcfa > 0),

    -- montantDemande : part MUNASEB calculée à la création (taux ou
    -- forfait, arrondi à l'entier le plus proche). Figé : une modification
    -- ultérieure du tarif ne le touche jamais.
    montant_demande_fcfa   INTEGER NOT NULL CHECK (montant_demande_fcfa >= 0),

    -- montantRembourse : montant_demande plafonné au solde restant, fixé
    -- au passage à 'valide'. NULL avant (voir contrainte plus bas).
    montant_rembourse_fcfa INTEGER CHECK (montant_rembourse_fcfa >= 0),

    -- Cycle En attente -> En cours -> Validé / Rejeté -> Payé (mémoire
    -- l. 815), statut initial En attente (décision actée).
    statut                 TEXT NOT NULL DEFAULT 'en_attente'
                           CHECK (statut IN ('en_attente', 'en_cours', 'valide', 'rejete', 'paye')),
    motif_rejet            TEXT,
    date_demande           TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT reference_acte_unique_par_partenaire UNIQUE (partenaire_id, reference_acte),

    -- Seuls Validé et Payé consomment le plafond (décision actée) : ce sont
    -- exactement les statuts où le montant remboursé est connu.
    CONSTRAINT montant_rembourse_si_valide_ou_paye
        CHECK ((statut IN ('valide', 'paye')) = (montant_rembourse_fcfa IS NOT NULL)),

    -- rejeter(motif) dans le diagramme : un rejet porte toujours un motif.
    CONSTRAINT motif_si_rejete
        CHECK (statut <> 'rejete' OR motif_rejet IS NOT NULL)
);

-- PostgreSQL n'indexe pas automatiquement une clé étrangère. contrat_id
-- sert au calcul du solde restant (somme des demandes Validé/Payé d'un
-- contrat).
CREATE INDEX demande_remboursement_munaseb_contrat_idx
    ON demande_remboursement_munaseb (contrat_id);
