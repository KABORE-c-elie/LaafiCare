import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import {
  AccesService,
  Affectation,
  MESSAGES,
  MessageAffiche,
  MessageEtatComponent,
  MessageNavigation,
  decoderErreur,
  formaterJour,
  messagePourErreur,
} from 'commun';
import { CadreComponent } from '../cadre';

const LIBELLES_ROLE: Readonly<Record<string, string>> = {
  agent_assurance_munaseb: 'Agent MUNASEB',
  responsable: 'Responsable',
};

// Statuts d'une structure (CLAUDE.md §14, « Structures et licence »).
const LIBELLES_STATUT_STRUCTURE: Readonly<Record<string, string>> = {
  brouillon: 'demande en brouillon',
  en_attente: 'demande en attente de validation',
  refusee: 'demande refusée',
  suspendue: 'établissement suspendu',
  abandonnee: 'demande abandonnée',
};

/** Choix de l'espace (docs/api.md §2.6) : jamais d'activation automatique, même avec une seule ligne. */
@Component({
  selector: 'lc-espaces',
  imports: [CadreComponent, MessageEtatComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre [afficherChangerEspace]="false">
      <h1 class="page__titre">Choisissez votre espace</h1>

      @if (message(); as m) {
        <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
      }

      @if (chargement()) {
        <p role="status">Chargement…</p>
      } @else {
        <ul class="espaces">
          @for (affectation of affectations(); track affectation.id) {
            <li class="carte espace" [class.espace--indisponible]="!affectation.exercable">
              <div class="espace__texte">
                <span class="espace__nom">{{ affectation.structure_nom }}</span>
                <span class="espace__role">{{ libelleRole(affectation.role) }}</span>
                @if (!affectation.exercable) {
                  <span class="espace__raison">Indisponible : {{ raison(affectation) }}</span>
                }
              </div>
              <button
                type="button"
                class="bouton bouton--principal"
                [disabled]="!affectation.exercable || activationEnCours()"
                [attr.aria-label]="'Entrer dans ' + affectation.structure_nom"
                (click)="entrer(affectation)"
              >
                Entrer
              </button>
            </li>
          } @empty {
            <li class="carte">Aucun espace MUNASEB n'est rattaché à votre compte.</li>
          }
        </ul>
      }

      <p class="texte-secondaire">Vos autres espaces (pharmacie, laboratoire, hôpital) s'ouvrent dans leur propre application.</p>
    </lc-cadre>
  `,
})
export class EspacesPage implements OnInit {
  private readonly acces = inject(AccesService);
  private readonly router = inject(Router);

  protected readonly affectations = signal<Affectation[]>([]);
  protected readonly chargement = signal(true);
  protected readonly activationEnCours = signal(false);
  protected readonly message = signal<MessageAffiche | null>(inject(MessageNavigation).consommer());

  ngOnInit(): void {
    void this.charger();
  }

  protected libelleRole(role: string): string {
    return LIBELLES_ROLE[role] ?? role;
  }

  /** Raison d'une ligne grisée (§2.6) : statut de la structure, sinon licence expirée. */
  protected raison(affectation: Affectation): string {
    if (affectation.structure_statut !== 'validee') {
      return LIBELLES_STATUT_STRUCTURE[affectation.structure_statut] ?? affectation.structure_statut;
    }
    if (affectation.licence_expire_le !== null) {
      return `licence expirée le ${formaterJour(affectation.licence_expire_le)}`;
    }
    return 'espace non disponible';
  }

  async entrer(affectation: Affectation): Promise<void> {
    if (this.activationEnCours()) {
      return;
    }
    this.activationEnCours.set(true);
    this.message.set(null);
    try {
      await this.acces.activer(affectation);
      await this.router.navigate(['/demandes']);
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      const indisponible =
        decodee.genre === 'api' && (decodee.code === 'affectation_non_exercable' || decodee.code === 'affectation_inconnue');
      this.message.set({
        genre: 'erreur',
        texte: indisponible ? MESSAGES.affectationNonExercable : messagePourErreur(decodee),
      });
      if (indisponible) {
        await this.charger();
      }
    } finally {
      this.activationEnCours.set(false);
    }
  }

  private async charger(): Promise<void> {
    this.chargement.set(true);
    try {
      const toutes = await this.acces.listerAffectations();
      this.affectations.set(toutes.filter((a) => a.structure_type === 'munaseb'));
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      // Session terminée : l'intercepteur a déjà ramené à la connexion.
      if (!(decodee.genre === 'api' && decodee.code === 'jeton_invalide')) {
        this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
      }
    } finally {
      this.chargement.set(false);
    }
  }
}
