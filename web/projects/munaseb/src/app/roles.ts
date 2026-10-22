/**
 * Rôles d'une structure MUNASEB ouverts dans cette application (docs/api.md
 * §2.10, « Pour la MUNASEB, deux rôles »), et la page où chacun arrive. Chaque
 * rôle n'ouvre que ses pages : l'agent les demandes (§2.3), le responsable les
 * invitations (§2.10).
 */
export const PAGE_DU_ROLE = {
  agent_assurance_munaseb: '/demandes',
  responsable: '/invitations',
} as const;

export type RoleMunaseb = keyof typeof PAGE_DU_ROLE;

export function estRoleMunaseb(role: string): role is RoleMunaseb {
  return Object.hasOwn(PAGE_DU_ROLE, role);
}

export const LIBELLES_ROLE: Readonly<Record<RoleMunaseb, string>> = {
  agent_assurance_munaseb: 'Agent MUNASEB',
  responsable: 'Responsable',
};
