import { DemandeDetail, DemandeResume } from 'commun';

// Données fictives (CLAUDE.md §13) : jamais de vrai nom ni de vrai numéro.
export function demandeResume(modifs: Partial<DemandeResume> = {}): DemandeResume {
  return {
    id: '0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55',
    statut: 'en_attente',
    type_acte: 'pharmacie',
    date_acte: '2026-10-08',
    montant_acte_fcfa: 12500,
    montant_demande_fcfa: 10000,
    montant_rembourse_fcfa: null,
    date_demande: '2026-10-08T09:30:00Z',
    partenaire: 'Pharmacie exemple',
    patient_nom: 'OUEDRAOGO',
    patient_prenom: 'Awa',
    numero_carte: 'MUN-0001',
    ...modifs,
  };
}

export function demandeDetail(modifs: Partial<DemandeDetail> = {}): DemandeDetail {
  return {
    ...demandeResume(),
    motif_rejet: null,
    periode_debut: '2026-02-01',
    periode_fin: '2027-01-31',
    solde_restant_periode_fcfa: 87500,
    historique: [
      {
        statut_precedent: null,
        statut_nouveau: 'en_attente',
        date_changement: '2026-10-08T09:30:00Z',
        agent_nom: null,
        agent_prenom: null,
      },
    ],
    ...modifs,
  };
}

export function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}
