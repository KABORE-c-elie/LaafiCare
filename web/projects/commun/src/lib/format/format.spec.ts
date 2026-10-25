import { formaterDateBurkina, jourAuBurkina, lendemain } from './format';

describe('dates au Burkina Faso (UTC+0, sans heure d’été)', () => {
  it('jour du Burkina d’un instant, sous la forme AAAA-MM-JJ', () => {
    expect(jourAuBurkina(new Date('2026-10-22T23:59:59Z'))).toBe('2026-10-22');
    expect(jourAuBurkina(new Date('2026-10-23T00:00:00Z'))).toBe('2026-10-23');
  });

  it('lendemain, y compris en fin de mois, d’année et un 29 février', () => {
    expect(lendemain('2026-10-22')).toBe('2026-10-23');
    expect(lendemain('2026-10-31')).toBe('2026-11-01');
    expect(lendemain('2026-12-31')).toBe('2027-01-01');
    expect(lendemain('2028-02-28')).toBe('2028-02-29');
  });

  it('jour d’un instant au format du SMS d’invitation (JJ/MM/AAAA)', () => {
    expect(formaterDateBurkina('2026-10-29T09:00:00Z')).toBe('29/10/2026');
  });
});
