import { TestBed } from '@angular/core/testing';
import { QrCodeComponent, cheminQr } from './qr-code';

// Adresse fictive à la forme de §2.5 (Z8 : aucune donnée personnelle) ; secret inventé pour le test.
const URL = 'otpauth://totp/LaafiCare:Administrateur?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=LaafiCare';

describe('QR code', () => {
  it('matrice carrée avec la marge de 4 modules, motif de repérage en haut à gauche', () => {
    const { taille, chemin } = cheminQr(URL);
    // Une version de QR code a 17 + 4 × v modules de côté (v de 1 à 40), plus 2 × 4 de marge.
    expect((taille - 8 - 17) % 4).toBe(0);
    expect(chemin.startsWith('M4 4h1v1h-1z')).toBe(true);
    // Le module (4 + 1, 4 + 1) est clair : anneau blanc du motif de repérage.
    expect(chemin).not.toContain('M5 5h1v1h-1z');
  });

  it('même texte, même dessin ; un autre texte, un autre dessin', () => {
    expect(cheminQr(URL)).toEqual(cheminQr(URL));
    expect(cheminQr(URL).chemin).not.toBe(cheminQr(URL.replace('JBSW', 'KBSW')).chemin);
  });

  it('dessiné en SVG dans le gabarit, avec un nom accessible ; aucune image ni adresse data:', async () => {
    TestBed.configureTestingModule({ imports: [QrCodeComponent] });
    const fixture = TestBed.createComponent(QrCodeComponent);
    fixture.componentRef.setInput('texte', URL);
    fixture.componentRef.setInput('libelle', 'QR code à scanner');
    await fixture.whenStable();
    const racine = fixture.nativeElement as HTMLElement;
    const svg = racine.querySelector('svg');
    expect(svg?.getAttribute('role')).toBe('img');
    expect(svg?.getAttribute('aria-label')).toBe('QR code à scanner');
    expect(svg?.getAttribute('viewBox')).toBe(`0 0 ${cheminQr(URL).taille} ${cheminQr(URL).taille}`);
    expect(racine.querySelector('path')?.getAttribute('d')).toBe(cheminQr(URL).chemin);
    expect(racine.querySelector('img')).toBeNull();
    expect(racine.innerHTML).not.toContain('data:');
    // Le secret n'apparaît pas en clair dans le document.
    expect(racine.innerHTML).not.toContain('JBSWY3DP');
  });
});
