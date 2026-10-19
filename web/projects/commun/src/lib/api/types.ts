// Types des corps échangés avec l'API, recopiés de docs/api.md (branche dev,
// état au 2026-10-18). Les noms de champs restent en snake_case, comme dans le
// contrat (§1.1) : aucune conversion, pour qu'un écart avec le contrat se voie.

/** Corps de toute réponse d'erreur (§1.3). Les applications testent `code`, jamais `erreur`. */
export interface CorpsErreur {
  erreur: string;
  code: string;
  /** Avec `mot_de_passe_non_conforme` (§1.5). */
  regles_non_respectees?: string[];
  /** Avec `en_carence`, date `AAAA-MM-JJ`. */
  debut_couverture?: string;
  /** Avec `transition_invalide`. */
  statut_actuel?: StatutDemande;
}

/** Paire de jetons d'une session (§1.7). `jeton_renouvellement` : 64 caractères hexadécimaux. */
export interface PaireJetons {
  jeton_acces: string;
  /** Durée de vie du jeton d'accès, en secondes. */
  expire_dans: number;
  jeton_renouvellement: string;
}

/** Réponse 200 de `POST /api/professionnels/connexion` (§2.4). L'application teste toujours `etape`. */
export type ReponseConnexionProfessionnelle =
  | ({ etape: 'complete' } & PaireJetons)
  | { etape: 'second_facteur'; jeton_intermediaire: string };

/** Corps de `POST /api/professionnels/connexion/second-facteur` : un seul des deux champs (§2.4). */
export type CorpsSecondFacteur = { code: string } | { code_secours: string };

/** Réponse 200 du second facteur (§2.4). */
export type ReponseSecondFacteur = { etape: 'complete' } & PaireJetons;

/** Une ligne de `GET /api/professionnels/affectations` (§2.6). */
export interface Affectation {
  id: string;
  role: string;
  structure_id: string;
  structure_nom: string;
  structure_type: string;
  structure_statut: string;
  /** Date `AAAA-MM-JJ` comprise, ou `null` pour une licence sans fin. */
  licence_expire_le: string | null;
  exercable: boolean;
}

export interface ListeAffectations {
  affectations: Affectation[];
}

/** Valeurs de `type_acte` (§2.3.2). */
export type TypeActe = 'consultation' | 'hospitalisation' | 'pharmacie' | 'laboratoire' | 'lunetterie';

/** Valeurs de `statut` d'une demande (§2.3.2). */
export type StatutDemande = 'en_attente' | 'en_cours' | 'valide' | 'rejete' | 'paye';

/** Une demande dans la liste de `GET /api/assurance-munaseb/remboursements` (§2.3.4). */
export interface DemandeResume {
  id: string;
  statut: StatutDemande;
  type_acte: TypeActe;
  date_acte: string;
  montant_acte_fcfa: number;
  montant_demande_fcfa: number;
  /** `null` tant que la demande n'est pas validée. */
  montant_rembourse_fcfa: number | null;
  /** Date et heure RFC 3339 en UTC. */
  date_demande: string;
  partenaire: string;
  patient_nom: string | null;
  patient_prenom: string | null;
  numero_carte: string;
}

export interface PageDemandes {
  demandes: DemandeResume[];
  /** À passer dans `apres` pour la page suivante ; `null` quand la page n'est pas pleine. */
  suivant: string | null;
}

/** Une ligne de l'historique d'une demande (§2.3.5). */
export interface LigneHistorique {
  statut_precedent: StatutDemande | null;
  statut_nouveau: StatutDemande;
  date_changement: string;
  /** `null` pour la création (faite par le partenaire). */
  agent_nom: string | null;
  agent_prenom: string | null;
}

/** Détail de `GET /api/assurance-munaseb/remboursements/{id}` (§2.3.5). */
export interface DemandeDetail extends DemandeResume {
  motif_rejet: string | null;
  periode_debut: string;
  periode_fin: string;
  /** Indicatif : recalculé au moment de la validation. */
  solde_restant_periode_fcfa: number;
  historique: LigneHistorique[];
}

/** Réponse 200 de `POST …/remboursements/{id}/valider` (§2.3.6). Peut valoir 0. */
export interface ReponseValidation {
  montant_rembourse_fcfa: number;
}
