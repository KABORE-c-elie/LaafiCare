import { ChangeDetectionStrategy, Component, OnInit, effect, inject, signal, untracked } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  DateHeurePipe,
  DemandeResume,
  FcfaPipe,
  JourPipe,
  MessageAffiche,
  MessageEtatComponent,
  PastilleStatutComponent,
  ReseauService,
  StatutDemande,
  codeErreur,
  decoderErreur,
  messagePourErreur,
} from 'commun';
import { CadreComponent } from '../cadre';
import { RemboursementsService, libelleActe, nomAdherent } from '../remboursements';

interface Filtre {
  libelle: string;
  statut: StatutDemande | null;
}

const FILTRES: readonly Filtre[] = [
  { libelle: 'Toutes', statut: null },
  { libelle: 'En attente', statut: 'en_attente' },
  { libelle: 'En cours', statut: 'en_cours' },
  { libelle: 'Validées', statut: 'valide' },
  { libelle: 'Payées', statut: 'paye' },
  { libelle: 'Rejetées', statut: 'rejete' },
];

/** Liste des demandes (docs/api.md §2.3.4), la plus ancienne d'abord, par pages de 20. */
@Component({
  selector: 'lc-demandes',
  imports: [CadreComponent, MessageEtatComponent, PastilleStatutComponent, RouterLink, FcfaPipe, JourPipe, DateHeurePipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <div class="barre">
        <h1 class="page__titre">Demandes de remboursement</h1>
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

      <div class="tableau-defilant" tabindex="0" role="region" aria-labelledby="legende-demandes">
        <table class="tableau">
          <caption id="legende-demandes" class="visuellement-cache">Demandes de remboursement</caption>
          <colgroup>
            <col class="col-date" />
            <col class="col-adherent" />
            <col class="col-carte" />
            <col class="col-partenaire" />
            <col class="col-acte" />
            <col class="col-jour" />
            <col class="col-montant" />
            <col class="col-montant" />
            <col class="col-montant" />
            <col class="col-statut" />
            <col class="col-ouvrir" />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">Reçue le</th>
              <th scope="col">Adhérent</th>
              <th scope="col">N° de carte</th>
              <th scope="col">Partenaire</th>
              <th scope="col">Acte</th>
              <th scope="col">Date de l'acte</th>
              <th scope="col" class="nombre">Montant de l'acte</th>
              <th scope="col" class="nombre">Demandé</th>
              <th scope="col" class="nombre">Remboursé</th>
              <th scope="col">Statut</th>
              <th scope="col"><span class="visuellement-cache">Ouvrir</span></th>
            </tr>
          </thead>
          <tbody>
            @for (demande of demandes(); track demande.id) {
              <tr>
                <td [title]="demande.date_demande | dateHeure">{{ demande.date_demande | dateHeure }}</td>
                <td [title]="adherent(demande)">{{ adherent(demande) }}</td>
                <td [title]="demande.numero_carte">{{ demande.numero_carte }}</td>
                <td [title]="demande.partenaire">{{ demande.partenaire }}</td>
                <td>{{ acte(demande) }}</td>
                <td>{{ demande.date_acte | jour }}</td>
                <td class="nombre">{{ demande.montant_acte_fcfa | fcfa }}</td>
                <td class="nombre">{{ demande.montant_demande_fcfa | fcfa }}</td>
                <td class="nombre">
                  @if (demande.montant_rembourse_fcfa === null) {
                    <span aria-label="Pas encore fixé">—</span>
                  } @else {
                    {{ demande.montant_rembourse_fcfa | fcfa }}
                  }
                </td>
                <td><lc-pastille-statut [statut]="demande.statut" /></td>
                <td>
                  <a class="lien-ouvrir" [routerLink]="['/demandes', demande.id]">
                    Ouvrir<span class="visuellement-cache"> la demande de {{ adherent(demande) }}</span>
                  </a>
                </td>
              </tr>
            }
          </tbody>
        </table>
      </div>

      @if (chargement()) {
        <p role="status">Chargement…</p>
      } @else if (demandes().length === 0 && message() === null && !echecReseau()) {
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
export class DemandesPage implements OnInit {
  private readonly remboursements = inject(RemboursementsService);
  private readonly reseau = inject(ReseauService);
  /** Dernier chargement échoué faute de réseau : la liste se recharge dès le retour du réseau. */
  protected readonly echecReseau = signal(false);

  protected readonly filtres = FILTRES;
  protected readonly statut = signal<StatutDemande | null>(null);
  protected readonly demandes = signal<DemandeResume[]>([]);
  protected readonly suivant = signal<string | null>(null);
  protected readonly chargement = signal(false);
  protected readonly message = signal<MessageAffiche | null>(null);

  constructor() {
    effect(() => {
      if (this.reseau.enLigne() && untracked(this.echecReseau)) {
        void this.charger(null);
      }
    });
  }

  ngOnInit(): void {
    void this.charger(null);
  }

  protected adherent(demande: DemandeResume): string {
    return nomAdherent(demande);
  }

  protected acte(demande: DemandeResume): string {
    return libelleActe(demande.type_acte);
  }

  filtrer(statut: StatutDemande | null): void {
    this.statut.set(statut);
    void this.charger(null);
  }

  actualiser(): void {
    void this.charger(null);
  }

  /** Page suivante : `apres` = `suivant` de la page précédente (§2.3.4). */
  suivante(): void {
    void this.charger(this.suivant());
  }

  private async charger(apres: string | null): Promise<void> {
    if (apres === null) {
      this.demandes.set([]);
      this.suivant.set(null);
    }
    this.chargement.set(true);
    this.message.set(null);
    this.echecReseau.set(false);
    try {
      const page = await this.remboursements.lister(this.statut(), apres);
      this.demandes.update((deja) => [...deja, ...page.demandes]);
      this.suivant.set(page.suivant);
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      // Hors connexion, le bandeau général le dit déjà ; la liste revient avec le réseau.
      if (decodee.genre === 'reseau') {
        this.echecReseau.set(true);
        if (this.reseau.enLigne()) {
          this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
        }
        return;
      }
      // jeton_invalide et acces_refuse : l'intercepteur a déjà changé d'écran.
      const code = codeErreur(erreur);
      if (code !== 'jeton_invalide' && code !== 'acces_refuse') {
        this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
      }
    } finally {
      this.chargement.set(false);
    }
  }
}
