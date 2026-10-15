-- Réenrôlement d'un administrateur (étape 3.6.4, décision du 2026-10-15).
-- Un administrateur sans TOTP actif (premier administrateur, ou TOTP
-- désactivé après la perte du téléphone) ne peut enregistrer un nouveau
-- TOTP que pendant une fenêtre ouverte par un outil local de l'équipe
-- LaafiCare (création du premier administrateur, désactivation d'un TOTP,
-- étape 3.6.5), jamais par une route. Sans fenêtre en cours, il ne reçoit
-- aucun jeton, même avec le bon mot de passe : sinon un mot de passe volé
-- suffirait à enregistrer son propre TOTP et à prendre le compte.
--
-- Durée : 10 minutes, constante du code (jamais un paramètre de l'outil).
-- Source : NIST SP 800-63B rév. 4, §4.1.3, un code d'association d'un
-- nouvel authentificateur « SHALL be valid for a maximum of 10 minutes ».
-- L'outil est lancé avec l'administrateur présent ou au téléphone ; si la
-- fenêtre expire, il est relancé.
--
-- Une seule horloge, celle de PostgreSQL : l'outil écrit
-- `now() + interval '10 minutes'`, la connexion, l'extracteur et la
-- confirmation comparent à `now()`. La confirmation réussie vide la
-- colonne, dans la transaction de l'activation.
--
-- La base ne peut pas interdire l'écriture par une route (un seul rôle
-- PostgreSQL pour l'application) : garanti par le code. Voir
-- docs/points-ouverts.md (rôle séparé pour les outils locaux).
ALTER TABLE compte
    ADD COLUMN reenrolement_autorise_jusqu_au TIMESTAMPTZ;

-- Seul un compte administrateur peut recevoir une autorisation : les
-- patients et les professionnels activent leur TOTP autrement (J10).
ALTER TABLE compte
    ADD CONSTRAINT compte_reenrolement_administrateur_seulement
        CHECK (reenrolement_autorise_jusqu_au IS NULL OR type_compte = 'administrateur_laaficare');
