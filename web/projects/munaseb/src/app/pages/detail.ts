import { LiveAnnouncer } from '@angular/cdk/a11y';
import { ChangeDetectionStrategy, Component, OnInit, computed, inject, input, signal } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  DateHeurePipe,
  DemandeDetail,
  FcfaPipe,
  JourPipe,
  LigneHistorique,
  MessageAffiche,
  MessageEtatComponent,
  PastilleStatutComponent,
  ReponseValidation,
  decoderErreur,
  formaterFcfa,
  libelleStatut,
  messagePourErreur,
} from 'commun';
import { FenetreService } from 'commun/fenetre';
import { firstValueFrom } from 'rxjs';
import { CadreComponent } from '../cadre';
import {
  DonneesRejet,
  DonneesValidation,
  FenetreRejetComponent,
  FenetreValidationComponent,
  IssueFenetre,
} from '../fenetres';
import { RemboursementsService, libelleActe, nomAdherent } from '../remboursements';

export const MESSAGE_PLAFOND_ATTEINT =
  'Plafond de la période atteint : 0 FCFA remboursé. La demande reste validée. Marquez-la comme payée pour clore le dossier.';
export const MESSAGE_DEMANDE_INCONNUE = "Cette demande n'existe pas.";

/** Détail d'une demande et ses transitions (docs/api.md §2.3.5, §2.3.6). */
@Component({
  selector: 'lc-detail',
  imports: [CadreComponent, MessageEtatComponent, PastilleStatutComponent, RouterLink, FcfaPipe, JourPipe, DateHeurePipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <a class="lien-retour" routerLink="/demandes">Retour à la liste des demandes</a>

      @if (message(); as m) {
        <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
      }

      @if (introuvable()) {
        <lc-message-etat genre="erreur" [texte]="messageInconnue" />
      } @else if (demande(); as d) {
        <div class="detail__entete">
          <lc-pastille-statut [statut]="d.statut" />
          <h1 class="page__titre">{{ titre() }}</h1>
          <p class="texte-secondaire">Reçue le {{ d.date_demande | dateHeure }} · {{ d.partenaire }}</p>
        </div>

        @if (d.statut === 'valide' && d.montant_rembourse_fcfa === 0) {
          <lc-message-etat genre="info" [texte]="messagePlafond" />
        }

        @if (d.statut === 'rejete' && d.motif_rejet !== null) {
          <section class="carte" aria-labelledby="titre-motif">
            <h2 class="carte__titre" id="titre-motif">Motif du rejet</h2>
            <p class="motif">{{ d.motif_rejet }}</p>
          </section>
        }

        <div class="actions">
          @switch (d.statut) {
            @case ('en_attente') {
              <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="prendreEnCharge()">
                Prendre en charge
              </button>
              <button type="button" class="bouton bouton--sombre" [disabled]="envoiEnCours()" (click)="rejeter()">
                Rejeter
              </button>
            }
            @case ('en_cours') {
              <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="valider()">
                Valider
              </button>
              <button type="button" class="bouton bouton--sombre" [disabled]="envoiEnCours()" (click)="rejeter()">
                Rejeter
              </button>
            }
            @case ('valide') {
              <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="payer()">
                Marquer comme payée
              </button>
            }
          }
        </div>

        <div class="cartes">
          <section class="carte" aria-labelledby="titre-montants">
            <h2 class="carte__titre" id="titre-montants">Montants</h2>
            <dl class="donnees">
              <dt>Montant de l'acte</dt>
              <dd class="nombre">{{ d.montant_acte_fcfa | fcfa }}</dd>
              <dt>Demandé</dt>
              <dd class="nombre">{{ d.montant_demande_fcfa | fcfa }}</dd>
              <dt>Remboursé</dt>
              <dd class="nombre">
                @if (d.montant_rembourse_fcfa === null) {
                  <span aria-label="Pas encore fixé">—</span>
                } @else {
                  {{ d.montant_rembourse_fcfa | fcfa }}
                }
              </dd>
            </dl>
          </section>

          <section class="carte" aria-labelledby="titre-adherent">
            <h2 class="carte__titre" id="titre-adherent">Adhérent et acte</h2>
            <dl class="donnees">
              <dt>Adhérent</dt>
              <dd>{{ adherent() }}</dd>
              <dt>N° de carte</dt>
              <dd>{{ d.numero_carte }}</dd>
              <dt>Partenaire</dt>
              <dd>{{ d.partenaire }}</dd>
              <dt>Acte</dt>
              <dd>{{ acte() }}</dd>
              <dt>Date de l'acte</dt>
              <dd>{{ d.date_acte | jour }}</dd>
            </dl>
          </section>

          <section class="carte" aria-labelledby="titre-periode">
            <h2 class="carte__titre" id="titre-periode">Période de couverture</h2>
            <dl class="donnees">
              <dt>Du</dt>
              <dd>{{ d.periode_debut | jour }}</dd>
              <dt>Au</dt>
              <dd>{{ d.periode_fin | jour }}</dd>
              <dt>Solde restant</dt>
              <dd class="nombre">{{ d.solde_restant_periode_fcfa | fcfa }}</dd>
            </dl>
            <p class="champ__aide">Solde indicatif : il est recalculé au moment de la validation.</p>
          </section>
        </div>

        <section class="carte" aria-labelledby="titre-historique">
          <h2 class="carte__titre" id="titre-historique">Historique</h2>
          <ol class="historique">
            @for (ligne of historique(); track $index) {
              <li>
                <span class="historique__statut">{{ statut(ligne) }}</span>
                <span class="texte-secondaire">
                  le {{ ligne.date_changement | dateHeure }}, {{ auteur(ligne) }}
                </span>
              </li>
            }
          </ol>
        </section>
      } @else if (chargement()) {
        <p role="status">Chargement…</p>
      }
    </lc-cadre>
  `,
})
export class DetailPage implements OnInit {
  private readonly remboursements = inject(RemboursementsService);
  private readonly fenetres = inject(FenetreService);
  private readonly annonceur = inject(LiveAnnouncer);

  /** Paramètre `:id` de la route (withComponentInputBinding). */
  readonly id = input.required<string>();

  protected readonly messagePlafond = MESSAGE_PLAFOND_ATTEINT;
  protected readonly messageInconnue = MESSAGE_DEMANDE_INCONNUE;
  protected readonly demande = signal<DemandeDetail | null>(null);
  protected readonly chargement = signal(true);
  protected readonly introuvable = signal(false);
  protected readonly envoiEnCours = signal(false);
  protected readonly message = signal<MessageAffiche | null>(null);

  protected readonly titre = computed(() => {
    const d = this.demande();
    return d === null ? '' : `${libelleActe(d.type_acte)} · ${nomAdherent(d)}`;
  });
  protected readonly adherent = computed(() => {
    const d = this.demande();
    return d === null ? '' : nomAdherent(d);
  });
  protected readonly acte = computed(() => {
    const d = this.demande();
    return d === null ? '' : libelleActe(d.type_acte);
  });
  /** Du plus récent au plus ancien. */
  protected readonly historique = computed(() =>
    [...(this.demande()?.historique ?? [])].sort(
      (a, b) => Date.parse(b.date_changement) - Date.parse(a.date_changement),
    ),
  );

  ngOnInit(): void {
    void this.charger();
  }

  protected statut(ligne: LigneHistorique): string {
    return libelleStatut(ligne.statut_nouveau);
  }

  /** §2.3.5 : agent `null` pour la création, faite par le partenaire. */
  protected auteur(ligne: LigneHistorique): string {
    const nom = [ligne.agent_prenom, ligne.agent_nom].filter((m) => m !== null && m !== '').join(' ');
    return nom === '' ? 'par le partenaire' : `par ${nom}`;
  }

  prendreEnCharge(): void {
    void this.executer(() => this.remboursements.prendreEnCharge(this.id()), 'La demande est prise en charge.');
  }

  payer(): void {
    void this.executer(() => this.remboursements.payer(this.id()), 'La demande est marquée comme payée.');
  }

  async valider(): Promise<void> {
    const d = this.demande();
    if (d === null || this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    const donnees: DonneesValidation = {
      montantDemande: d.montant_demande_fcfa,
      solde: d.solde_restant_periode_fcfa,
      envoyer: () => this.remboursements.valider(this.id()),
    };
    const ref = this.fenetres.ouvrir<IssueFenetre<ReponseValidation>, DonneesValidation, FenetreValidationComponent>(
      FenetreValidationComponent,
      donnees,
      'titre-validation',
    );
    const issue = await firstValueFrom(ref.closed);
    if (issue?.ok === true) {
      const montant = issue.resultat.montant_rembourse_fcfa;
      // 0 FCFA n'est pas une erreur (§2.3.6) : l'encadré d'information s'affiche avec le détail rechargé.
      await this.reussir(
        montant === 0 ? MESSAGE_PLAFOND_ATTEINT : `La demande est validée : ${formaterFcfa(montant)} remboursés.`,
        montant !== 0,
      );
    } else if (issue?.ok === false) {
      await this.traiterErreur(issue.erreur);
    }
    this.envoiEnCours.set(false);
  }

  async rejeter(): Promise<void> {
    if (this.demande() === null || this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    const donnees: DonneesRejet = { envoyer: (motif) => this.remboursements.rejeter(this.id(), motif) };
    const ref = this.fenetres.ouvrir<IssueFenetre<void>, DonneesRejet, FenetreRejetComponent>(
      FenetreRejetComponent,
      donnees,
      'titre-rejet',
    );
    const issue = await firstValueFrom(ref.closed);
    if (issue?.ok === true) {
      await this.reussir('La demande est rejetée.', true);
    } else if (issue?.ok === false) {
      await this.traiterErreur(issue.erreur);
    }
    this.envoiEnCours.set(false);
  }

  /** Une transition à la fois : les boutons restent désactivés jusqu'à la réponse. */
  private async executer(action: () => Promise<void>, annonce: string): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    try {
      await action();
      await this.reussir(annonce, true);
    } catch (erreur) {
      await this.traiterErreur(erreur);
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  /** Après un succès : détail rechargé, résultat annoncé aux lecteurs d'écran (CDK LiveAnnouncer). */
  private async reussir(texte: string, afficher: boolean): Promise<void> {
    await this.charger();
    if (afficher) {
      this.message.set({ genre: 'info', texte });
    }
    void this.annonceur.announce(texte);
  }

  private async traiterErreur(erreur: unknown): Promise<void> {
    const decodee = decoderErreur(erreur);
    if (decodee.genre === 'api') {
      switch (decodee.code) {
        case 'transition_invalide': {
          const actuel = decodee.corps.statut_actuel;
          this.message.set({
            genre: 'erreur',
            texte: `Cette demande a changé entre-temps. Elle est maintenant : ${actuel === undefined ? 'inconnu' : libelleStatut(actuel)}.`,
          });
          await this.charger();
          return;
        }
        case 'demande_inconnue':
          this.introuvable.set(true);
          return;
        case 'jeton_invalide':
        case 'acces_refuse':
          // L'intercepteur a déjà changé d'écran.
          return;
      }
    }
    this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
  }

  private async charger(): Promise<void> {
    this.chargement.set(true);
    try {
      this.demande.set(await this.remboursements.detail(this.id()));
      this.introuvable.set(false);
    } catch (erreur) {
      const decodee = decoderErreur(erreur);
      // Un identifiant qui n'est pas un UUID (400 chemin_invalide) ne désigne aucune demande non plus.
      if (decodee.genre === 'api' && (decodee.code === 'demande_inconnue' || decodee.code === 'chemin_invalide')) {
        this.demande.set(null);
        this.introuvable.set(true);
      } else if (!(decodee.genre === 'api' && (decodee.code === 'jeton_invalide' || decodee.code === 'acces_refuse'))) {
        this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
      }
    } finally {
      this.chargement.set(false);
    }
  }
}
