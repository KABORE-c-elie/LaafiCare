-- Module médecin, étape 1 (décisions du porteur du 2026-11-17, CLAUDE.md
-- §17) : rôle auxiliaire de pharmacie, profil professionnel (numéro d'ordre
-- et spécialité) et sexe du patient.

-- ---------------------------------------------------------------------
-- Rôle `auxiliaire_pharmacie` (remplace la décision D5 : pas de
-- préparateur). Il fait les entrées, les ventes, les pertes et les
-- inventaires ; il ne voit jamais les prix d'achat, les dépenses, le
-- bénéfice net ni aucun dossier (contrôlé par le code).
-- ---------------------------------------------------------------------
ALTER TABLE affectation DROP CONSTRAINT affectation_role_check;
ALTER TABLE affectation ADD CONSTRAINT affectation_role_check CHECK (role IN (
    'responsable', 'directeur', 'chef_de_service', 'major', 'medecin',
    'infirmier', 'agent_accueil', 'pharmacien', 'biologiste',
    'agent_assurance_munaseb', 'auxiliaire_pharmacie'));

ALTER TABLE invitation DROP CONSTRAINT invitation_role_check;
ALTER TABLE invitation ADD CONSTRAINT invitation_role_check CHECK (role IN (
    'responsable', 'directeur', 'chef_de_service', 'major', 'medecin',
    'infirmier', 'agent_accueil', 'pharmacien', 'biologiste',
    'agent_assurance_munaseb', 'auxiliaire_pharmacie'));

-- Auteurs d'une vente ou d'un mouvement de stock. Les prix, les seuils, les
-- réglages et les dépenses gardent leurs auteurs (migrations 0031, 0032).
ALTER TABLE vente DROP CONSTRAINT vente_auteur_role_check;
ALTER TABLE vente ADD CONSTRAINT vente_auteur_role_check
    CHECK (auteur_role IN ('pharmacien', 'responsable', 'auxiliaire_pharmacie'));
ALTER TABLE mouvement_stock DROP CONSTRAINT mouvement_stock_auteur_role_check;
ALTER TABLE mouvement_stock ADD CONSTRAINT mouvement_stock_auteur_role_check
    CHECK (auteur_role IN ('pharmacien', 'responsable', 'auxiliaire_pharmacie'));

-- ---------------------------------------------------------------------
-- Profil professionnel : numéro d'ordre et spécialité appartiennent à la
-- personne, pas à l'affectation (CLAUDE.md §6, Q10). Une ligne par compte
-- professionnel. Numéro en texte libre (décision MD9) : aucune source n'en
-- donne le format ; l'équipe LaafiCare le contrôle et le marque vérifié.
-- ---------------------------------------------------------------------
CREATE TABLE profil_professionnel (
    compte_id              UUID PRIMARY KEY,
    compte_type            TEXT NOT NULL DEFAULT 'professionnel' CHECK (compte_type = 'professionnel'),
    numero_ordre           TEXT CHECK (numero_ordre IS NULL OR (
                               numero_ordre = btrim(numero_ordre) AND numero_ordre <> ''
                               AND char_length(numero_ordre) <= 50 AND numero_ordre !~ '[[:cntrl:]]')),
    -- Liste courte proposée au porteur (décision MD10), à valider.
    specialite             TEXT CHECK (specialite IN (
                               'medecine_generale', 'gynecologie_obstetrique', 'pediatrie',
                               'chirurgie', 'sage_femme')),
    modifie_le             TIMESTAMPTZ NOT NULL DEFAULT now(),
    verifie_le             TIMESTAMPTZ,
    verifie_par_compte_id  UUID,
    verifie_par_type       TEXT NOT NULL DEFAULT 'administrateur_laaficare'
                           CHECK (verifie_par_type = 'administrateur_laaficare'),

    CONSTRAINT profil_professionnel_compte
        FOREIGN KEY (compte_id, compte_type) REFERENCES compte (id, type_compte),
    -- Seul un compte administrateur vérifie (clé composite, comme
    -- profil_administrateur, migration 0013).
    CONSTRAINT profil_professionnel_verifie_par
        FOREIGN KEY (verifie_par_compte_id, verifie_par_type) REFERENCES compte (id, type_compte),
    CONSTRAINT profil_professionnel_verification_complete
        CHECK ((verifie_le IS NULL) = (verifie_par_compte_id IS NULL)),
    -- On ne vérifie pas un profil vide.
    CONSTRAINT profil_professionnel_verifie_si_numero
        CHECK (verifie_le IS NULL OR numero_ordre IS NOT NULL)
);

-- Tout changement du numéro ou de la spécialité annule la vérification,
-- même si la même mise à jour la pose (même règle que l'email, migration
-- 0016) ; un profil ne naît jamais vérifié.
CREATE FUNCTION profil_professionnel_verification() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.verifie_le := NULL;
        NEW.verifie_par_compte_id := NULL;
    ELSIF NEW.numero_ordre IS DISTINCT FROM OLD.numero_ordre
          OR NEW.specialite IS DISTINCT FROM OLD.specialite THEN
        NEW.verifie_le := NULL;
        NEW.verifie_par_compte_id := NULL;
        NEW.modifie_le := now();
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER profil_professionnel_verification
    BEFORE INSERT OR UPDATE ON profil_professionnel
    FOR EACH ROW EXECUTE FUNCTION profil_professionnel_verification();

-- Pas d'effacement : un profil se modifie, il ne disparaît pas.
REVOKE DELETE ON profil_professionnel FROM laaficare_serveur;

-- ---------------------------------------------------------------------
-- Sexe du patient, facultatif (décision du porteur du 2026-11-17). Deux
-- valeurs ; aucun texte officiel lu ne fixe la liste (question ouverte dans
-- docs/points-ouverts.md). Vide tant que la personne ne l'a pas donné.
-- ---------------------------------------------------------------------
ALTER TABLE patient ADD COLUMN sexe TEXT CHECK (sexe IN ('feminin', 'masculin'));
