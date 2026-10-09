-- Limites de l'OTP patient (décisions O1 à O7, 2026-10-08). NIST SP 800-63B
-- rév. 4 §3.1.3 : un secret de moins de 64 bits impose une limitation des
-- essais (« SHALL implement a rate-limiting mechanism »), et « Generating a
-- new authentication secret SHALL NOT reset the failed authentication
-- count ». OWASP MFA Cheat Sheet : limites par compte, protection du
-- budget SMS. Les valeurs (60 s, 5 par heure, 5 échecs) sont celles du
-- porteur : aucune des deux sources n'en donne.

-- Essais de code (O3, lecture A). `otp_echecs` : échecs cumulés depuis le
-- dernier succès, jamais remis à zéro par un nouveau code. Après 5 échecs
-- cumulés, chaque nouveau code n'a qu'un essai, jusqu'au premier succès.
-- `otp_essais_restants` : essais permis sur le code en cours, fixés à sa
-- génération à partir de `otp_echecs`, et décomptés AVANT la comparaison,
-- en une seule requête (même principe que le verrouillage BF-01) : même des
-- essais simultanés ne dépassent pas la limite.
ALTER TABLE utilisateur
    ADD COLUMN otp_echecs          SMALLINT NOT NULL DEFAULT 0 CHECK (otp_echecs >= 0),
    ADD COLUMN otp_essais_restants SMALLINT NOT NULL DEFAULT 0 CHECK (otp_essais_restants >= 0);

-- Envois (O1, O2, O7). L'identité plutôt que le numéro : la table ne
-- contient aucun numéro de téléphone. Une ligne par SMS envoyé ; seules les
-- lignes de moins d'une heure servent au comptage. Les autres sont effacées
-- à chaque envoi, pour tous les numéros, dans la même transaction (O7 b).
-- ON DELETE CASCADE : ces lignes ne doivent jamais empêcher la suppression
-- d'une identité (future purge des inscriptions abandonnées).
CREATE TABLE envoi_otp (
    id             UUID PRIMARY KEY DEFAULT uuidv7(),
    utilisateur_id UUID NOT NULL REFERENCES utilisateur (id) ON DELETE CASCADE,
    envoye_le      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Comptage par numéro sur l'heure glissante, et dernier envoi (O1, O2).
CREATE INDEX envoi_otp_utilisateur_date_idx ON envoi_otp (utilisateur_id, envoye_le);
-- Nettoyage des lignes de plus d'une heure, tous numéros confondus.
CREATE INDEX envoi_otp_date_idx ON envoi_otp (envoye_le);
