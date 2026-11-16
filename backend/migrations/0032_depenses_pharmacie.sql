-- Dépenses d'une pharmacie (besoin du porteur du 2026-11-16, absent du
-- CDC : bénéfice net de la pharmacie ; CLAUDE.md §15).
--
-- Un journal par pharmacie : date, catégorie, montant en FCFA entiers,
-- libellé, auteur. **Jamais modifié ni effacé** : une correction se fait par
-- une écriture d'annulation tracée (montant opposé, même date et même
-- catégorie que la dépense annulée, motif obligatoire), comme pour le
-- stock (migration 0031).
--
-- Catégories : liste fermée donnée par le porteur. Les impôts et taxes sont
-- une dépense saisie : aucun calcul d'impôt.
--
-- Auteur : le **responsable** de la pharmacie seulement (décision du
-- porteur, 2026-11-16), garanti par la base : clé étrangère composite
-- MATCH FULL vers `affectation (id, structure_id, role)` et rôle
-- `responsable` (comme `prix_vente_pharmacie`, migration 0031).

CREATE TABLE depense_pharmacie (
    id                     UUID PRIMARY KEY DEFAULT uuidv7(),
    structure_id           UUID NOT NULL REFERENCES structure (id),
    -- Date de la dépense (date du Burkina), jamais dans le futur : contrôlé
    -- par le code à l'heure du Burkina.
    date_depense           DATE NOT NULL,
    categorie              TEXT NOT NULL CHECK (categorie IN ('loyer', 'salaires', 'electricite', 'eau', 'transport',
                                                              'impots_taxes', 'entretien', 'autres')),
    -- Positif pour une dépense, négatif pour son annulation.
    montant_fcfa           BIGINT NOT NULL CHECK (montant_fcfa <> 0),
    libelle                TEXT NOT NULL CHECK (char_length(libelle) BETWEEN 1 AND 200),
    annule_depense_id      UUID UNIQUE,
    motif_annulation       TEXT CHECK (char_length(motif_annulation) BETWEEN 1 AND 1000),
    auteur_affectation_id  UUID NOT NULL,
    auteur_structure_id    UUID NOT NULL,
    auteur_role            TEXT NOT NULL CHECK (auteur_role = 'responsable'),
    date_creation          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT depense_id_structure UNIQUE (id, structure_id),
    CONSTRAINT depense_auteur FOREIGN KEY (auteur_affectation_id, auteur_structure_id, auteur_role)
        REFERENCES affectation (id, structure_id, role) MATCH FULL,
    CONSTRAINT depense_par_sa_pharmacie CHECK (auteur_structure_id = structure_id),
    -- Une annulation vise une dépense de la même pharmacie.
    CONSTRAINT depense_annulee_de_sa_pharmacie FOREIGN KEY (annule_depense_id, structure_id)
        REFERENCES depense_pharmacie (id, structure_id),
    CONSTRAINT depense_sens CHECK (
        (annule_depense_id IS NULL AND montant_fcfa > 0 AND motif_annulation IS NULL)
        OR (annule_depense_id IS NOT NULL AND montant_fcfa < 0 AND motif_annulation IS NOT NULL)
    )
);

CREATE INDEX depense_pharmacie_structure_date_idx ON depense_pharmacie (structure_id, date_depense);

CREATE TRIGGER depense_pharmacie_sans_modification BEFORE UPDATE OR DELETE ON depense_pharmacie
    FOR EACH ROW EXECUTE FUNCTION stock_non_modifiable();
CREATE TRIGGER depense_pharmacie_sans_troncature BEFORE TRUNCATE ON depense_pharmacie
    FOR EACH STATEMENT EXECUTE FUNCTION stock_non_modifiable();

-- Droits (suite de P1, migration 0026) : le serveur lit et ajoute, jamais
-- ne modifie ni n'efface.
REVOKE UPDATE, DELETE ON depense_pharmacie FROM laaficare_serveur;
