import { Pipe, PipeTransform } from '@angular/core';

// Formats français par Intl (MDN, Intl.NumberFormat et Intl.DateTimeFormat),
// avec la locale `fr-FR` : les milliers y sont séparés par une espace fine
// insécable, et les noms de mois sont en français.
const LOCALE = 'fr-FR';

/** Fuseau du Burkina Faso (base IANA), le même que le backend (CLAUDE.md §14, D9). */
export const FUSEAU_BURKINA = 'Africa/Ouagadougou';

const NOMBRE = new Intl.NumberFormat(LOCALE, { maximumFractionDigits: 0 });
// Une date seule (`AAAA-MM-JJ`) est un jour du calendrier, pas un instant :
// elle est lue et affichée en UTC pour ne jamais glisser d'un jour.
const JOUR = new Intl.DateTimeFormat(LOCALE, { dateStyle: 'long', timeZone: 'UTC' });
const DATE_HEURE = new Intl.DateTimeFormat(LOCALE, {
  dateStyle: 'short',
  timeStyle: 'short',
  timeZone: FUSEAU_BURKINA,
});

/** Montant entier en FCFA (docs/api.md §1.1), ex. `12 500 FCFA`. */
export function formaterFcfa(montant: number): string {
  // Espace insécable avant l'unité : « FCFA » ne part jamais seul à la ligne.
  return `${NOMBRE.format(montant)}\u{00A0}FCFA`;
}

/** Date `AAAA-MM-JJ` (docs/api.md §1.1), ex. `18 octobre 2027`. Texte d'origine si illisible. */
export function formaterJour(jour: string): string {
  const morceaux = /^(\d{4})-(\d{2})-(\d{2})$/.exec(jour);
  if (morceaux === null) {
    return jour;
  }
  const [, annee, mois, quantieme] = morceaux;
  return JOUR.format(new Date(Date.UTC(Number(annee), Number(mois) - 1, Number(quantieme))));
}

const DATE_BURKINA = new Intl.DateTimeFormat(LOCALE, { dateStyle: 'short', timeZone: FUSEAU_BURKINA });

/**
 * Jour d'un instant RFC 3339 au Burkina, ex. `29/10/2026` : la forme du SMS
 * d'invitation (docs/api.md §2.10, « avant le JJ/MM/AAAA »).
 */
export function formaterDateBurkina(instant: string): string {
  const date = new Date(instant);
  return Number.isNaN(date.getTime()) ? instant : DATE_BURKINA.format(date);
}

// `en-CA` écrit une date seule sous la forme AAAA-MM-JJ (forme de l'API, §1.1).
const JOUR_ISO_BURKINA = new Intl.DateTimeFormat('en-CA', {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  timeZone: FUSEAU_BURKINA,
});

/** Date du jour au Burkina Faso, `AAAA-MM-JJ` (la date que le backend prend pour « aujourd'hui », D9). */
export function jourAuBurkina(maintenant: Date = new Date()): string {
  return JOUR_ISO_BURKINA.format(maintenant);
}

/** Lendemain d'une date `AAAA-MM-JJ`, calculé en UTC (aucun changement d'heure ne s'y glisse). */
export function lendemain(jour: string): string {
  const [annee, mois, quantieme] = jour.split('-').map(Number);
  const suivant = new Date(Date.UTC(annee, mois - 1, quantieme + 1));
  return suivant.toISOString().slice(0, 10);
}

/** Date et heure RFC 3339 en UTC (docs/api.md §1.1), affichée à l'heure du Burkina. */
export function formaterDateHeure(instant: string): string {
  const date = new Date(instant);
  return Number.isNaN(date.getTime()) ? instant : DATE_HEURE.format(date);
}

@Pipe({ name: 'fcfa' })
export class FcfaPipe implements PipeTransform {
  transform(montant: number): string {
    return formaterFcfa(montant);
  }
}

@Pipe({ name: 'jour' })
export class JourPipe implements PipeTransform {
  transform(jour: string): string {
    return formaterJour(jour);
  }
}

@Pipe({ name: 'dateHeure' })
export class DateHeurePipe implements PipeTransform {
  transform(instant: string): string {
    return formaterDateHeure(instant);
  }
}
