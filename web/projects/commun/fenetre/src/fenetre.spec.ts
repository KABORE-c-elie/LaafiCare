import { DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { HttpErrorResponse } from '@angular/common/http';
import { TestBed } from '@angular/core/testing';
import { DonneesMotif, FenetreConfirmationComponent, FenetreMotifComponent } from './fenetre';

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

function bouton(racine: HTMLElement, texte: string): HTMLButtonElement | undefined {
  return [...racine.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes(texte));
}

describe('FenetreMotifComponent (commun)', () => {
  function ouvrir(envoyer: (motif: string) => Promise<void>) {
    const ref = { close: vi.fn(), disableClose: false };
    const donnees: DonneesMotif = {
      titre: 'Refuser',
      idTitre: 'titre-essai',
      libelle: 'Motif',
      action: 'Envoyer le refus',
      rappel: 'Rappel d’essai.',
      max: 10,
      maxAffiche: '10',
      messageVide: 'Vide.',
      messageTropLong: 'Trop long.',
      envoyer,
    };
    TestBed.configureTestingModule({
      imports: [FenetreMotifComponent],
      providers: [
        { provide: DIALOG_DATA, useValue: donnees },
        { provide: DialogRef, useValue: ref },
      ],
    });
    const fixture = TestBed.createComponent(FenetreMotifComponent);
    return { fixture, ref, racine: fixture.nativeElement as HTMLElement };
  }

  async function saisir(fixture: { whenStable: () => Promise<unknown> }, racine: HTMLElement, valeur: string) {
    const zone = racine.querySelector<HTMLTextAreaElement>('#motif');
    if (zone === null) {
      throw new Error('zone absente');
    }
    zone.value = valeur;
    zone.dispatchEvent(new Event('input'));
    await fixture.whenStable();
  }

  it('textes et longueur viennent des données ; vide et trop long refusés sans envoi', async () => {
    const envoyer = vi.fn(() => Promise.resolve());
    const { fixture, ref, racine } = ouvrir(envoyer);
    await fixture.whenStable();
    expect(racine.textContent).toContain('Rappel d’essai.');
    bouton(racine, 'Envoyer le refus')?.click();
    await fixture.whenStable();
    expect(racine.textContent).toContain('Vide.');
    await saisir(fixture, racine, 'a'.repeat(11));
    expect(racine.textContent).toContain('11 / 10 caractères');
    bouton(racine, 'Envoyer le refus')?.click();
    await fixture.whenStable();
    expect(racine.textContent).toContain('Trop long.');
    expect(envoyer).not.toHaveBeenCalled();
    expect(ref.close).not.toHaveBeenCalled();
  });

  it('motif valide envoyé ; refus du serveur motif_trop_long affiché, fenêtre ouverte', async () => {
    const envoyer = vi.fn(() =>
      Promise.reject(new HttpErrorResponse({ status: 422, error: { erreur: 'x', code: 'motif_trop_long' } })),
    );
    const { fixture, ref, racine } = ouvrir(envoyer);
    await fixture.whenStable();
    await saisir(fixture, racine, 'Pièces');
    bouton(racine, 'Envoyer le refus')?.click();
    await attendre();
    await fixture.whenStable();
    expect(envoyer).toHaveBeenCalledWith('Pièces');
    expect(racine.textContent).toContain('Trop long.');
    expect(ref.close).not.toHaveBeenCalled();
  });
});

describe('FenetreConfirmationComponent (commun)', () => {
  it('double clic : un seul envoi ; fermée avec le résultat', async () => {
    let terminer: (valeur: string) => void = () => undefined;
    const envoyer = vi.fn(() => new Promise<string>((resolve) => (terminer = resolve)));
    const ref = { close: vi.fn(), disableClose: false };
    TestBed.configureTestingModule({
      imports: [FenetreConfirmationComponent],
      providers: [
        { provide: DIALOG_DATA, useValue: { titre: 'Confirmer', texte: 'Sûr ?', action: 'Oui', envoyer } },
        { provide: DialogRef, useValue: ref },
      ],
    });
    const fixture = TestBed.createComponent(FenetreConfirmationComponent);
    await fixture.whenStable();
    const racine = fixture.nativeElement as HTMLElement;
    bouton(racine, 'Oui')?.click();
    bouton(racine, 'Oui')?.click();
    expect(envoyer).toHaveBeenCalledTimes(1);
    expect(ref.disableClose).toBe(true);
    terminer('fait');
    await attendre();
    expect(ref.close).toHaveBeenCalledWith({ ok: true, resultat: 'fait' });
  });

  it('erreur : fermée avec l’erreur, laissée à la page', async () => {
    const erreur = new Error('panne');
    const ref = { close: vi.fn(), disableClose: false };
    TestBed.configureTestingModule({
      imports: [FenetreConfirmationComponent],
      providers: [
        { provide: DIALOG_DATA, useValue: { titre: 'Confirmer', texte: 'Sûr ?', action: 'Oui', envoyer: () => Promise.reject(erreur) } },
        { provide: DialogRef, useValue: ref },
      ],
    });
    const fixture = TestBed.createComponent(FenetreConfirmationComponent);
    await fixture.whenStable();
    bouton(fixture.nativeElement as HTMLElement, 'Oui')?.click();
    await attendre();
    expect(ref.close).toHaveBeenCalledWith({ ok: false, erreur });
  });
});
