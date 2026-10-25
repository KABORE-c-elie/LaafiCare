import { DetailStructure, ResumeStructure } from 'commun';

// Données fictives (CLAUDE.md §13) : jamais de vrai nom ni de vrai numéro.
export function resumeStructure(modifs: Partial<ResumeStructure> = {}): ResumeStructure {
  return {
    id: '0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a10',
    type: 'munaseb',
    nom: 'MUNASEB',
    statut: 'en_attente',
    date_creation: '2026-10-21T08:00:00Z',
    commune: 'Commune fictive',
    responsable_nom: 'OUEDRAOGO',
    responsable_prenom: 'Awa',
    ...modifs,
  };
}

export function detailStructure(modifs: Partial<DetailStructure> = {}): DetailStructure {
  return {
    ...resumeStructure(),
    telephone: '+22670000010',
    telephone_urgence: '+22670000011',
    email: 'contact.munaseb@example.org',
    region: 'Région fictive',
    province: 'Province fictive',
    parcelle: '12',
    lot: null,
    section: 'B',
    latitude: 12.3714,
    longitude: -1.5197,
    licence_expire_le: null,
    responsable_telephone: '+22670000001',
    responsable_email: 'responsable@example.org',
    responsable_email_verifie: true,
    decisions: [],
    autres_demandes_munaseb_en_attente: [],
    munaseb_deja_validee: null,
    ...modifs,
  };
}

export function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}
