import { DIALOG_DATA, DialogRef } from '@angular/cdk/dialog';
import { HttpErrorResponse } from '@angular/common/http';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { FenetreValidationStructureComponent, MESSAGES_VALIDATION } from './fenetre-validation';

const DEMAIN = '2026-10-23';

function attendre(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve));
}

describe('FenetreValidationStructureComponent', () => {
  let fixture: ComponentFixture<FenetreValidationStructureComponent>;
  let envoyer: ReturnType<typeof vi.fn<(licence: string | null) => Promise<void>>>;
  let ref: { close: ReturnType<typeof vi.fn>; disableClose: boolean };

  beforeEach(async () => {
    envoyer = vi.fn<(licence: string | null) => Promise<void>>(() => Promise.resolve());
    ref = { close: vi.fn(), disableClose: false };
    TestBed.configureTestingModule({
      imports: [FenetreValidationStructureComponent],
      providers: [
        { provide: DIALOG_DATA, useValue: { nom: 'Mutuelle "essai"', demain: DEMAIN, envoyer } },
        { provide: DialogRef, useValue: ref },
      ],
    });
    fixture = TestBed.createComponent(FenetreValidationStructureComponent);
    await fixture.whenStable();
  });

  function element(): HTMLElement {
    return fixture.nativeElement as HTMLElement;
  }

  async function cocherNom(): Promise<void> {
    element().querySelector<HTMLInputElement>('input[type="checkbox"]')?.click();
    await fixture.whenStable();
  }

  async function choisir(valeur: 'sans_fin' | 'jusqu_au'): Promise<void> {
    element().querySelector<HTMLInputElement>(`input[value="${valeur}"]`)?.click();
    await fixture.whenStable();
  }

  async function saisirDate(valeur: string): Promise<void> {
    const champ = element().querySelector<HTMLInputElement>('#licence');
    if (champ === null) {
      throw new Error('champ de date absent');
    }
    champ.value = valeur;
    champ.dispatchEvent(new Event('input'));
    await fixture.whenStable();
  }

  async function valider(): Promise<void> {
    [...element().querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Valider la demande'))?.click();
    await attendre();
    await fixture.whenStable();
  }

  it('avertissement : le nom exact, qui part dans les SMS d’invitation', () => {
    expect(element().querySelector('.nom-exact')?.textContent).toContain('« Mutuelle "essai" »');
    expect(element().textContent).toContain('il est repris tel quel dans les SMS d\'invitation');
  });

  it('aucun choix de licence par défaut ; rien ne part sans le nom vérifié ni la durée', async () => {
    expect(element().querySelector<HTMLInputElement>('input[value="sans_fin"]')?.checked).toBe(false);
    expect(element().querySelector<HTMLInputElement>('input[value="jusqu_au"]')?.checked).toBe(false);
    await valider();
    expect(element().textContent).toContain(MESSAGES_VALIDATION.nomNonVerifie);
    expect(element().textContent).toContain(MESSAGES_VALIDATION.choixLicence);
    await choisir('sans_fin');
    await valider();
    expect(envoyer).not.toHaveBeenCalled();
  });

  it('licence sans fin, nom vérifié : envoyée avec null', async () => {
    await cocherNom();
    await choisir('sans_fin');
    await valider();
    expect(envoyer).toHaveBeenCalledWith(null);
    expect(ref.close).toHaveBeenCalledWith({ ok: true, resultat: undefined });
  });

  it('date : au plus tôt demain (attribut min) ; aujourd’hui refusé ; demain accepté', async () => {
    await cocherNom();
    await choisir('jusqu_au');
    expect(element().querySelector('#licence')?.getAttribute('min')).toBe(DEMAIN);
    await valider();
    expect(element().textContent).toContain(MESSAGES_VALIDATION.dateManquante);
    await saisirDate('2026-10-22');
    await valider();
    expect(element().textContent).toContain(MESSAGES_VALIDATION.licenceInvalide);
    expect(envoyer).not.toHaveBeenCalled();
    await saisirDate(DEMAIN);
    await valider();
    expect(envoyer).toHaveBeenCalledWith(DEMAIN);
  });

  it('licence_invalide du serveur : affiché dans la fenêtre, qui reste ouverte', async () => {
    envoyer.mockRejectedValueOnce(
      new HttpErrorResponse({ status: 422, error: { erreur: 'x', code: 'licence_invalide' } }),
    );
    await cocherNom();
    await choisir('jusqu_au');
    await saisirDate(DEMAIN);
    await valider();
    expect(element().textContent).toContain(MESSAGES_VALIDATION.licenceInvalide);
    expect(ref.close).not.toHaveBeenCalled();
  });

  it('double clic : un seul envoi', async () => {
    let terminer: () => void = () => undefined;
    envoyer.mockImplementation(() => new Promise<void>((resolve) => (terminer = resolve)));
    await cocherNom();
    await choisir('sans_fin');
    const bouton = [...element().querySelectorAll<HTMLButtonElement>('button')].find((b) =>
      b.textContent?.includes('Valider la demande'),
    );
    bouton?.click();
    bouton?.click();
    expect(envoyer).toHaveBeenCalledTimes(1);
    terminer();
    await attendre();
  });
});
