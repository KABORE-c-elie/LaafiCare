import { ChangeDetectionStrategy, Component, ElementRef, effect, inject, input, signal, untracked, viewChild } from '@angular/core';
import { RouterLink } from '@angular/router';
import {
  DateHeurePipe,
  DetailStructure,
  JourPipe,
  MessageAffiche,
  MessageEtatComponent,
  decoderErreur,
  jourAuBurkina,
  lendemain,
  messagePourErreur,
} from 'commun';
import { DonneesMotif, FenetreMotifComponent, FenetreService, IssueFenetre } from 'commun/fenetre';
import { firstValueFrom } from 'rxjs';
import { CadreComponent } from '../cadre';
import { DonneesValidationStructure, FenetreValidationStructureComponent } from '../fenetre-validation';
import { StatutStructureComponent } from '../statut-structure';
import {
  MOTIF_REFUS_MAX,
  StructuresService,
  libelleDecision,
  libelleStatutStructure,
  libelleType,
  nomComplet,
} from '../structures';

// Textes d'après le champ `code` (docs/api.md §2.9), validés par le porteur (M1, 2026-10-22).
export const MESSAGES_STRUCTURE = {
  validee: 'La demande est validée.',
  refusee: 'La demande est refusée.',
  propreStructure: 'Vous êtes responsable de cette structure : un autre membre de l’équipe doit décider.',
  munasebDejaValidee: 'Une MUNASEB est déjà validée : une seule est admise.',
  emailNonVerifie: "L'email du responsable n'est pas encore vérifié : la validation attend. Le refus reste possible.",
  autorisationDejaValidee: 'Un numéro d’autorisation de cette demande est déjà validé pour une autre structure.',
  inconnue: "Cette structure n'existe pas.",
  motifVide: 'Saisissez le motif du refus.',
  motifTropLong: '1 000 caractères au plus.',
} as const;

const PAR_CODE: Readonly<Record<string, string>> = {
  decision_sur_sa_propre_structure: MESSAGES_STRUCTURE.propreStructure,
  munaseb_deja_validee: MESSAGES_STRUCTURE.munasebDejaValidee,
  email_responsable_non_verifie: MESSAGES_STRUCTURE.emailNonVerifie,
  autorisation_deja_validee: MESSAGES_STRUCTURE.autorisationDejaValidee,
};

/**
 * Détail d'une demande de structure et décisions de l'équipe (docs/api.md
 * §2.9) : validation (licence) ou refus (motif), depuis `en_attente`
 * seulement. Chaque décision est tracée par le serveur, sans jamais être modifiée.
 */
@Component({
  selector: 'lc-structure',
  imports: [CadreComponent, MessageEtatComponent, StatutStructureComponent, RouterLink, DateHeurePipe, JourPipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <a class="lien-retour" routerLink="/structures">← Demandes de structure</a>

      @if (introuvable()) {
        <h1 class="page__titre">Demande introuvable</h1>
        <lc-message-etat genre="erreur" [texte]="textes.inconnue" />
      } @else if (structure(); as s) {
        <div class="detail__entete">
          <h1 #titre class="page__titre" tabindex="-1">{{ s.nom }}</h1>
          <lc-statut-structure [statut]="s.statut" />
        </div>

        @if (message(); as m) {
          <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
        }

        @if (s.munaseb_deja_validee !== null) {
          <div class="message message--info avertissement">
            <p>
              Une MUNASEB est déjà validée ou suspendue : une seule est admise.
              <a [routerLink]="['/structures', s.munaseb_deja_validee]">Ouvrir la MUNASEB validée</a>
            </p>
          </div>
        }
        @if (s.autres_demandes_munaseb_en_attente.length > 0) {
          <div class="message message--info avertissement">
            <p>
              {{ s.autres_demandes_munaseb_en_attente.length === 1 ? 'Une autre demande' : s.autres_demandes_munaseb_en_attente.length + ' autres demandes' }}
              MUNASEB {{ s.autres_demandes_munaseb_en_attente.length === 1 ? 'attend' : 'attendent' }} une décision :
            </p>
            <ul>
              @for (autre of s.autres_demandes_munaseb_en_attente; track autre; let i = $index) {
                <li><a [routerLink]="['/structures', autre]">Ouvrir l’autre demande {{ i + 1 }}</a></li>
              }
            </ul>
          </div>
        }
        @if (s.statut === 'en_attente' && !s.responsable_email_verifie) {
          <lc-message-etat genre="info" [texte]="textes.emailNonVerifie" />
        }

        @if (s.statut === 'en_attente') {
          <div class="actions">
            <button type="button" class="bouton bouton--principal" [disabled]="envoiEnCours()" (click)="valider()">
              Valider
            </button>
            <button type="button" class="bouton bouton--sombre" [disabled]="envoiEnCours()" (click)="refuser()">
              Refuser
            </button>
          </div>
        }

        <div class="cartes">
          <section class="carte" aria-labelledby="titre-structure">
            <h2 class="carte__titre" id="titre-structure">Structure</h2>
            <dl class="donnees">
              <dt>Type</dt>
              <dd>{{ type(s.type) }}</dd>
              <dt>Créée le</dt>
              <dd>{{ s.date_creation | dateHeure }}</dd>
              <dt>Téléphone</dt>
              <dd>{{ s.telephone }}</dd>
              <dt>Téléphone d'urgence</dt>
              <dd>{{ s.telephone_urgence }}</dd>
              <dt>Email</dt>
              <dd>{{ s.email }}</dd>
              <dt>Licence</dt>
              <dd>{{ s.licence_expire_le === null ? (s.statut === 'validee' ? 'sans fin' : '—') : 'jusqu’au ' + (s.licence_expire_le | jour) }}</dd>
            </dl>
          </section>

          <section class="carte" aria-labelledby="titre-adresse">
            <h2 class="carte__titre" id="titre-adresse">Adresse</h2>
            <dl class="donnees">
              <dt>Région</dt>
              <dd>{{ s.region }}</dd>
              <dt>Province</dt>
              <dd>{{ s.province }}</dd>
              <dt>Commune</dt>
              <dd>{{ s.commune }}</dd>
              <dt>Parcelle, lot, section</dt>
              <dd>{{ cadastre(s) }}</dd>
              <dt>Position</dt>
              <dd>{{ s.latitude }}, {{ s.longitude }}</dd>
            </dl>
          </section>

          <section class="carte" aria-labelledby="titre-responsable">
            <h2 class="carte__titre" id="titre-responsable">Responsable</h2>
            <dl class="donnees">
              <dt>Nom</dt>
              <dd>{{ nom(s.responsable_nom, s.responsable_prenom) }}</dd>
              <dt>Téléphone</dt>
              <dd>{{ s.responsable_telephone }}</dd>
              <dt>Email</dt>
              <dd>{{ s.responsable_email ?? '—' }}</dd>
              <dt>Email vérifié</dt>
              <dd>{{ s.responsable_email_verifie ? 'oui' : 'non' }}</dd>
            </dl>
          </section>
        </div>

        <section class="carte" aria-labelledby="titre-decisions">
          <h2 class="carte__titre" id="titre-decisions">Décisions de l'équipe</h2>
          @if (s.decisions.length === 0) {
            <p class="liste-vide">Aucune décision pour l'instant.</p>
          } @else {
            <ol class="historique">
              @for (d of s.decisions; track d.date_decision) {
                <li>
                  <span class="historique__statut">{{ decision(d.decision) }}</span>
                  <span>le {{ d.date_decision | dateHeure }}</span>
                  <span>par {{ nom(d.administrateur_nom, d.administrateur_prenom) }}</span>
                  @if (d.licence_expire_le !== null) {
                    <span>licence jusqu’au {{ d.licence_expire_le | jour }}</span>
                  }
                  @if (d.motif !== null) {
                    <span class="motif">Motif : {{ d.motif }}</span>
                  }
                </li>
              }
            </ol>
          }
        </section>
      } @else {
        @if (message(); as m) {
          <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
        } @else {
          <p role="status">Chargement…</p>
        }
      }
    </lc-cadre>
  `,
})
export class StructurePage {
  private readonly service = inject(StructuresService);
  private readonly fenetres = inject(FenetreService);
  private readonly titre = viewChild<ElementRef<HTMLElement>>('titre');

  /** Paramètre `:id` de la route (withComponentInputBinding). */
  readonly id = input.required<string>();

  protected readonly textes = MESSAGES_STRUCTURE;
  protected readonly structure = signal<DetailStructure | null>(null);
  protected readonly introuvable = signal(false);
  protected readonly envoiEnCours = signal(false);
  protected readonly message = signal<MessageAffiche | null>(null);

  constructor() {
    // Un lien vers une autre demande garde la même page : on recharge à chaque identifiant.
    effect(() => {
      const id = this.id();
      untracked(() => {
        this.structure.set(null);
        this.message.set(null);
        void this.charger(id);
      });
    });
  }

  protected type(valeur: string): string {
    return libelleType(valeur);
  }

  protected decision(valeur: string): string {
    return libelleDecision(valeur);
  }

  protected nom(nom: string | null, prenom: string | null): string {
    return nomComplet(nom, prenom);
  }

  protected cadastre(s: DetailStructure): string {
    const morceaux = [s.parcelle, s.lot, s.section].filter((m) => m !== null && m.trim() !== '');
    return morceaux.length === 0 ? '—' : morceaux.join(', ');
  }

  async valider(): Promise<void> {
    const s = this.structure();
    if (s === null || this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    const donnees: DonneesValidationStructure = {
      nom: s.nom,
      // §2.9 : date « postérieure à aujourd'hui », jour du Burkina (D9) : demain au plus tôt.
      demain: lendemain(jourAuBurkina()),
      envoyer: (licence) => this.service.valider(s.id, licence),
    };
    const ref = this.fenetres.ouvrir<IssueFenetre<void>, DonneesValidationStructure, FenetreValidationStructureComponent>(
      FenetreValidationStructureComponent,
      donnees,
      'titre-validation',
    );
    await this.apresFenetre(await firstValueFrom(ref.closed), MESSAGES_STRUCTURE.validee);
  }

  async refuser(): Promise<void> {
    const s = this.structure();
    if (s === null || this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.message.set(null);
    const donnees: DonneesMotif = {
      titre: 'Refuser la demande',
      idTitre: 'titre-refus',
      libelle: 'Motif du refus',
      action: 'Refuser la demande',
      // §2.9 : visible par le responsable dans le suivi de sa demande, jamais mis dans un email.
      rappel: 'Le responsable lit ce motif dans le suivi de sa demande ; il n’est jamais mis dans un email.',
      max: MOTIF_REFUS_MAX,
      maxAffiche: '1 000',
      messageVide: MESSAGES_STRUCTURE.motifVide,
      messageTropLong: MESSAGES_STRUCTURE.motifTropLong,
      envoyer: (motif) => this.service.refuser(s.id, motif),
    };
    const ref = this.fenetres.ouvrir<IssueFenetre<void>, DonneesMotif, FenetreMotifComponent>(
      FenetreMotifComponent,
      donnees,
      donnees.idTitre,
    );
    await this.apresFenetre(await firstValueFrom(ref.closed), MESSAGES_STRUCTURE.refusee);
  }

  private async apresFenetre(issue: IssueFenetre<void> | undefined, succes: string): Promise<void> {
    try {
      if (issue === undefined) {
        return;
      }
      await this.charger(this.id());
      if (issue.ok) {
        this.message.set({ genre: 'info', texte: succes });
      } else {
        this.traiterErreur(issue.erreur);
      }
      // Le bouton cliqué peut disparaître avec le nouveau statut : le focus passe au titre (WCAG 2.2, 2.4.3).
      this.titre()?.nativeElement.focus();
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  private traiterErreur(erreur: unknown): void {
    const decodee = decoderErreur(erreur);
    if (decodee.genre === 'api') {
      if (decodee.code === 'jeton_invalide') {
        return;
      }
      if (decodee.code === 'transition_invalide') {
        const actuel = decodee.corps.statut_actuel;
        this.message.set({
          genre: 'erreur',
          texte: `Cette demande a changé entre-temps. Elle est maintenant : ${
            actuel === undefined ? 'inconnu' : libelleStatutStructure(actuel).toLowerCase()
          }.`,
        });
        return;
      }
      if (decodee.code === 'structure_inconnue') {
        this.introuvable.set(true);
        return;
      }
      const texte = PAR_CODE[decodee.code];
      if (texte !== undefined) {
        this.message.set({ genre: 'erreur', texte });
        return;
      }
    }
    this.message.set({ genre: 'erreur', texte: messagePourErreur(decodee) });
  }

  private async charger(id: string): Promise<void> {
    try {
      this.structure.set(await this.service.detail(id));
      this.introuvable.set(false);
    } catch (erreur) {
      this.traiterErreur(erreur);
    }
  }
}
