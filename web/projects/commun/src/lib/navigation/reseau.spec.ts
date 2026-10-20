import { TestBed } from '@angular/core/testing';
import { ReseauService } from './reseau';

describe('ReseauService', () => {
  it('suit les événements online et offline du navigateur', () => {
    const reseau = TestBed.inject(ReseauService);
    window.dispatchEvent(new Event('offline'));
    expect(reseau.enLigne()).toBe(false);
    window.dispatchEvent(new Event('online'));
    expect(reseau.enLigne()).toBe(true);
  });
});
