import { DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { HttpErrorResponse } from '@angular/common/http';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { FenetreMotifComponent } from 'commun/fenetre';
import { FenetreValidationComponent, MESSAGE_MOTIF_TROP_LONG, MESSAGE_MOTIF_VIDE, donneesRejet } from './fenetres';

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

// Fenêtre de motif commune (commun/fenetre), avec les textes du rejet d'une demande.
describe('FenetreMotifComponent (rejet d’une demande)', () => {
  let fixture: ComponentFixture<FenetreMotifComponent>;
  let envoyer: ReturnType<typeof vi.fn<(motif: string) => Promise<void>>>;
  let ref: { close: ReturnType<typeof vi.fn>; disableClose: boolean };

  beforeEach(async () => {
    envoyer = vi.fn<(motif: string) => Promise<void>>(() => Promise.resolve());
    ref = { close: vi.fn(), disableClose: false };
    TestBed.configureTestingModule({
      imports: [FenetreMotifComponent],
      providers: [
        { provide: DIALOG_DATA, useValue: donneesRejet(envoyer) },
        { provide: DialogRef, useValue: ref },
      ],
    });
    fixture = TestBed.createComponent(FenetreMotifComponent);
    await fixture.whenStable();
  });

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function saisir(valeur: string): Promise<void> {
    const zone = element().querySelector<HTMLTextAreaElement>('#motif');
    if (zone === null) {
      throw new Error('zone du motif absente');
    }
    zone.value = valeur;
    zone.dispatchEvent(new Event('input'));
    await fixture.whenStable();
  }

  async function rejeter(): Promise<void> {
    [...element().querySelectorAll<HTMLButtonElement>('button')]
      .find((b) => b.textContent?.includes('Rejeter la demande'))
      ?.click();
    await attendre();
    await fixture.whenStable();
  }

  it('motif vide (ou fait d’espaces) : message, aucun envoi', async () => {
    await saisir('   ');
    await rejeter();
    expect(element().textContent).toContain(MESSAGE_MOTIF_VIDE);
    expect(envoyer).not.toHaveBeenCalled();
    expect(ref.close).not.toHaveBeenCalled();
  });

  it('motif de plus de 1 000 caractères : message, aucun envoi, compteur', async () => {
    await saisir('a'.repeat(1001));
    expect(element().textContent).toContain('1001 / 1 000 caractères');
    await rejeter();
    expect(element().textContent).toContain(MESSAGE_MOTIF_TROP_LONG);
    expect(envoyer).not.toHaveBeenCalled();
  });

  it('motif valide : envoyé tel que saisi, fenêtre fermée avec succès', async () => {
    await saisir('Pièce manquante');
    await rejeter();
    expect(envoyer).toHaveBeenCalledWith('Pièce manquante');
    expect(ref.close).toHaveBeenCalledWith({ ok: true, resultat: undefined });
  });

  it('motif_vide renvoyé par le serveur : affiché dans la fenêtre, qui reste ouverte', async () => {
    envoyer.mockRejectedValueOnce(
      new HttpErrorResponse({ status: 422, error: { erreur: 'x', code: 'motif_vide' } }),
    );
    await saisir('x');
    await rejeter();
    expect(element().textContent).toContain(MESSAGE_MOTIF_VIDE);
    expect(ref.close).not.toHaveBeenCalled();
  });

  it('rappelle que le motif n’apparaît pas dans la notification', () => {
    expect(element().textContent).toContain("Le motif n'apparaît pas dans la notification de l'adhérent.");
  });
});

describe('FenetreValidationComponent', () => {
  it('double clic : un seul envoi', async () => {
    let terminer: (valeur: { montant_rembourse_fcfa: number }) => void = () => undefined;
    const envoyer = vi.fn(() => new Promise<{ montant_rembourse_fcfa: number }>((resolve) => (terminer = resolve)));
    const ref = { close: vi.fn(), disableClose: false };
    TestBed.configureTestingModule({
      imports: [FenetreValidationComponent],
      providers: [
        { provide: DIALOG_DATA, useValue: { montantDemande: 10000, solde: 87500, envoyer } },
        { provide: DialogRef, useValue: ref },
      ],
    });
    const fixture = TestBed.createComponent(FenetreValidationComponent);
    await fixture.whenStable();
    const valider = [...(fixture.nativeElement as HTMLElement).querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === 'Valider',
    );
    valider?.click();
    valider?.click();
    expect(envoyer).toHaveBeenCalledTimes(1);
    expect(ref.disableClose).toBe(true);
    terminer({ montant_rembourse_fcfa: 0 });
    await attendre();
    expect(ref.close).toHaveBeenCalledWith({ ok: true, resultat: { montant_rembourse_fcfa: 0 } });
  });
});
