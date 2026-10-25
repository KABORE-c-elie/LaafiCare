import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  DateHeurePipe,
  MessageAffiche,
  MessageEtatComponent,
  ResumeStructure,
  StatutStructure,
  codeErreur,
  decoderErreur,
  messagePourErreur,
} from 'commun';
import { CadreComponent } from '../cadre';
import { StatutStructureComponent } from '../statut-structure';
import { StructuresService, libelleType, nomComplet } from '../structures';

interface Filtre {
  libelle: string;
  statut: StatutStructure | null;
}

// « En attente » d'abord et par défaut : c'est la file de travail de l'équipe (porteur, B6).
const FILTRES: readonly Filtre[] = [
  { libelle: 'En attente', statut: 'en_attente' },
  { libelle: 'Toutes', statut: null },
  { libelle: 'Validées', statut: 'validee' },
  { libelle: 'Refusées', statut: 'refusee' },
  { libelle: 'Brouillons', statut: 'brouillon' },
  { libelle: 'Suspendues', statut: 'suspendue' },
  { libelle: 'Abandonnées', statut: 'abandonnee' },
];

/** Demandes de structure (docs/api.md §2.9), les plus anciennes d'abord, par pages de 20. */
@Component({
  selector: 'lc-structures',
  imports: [CadreComponent, MessageEtatComponent, StatutStructureComponent, RouterLink, DateHeurePipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <div class="barre">
        <h1 class="page__titre">Demandes de structure</h1>
        <button type="button" class="bouton bouton--secondaire" [disabled]="chargement()" (click)="actualiser()">
          Actualiser
        </button>
      </div>

      <div class="pilules" role="group" aria-label="Filtrer par statut">
        @for (filtre of filtres; track filtre.libelle) {
          <button
            type="button"
            class="pilule"
            [class.pilule--active]="filtre.statut === statut()"
            [attr.aria-pressed]="filtre.statut === statut()"
            (click)="filtrer(filtre.statut)"
          >
            {{ filtre.libelle }}
          </button>
        }
      </div>

      @if (message(); as m) {
        <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
      }

      <div class="tableau-defilant" tabindex="0" role="region" aria-labelledby="legende-structures">
        <table class="tableau tableau--structures">
          <caption id="legende-structures" class="visuellement-cache">Demandes de structure</caption>
          <thead>
            <tr>
              <th scope="col">Nom</th>
              <th scope="col">Type</th>
              <th scope="col">Commune</th>
              <th scope="col">Responsable</th>
              <th scope="col">Créée le</th>
              <th scope="col">Statut</th>
              <th scope="col"><span class="visuellement-cache">Ouvrir</span></th>
            </tr>
          </thead>
          <tbody>
            @for (structure of structures(); track structure.id) {
              <tr>
                <td [title]="structure.nom">{{ structure.nom }}</td>
                <td>{{ type(structure) }}</td>
                <td [title]="structure.commune">{{ structure.commune }}</td>
                <td>{{ responsable(structure) }}</td>
                <td>{{ structure.date_creation | dateHeure }}</td>
                <td><lc-statut-structure [statut]="structure.statut" /></td>
                <td>
                  <a class="lien-ouvrir" [routerLink]="['/structures', structure.id]">
                    Ouvrir<span class="visuellement-cache"> la demande {{ structure.nom }}</span>
                  </a>
                </td>
              </tr>
            }
          </tbody>
        </table>
      </div>

      @if (chargement()) {
        <p role="status">Chargement…</p>
      } @else if (structures().length === 0 && message() === null) {
        <p class="liste-vide" role="status">Aucune demande avec ce statut.</p>
      }

      @if (suivant() !== null && !chargement()) {
        <button type="button" class="bouton bouton--secondaire" (click)="suivante()">
          Afficher les demandes suivantes
        </button>
      }
    </lc-cadre>
  `,
})
export class StructuresPage implements OnInit {
  private readonly service = inject(StructuresService);

  protected readonly filtres = FILTRES;
  protected readonly statut = signal<StatutStructure | null>('en_attente');
  protected readonly structures = signal<ResumeStructure[]>([]);
  protected readonly suivant = signal<string | null>(null);
  protected readonly chargement = signal(false);
  protected readonly message = signal<MessageAffiche | null>(null);

  ngOnInit(): void {
    void this.charger(null);
  }

  protected type(structure: ResumeStructure): string {
    return libelleType(structure.type);
  }

  protected responsable(structure: ResumeStructure): string {
    return nomComplet(structure.responsable_nom, structure.responsable_prenom);
  }

  filtrer(statut: StatutStructure | null): void {
    this.statut.set(statut);
    void this.charger(null);
  }

  actualiser(): void {
    void this.charger(null);
  }

  /** Page suivante : `apres` = `suivant` de la page précédente (§2.9). */
  suivante(): void {
    void this.charger(this.suivant());
  }

  private async charger(apres: string | null): Promise<void> {
    if (apres === null) {
      this.structures.set([]);
      this.suivant.set(null);
    }
    this.chargement.set(true);
    this.message.set(null);
    try {
      const page = await this.service.lister(this.statut(), apres);
      this.structures.update((deja) => [...deja, ...page.structures]);
      this.suivant.set(page.suivant);
    } catch (erreur) {
      // jeton_invalide : l'intercepteur a déjà ramené à la connexion.
      if (codeErreur(erreur) !== 'jeton_invalide') {
        this.message.set({ genre: 'erreur', texte: messagePourErreur(decoderErreur(erreur)) });
      }
    } finally {
      this.chargement.set(false);
    }
  }
}
