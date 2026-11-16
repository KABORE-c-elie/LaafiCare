-- Catalogue national des médicaments (pharmacie, étape 3 ; CLAUDE.md §15,
-- décision D6 du porteur, 2026-11-16). Un seul catalogue pour toutes les
-- pharmacies, tenu par l'équipe LaafiCare avec l'outil local
-- `importer_catalogue` (jamais une route HTTP). Aucune donnée ici : le
-- catalogue reste vide tant que le fichier officiel (liste nationale des
-- médicaments essentiels 2023) n'est pas dans `docs/Reglementation, loi et
-- reference/`.
--
-- BF-05 (CDC) : « chaque médicament du catalogue doit être associé à sa
-- DCI (molécule active) » ; une association de molécules porte plusieurs
-- DCI (table `medicament_dci`).

-- Trace de chaque import : qui, quand, depuis quel document et quel
-- fichier (empreinte). Jamais modifiée ni supprimée.
CREATE TABLE import_catalogue (
    id                          UUID PRIMARY KEY DEFAULT uuidv7(),
    administrateur_compte_id    UUID NOT NULL,
    administrateur_type_compte  TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                                CHECK (administrateur_type_compte = 'administrateur_laaficare'),
    -- Document officiel d'où viennent les lignes (titre, édition, texte de
    -- référence) : obligatoire, garde-fou de longueur.
    source                      TEXT NOT NULL CHECK (btrim(source) <> '' AND char_length(source) <= 300),
    fichier_sha256              BYTEA NOT NULL CHECK (octet_length(fichier_sha256) = 32),
    lignes_lues                 INTEGER NOT NULL CHECK (lignes_lues >= 0),
    medicaments_ajoutes         INTEGER NOT NULL CHECK (medicaments_ajoutes >= 0),
    date_import                 TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT import_catalogue_administrateur
        FOREIGN KEY (administrateur_compte_id, administrateur_type_compte) REFERENCES compte (id, type_compte)
);

-- Une DCI par molécule. `cle` : le nom ramené à une forme de comparaison
-- par le code (minuscules, espaces uniques), pour qu'un second import ne
-- crée pas de doublon.
CREATE TABLE dci (
    id         UUID PRIMARY KEY DEFAULT uuidv7(),
    nom        TEXT NOT NULL CHECK (nom = btrim(nom) AND char_length(nom) BETWEEN 1 AND 200 AND nom !~ '[[:cntrl:]]'),
    cle        TEXT NOT NULL UNIQUE CHECK (cle <> ''),
    import_id  UUID NOT NULL REFERENCES import_catalogue (id)
);

-- Un médicament : sa dénomination (nom commercial, ou la DCI pour un
-- générique), sa forme et son dosage, tels qu'écrits dans la source. Le
-- prix n'est pas ici (D7 : prix de vente par pharmacie, prix d'achat par
-- lot).
CREATE TABLE medicament (
    id            UUID PRIMARY KEY DEFAULT uuidv7(),
    denomination  TEXT NOT NULL CHECK (denomination = btrim(denomination) AND char_length(denomination) BETWEEN 1 AND 200
                                       AND denomination !~ '[[:cntrl:]]'),
    forme         TEXT NOT NULL CHECK (forme = btrim(forme) AND char_length(forme) BETWEEN 1 AND 100 AND forme !~ '[[:cntrl:]]'),
    dosage        TEXT NOT NULL CHECK (dosage = btrim(dosage) AND char_length(dosage) BETWEEN 1 AND 100 AND dosage !~ '[[:cntrl:]]'),
    -- Dénomination, forme et dosage ramenés à une forme de comparaison par
    -- le code : un même médicament n'entre qu'une fois.
    cle           TEXT NOT NULL UNIQUE CHECK (cle <> ''),
    import_id     UUID NOT NULL REFERENCES import_catalogue (id),
    date_creation TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE medicament_dci (
    medicament_id UUID NOT NULL REFERENCES medicament (id),
    dci_id        UUID NOT NULL REFERENCES dci (id),
    PRIMARY KEY (medicament_id, dci_id)
);

-- Recherche des équivalents : tous les médicaments d'une DCI.
CREATE INDEX medicament_dci_dci_idx ON medicament_dci (dci_id);

-- Rien ne se modifie ni ne s'efface : un catalogue est une référence ; une
-- erreur se corrigera par un import suivant (étape à concevoir si besoin,
-- docs/points-ouverts.md).
CREATE FUNCTION catalogue_non_modifiable() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '%: les lignes du catalogue ne sont jamais modifiees ni supprimees (%)', TG_TABLE_NAME, TG_OP;
END;
$$;

CREATE TRIGGER import_catalogue_sans_modification BEFORE UPDATE OR DELETE ON import_catalogue
    FOR EACH ROW EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER import_catalogue_sans_troncature BEFORE TRUNCATE ON import_catalogue
    FOR EACH STATEMENT EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER dci_sans_modification BEFORE UPDATE OR DELETE ON dci
    FOR EACH ROW EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER dci_sans_troncature BEFORE TRUNCATE ON dci
    FOR EACH STATEMENT EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER medicament_sans_modification BEFORE UPDATE OR DELETE ON medicament
    FOR EACH ROW EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER medicament_sans_troncature BEFORE TRUNCATE ON medicament
    FOR EACH STATEMENT EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER medicament_dci_sans_modification BEFORE UPDATE OR DELETE ON medicament_dci
    FOR EACH ROW EXECUTE FUNCTION catalogue_non_modifiable();
CREATE TRIGGER medicament_dci_sans_troncature BEFORE TRUNCATE ON medicament_dci
    FOR EACH STATEMENT EXECUTE FUNCTION catalogue_non_modifiable();

-- Droits (suite de P1, migration 0026) : le serveur lit le catalogue, ne
-- l'écrit jamais ; seul le rôle des outils l'écrit (outil local, D6).
-- `laaficare_outils` hérite des droits du serveur : l'insertion lui est
-- donc rendue explicitement.
REVOKE INSERT, UPDATE, DELETE ON import_catalogue, dci, medicament, medicament_dci FROM laaficare_serveur;
GRANT INSERT ON import_catalogue, dci, medicament, medicament_dci TO laaficare_outils;
