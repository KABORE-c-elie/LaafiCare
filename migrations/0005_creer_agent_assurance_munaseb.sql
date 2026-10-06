-- AgentAssurance (section 12 du CLAUDE.md), version minimale de cette
-- itération : nommé `agent_assurance_munaseb` et non `agent_assurance` --
-- décision actée avec le porteur (2026-09-23) -- parce que MUNASEB est une
-- assurance spécifique câblée en dur pour ce scope, pas un module Assurance
-- générique (celui-ci viendra avec son propre CDC plus tard, sous un autre
-- nom). Même schéma multi-rôles que `patient` (migration 0003) : clé
-- primaire propre + utilisateur_id en FK, pas d'héritage à clé partagée.
--
-- Pas de colonne `assureur` : tant qu'un seul assureur existe dans le
-- système, sa valeur ne varierait jamais -- MUNASEB reste une constante du
-- code (section 12), pas une donnée en base. À ajouter le jour où un
-- deuxième assureur existe réellement.
--
-- V1 : un seul rôle agent_assurance_munaseb, pas de sous-rôles (Régie des
-- recettes / Liquidation / Médecin Conseillé / Finance / Directeur du
-- mémoire MUNASEB ne sont pas répliqués en comptes séparés, section 12).
CREATE TABLE agent_assurance_munaseb (
    id             UUID PRIMARY KEY DEFAULT uuidv7(),
    utilisateur_id UUID NOT NULL UNIQUE REFERENCES utilisateur (id)
);
