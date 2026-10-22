import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import qrcode from 'qrcode-generator';

/** Marge autour du QR code, en modules (ISO/IEC 18004, « quiet zone » de 4 modules). */
const MARGE = 4;

/**
 * Matrice du QR code (qrcode-generator 2.0.4, MIT, décision B4 du porteur),
 * changée en un seul chemin SVG. Seules `addData`, `make`, `getModuleCount`
 * et `isDark` servent : ni `createSvgTag`, ni `createImgTag`, ni
 * `createDataURL`, qui produisent du texte HTML ou une adresse `data:`.
 * Niveau de correction M ; version (taille) choisie par la bibliothèque (0).
 */
export function cheminQr(texte: string): { taille: number; chemin: string } {
  const qr = qrcode(0, 'M');
  qr.addData(texte);
  qr.make();
  const modules = qr.getModuleCount();
  const morceaux: string[] = [];
  for (let ligne = 0; ligne < modules; ligne++) {
    for (let colonne = 0; colonne < modules; colonne++) {
      if (qr.isDark(ligne, colonne)) {
        morceaux.push(`M${colonne + MARGE} ${ligne + MARGE}h1v1h-1z`);
      }
    }
  }
  return { taille: modules + 2 * MARGE, chemin: morceaux.join('') };
}

/**
 * QR code dessiné dans le navigateur, dans le gabarit (aucun innerHTML, aucun
 * style en ligne : CSP `style-src 'self'`). Le texte encodé (l'adresse
 * `otpauth://`, qui contient le secret du second facteur) ne quitte jamais le
 * navigateur : aucun service en ligne, aucun stockage, aucune écriture dans la
 * console.
 */
@Component({
  selector: 'lc-qr-code',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <svg
      class="qr"
      role="img"
      focusable="false"
      [attr.aria-label]="libelle()"
      [attr.viewBox]="'0 0 ' + dessin().taille + ' ' + dessin().taille"
      shape-rendering="crispEdges"
    >
      <path class="qr__modules" [attr.d]="dessin().chemin" />
    </svg>
  `,
})
export class QrCodeComponent {
  readonly texte = input.required<string>();
  readonly libelle = input.required<string>();
  protected readonly dessin = computed(() => cheminQr(this.texte()));
}
