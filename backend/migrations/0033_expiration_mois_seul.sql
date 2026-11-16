-- Lot dont la boîte n'indique que le mois et l'année d'expiration (ex.
-- « EXP 06/2027 ») ; demande du porteur du 2026-11-16, CLAUDE.md §15.
--
-- Aucune source officielle burkinabè ni de l'UEMOA trouvée sur la
-- convention (docs/points-ouverts.md). Règle la plus prudente pour le
-- patient : le lot est périmé **dès le premier jour du mois imprimé**.
-- `date_expiration` (dernier jour utilisable, compris) est donc le **dernier
-- jour du mois précédent** (06/2027 donne le 31/05/2027), et cette colonne
-- garde la trace de la saisie au mois, pour que l'application affiche aussi
-- le mois imprimé sur la boîte.
--
-- Ajouter une colonne avec une valeur par défaut ne déclenche pas les
-- triggers de ligne : les lots existants (jamais modifiés, migration 0031)
-- valent `false`.

ALTER TABLE lot_stock ADD COLUMN expiration_mois_seul BOOLEAN NOT NULL DEFAULT false;

-- Saisie au mois : la date enregistrée est toujours la veille d'un premier
-- du mois.
ALTER TABLE lot_stock ADD CONSTRAINT lot_expiration_mois_seul_fin_de_mois
    CHECK (NOT expiration_mois_seul OR extract(day FROM date_expiration + 1) = 1);
