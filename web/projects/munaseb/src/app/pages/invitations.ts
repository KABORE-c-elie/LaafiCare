import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from '@angular/core';
import { FormControl, FormGroup, ReactiveFormsModule } from '@angular/forms';
import {
  ChampComponent,
  DateHeurePipe,
  IconeComponent,
  Invitation,
  InvitationEnvoyee,
  MessageAffiche,
  MessageEtatComponent,
  NomIcone,
  StatutInvitation,
  codeErreur,
  decoderErreur,
  formaterDateBurkina,
  messagePourErreur,
} from 'commun';
import { FenetreService } from 'commun/fenetre';
import { firstValueFrom } from 'rxjs';
import { CadreComponent } from '../cadre';
import { DonneesConfirmation, FenetreConfirmationComponent, IssueFenetre } from '../fenetres';
import { InvitationsService, libelleStatutInvitation } from '../invitations';
import { LIBELLES_ROLE, RoleMunaseb, estRoleMunaseb } from '../roles';

// Textes affichés d'après le champ `code` (docs/api.md §2.10), validés par le porteur (M1, 2026-10-22).
export const MESSAGES_INVITATION = {
  envoyee: 'Invitation envoyée.',
  annulee: 'Invitation annulée.',
  telephoneVide: 'Saisissez un numéro de téléphone.',
  telephoneInvalide: 'Numéro non reconnu. Exemple : 70 00 00 00.',
  roleNonPermis: "Ce rôle n'est pas disponible pour cette structure.",
  autoInvitation: 'Vous ne pouvez pas inviter votre propre numéro.',
  dejaEnAttente:
    'Une invitation est déjà en attente pour ce numéro et ce rôle. Annulez-la pour en envoyer une nouvelle.',
  tropEnAttente: '50 invitations sont déjà en attente. Annulez celles qui ne servent plus.',
  structureNonValidee: "Votre structure n'est pas validée : les invitations sont impossibles pour le moment.",
  tropDeDemandes: 'Vingt invitations ont été envoyées en 24 heures. Réessayez plus tard.',
  inconnue: "Cette invitation n'existe plus.",
  expiree: 'Cette invitation a expiré.',
} as const;

const PAR_CODE_ENVOI: Readonly<Record<string, string>> = {
  role_non_permis: MESSAGES_INVITATION.roleNonPermis,
  auto_invitation: MESSAGES_INVITATION.autoInvitation,
  invitation_deja_en_attente: MESSAGES_INVITATION.dejaEnAttente,
  trop_d_invitations_en_attente: MESSAGES_INVITATION.tropEnAttente,
  structure_non_validee: MESSAGES_INVITATION.structureNonValidee,
  // §2.10 ne donne pas de `Retry-After` pour cette limite : pas de décompte.
  trop_de_demandes: MESSAGES_INVITATION.tropDeDemandes,
};

// Statuts neutres, « Acceptée » en vert ; jamais de rouge ni d'orange (CLAUDE.md §6).
// Icône et texte : jamais la couleur seule (WCAG 2.2, 1.4.1).
const PASTILLES: Readonly<Record<StatutInvitation, { classe: string; icone: NomIcone }>> = {
  en_attente: { classe: 'pastille--en_attente', icone: 'attente' },
  acceptee: { classe: 'pastille--valide', icone: 'valide' },
  refusee: { classe: 'pastille--rejete', icone: 'rejete' },
  annulee: { classe: 'pastille--rejete', icone: 'rejete' },
  expiree: { classe: 'pastille--rejete', icone: 'attente' },
};

/**
 * Invitations du responsable (docs/api.md §2.10) : inviter au numéro, suivre et
 * annuler. La réponse est la même que le numéro ait un compte ou non : la page
 * n'en dit jamais plus. Le numéro saisi n'est écrit ni dans la console ni dans
 * le journal de développement (qui ne garde que la méthode et le chemin).
 */
@Component({
  selector: 'lc-invitations',
  imports: [CadreComponent, ChampComponent, IconeComponent, MessageEtatComponent, ReactiveFormsModule, DateHeurePipe],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <lc-cadre>
      <h1 class="page__titre">Invitations</h1>

      <section class="carte section" aria-labelledby="titre-inviter">
        <h2 class="section__titre" id="titre-inviter">Inviter une personne</h2>
        <form class="formulaire" [formGroup]="formulaire" (ngSubmit)="inviter()" novalidate>
          @if (messageEnvoi(); as m) {
            <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
          }

          <lc-champ
            libelle="Numéro de téléphone"
            pour="telephone"
            aide="8 chiffres, par exemple 70 00 00 00. Un numéro étranger commence par +."
            [erreur]="erreurTelephone()"
          >
            <input
              id="telephone"
              class="champ__saisie"
              type="tel"
              inputmode="tel"
              autocomplete="off"
              formControlName="telephone"
              [attr.aria-invalid]="erreurTelephone() !== null"
              [attr.aria-describedby]="erreurTelephone() !== null ? 'telephone-aide telephone-erreur' : 'telephone-aide'"
            />
          </lc-champ>

          <fieldset class="choix">
            <legend class="champ__libelle">Rôle</legend>
            <label class="choix__option">
              <input type="radio" formControlName="role" value="agent_assurance_munaseb" />
              <span>{{ libelles.agent_assurance_munaseb }}</span>
            </label>
            <label class="choix__option">
              <input type="radio" formControlName="role" value="responsable" />
              <span>{{ libelles.responsable }} : peut inviter et retirer des accès</span>
            </label>
          </fieldset>

          <button type="submit" class="bouton bouton--principal" [disabled]="envoiEnCours()">
            {{ envoiEnCours() ? 'Envoi…' : 'Envoyer l’invitation' }}
          </button>
        </form>
      </section>

      <div class="barre">
        <h2 class="section__titre">Invitations envoyées</h2>
        <button type="button" class="bouton bouton--secondaire" [disabled]="chargement()" (click)="actualiser()">
          Actualiser
        </button>
      </div>

      @if (messageListe(); as m) {
        <lc-message-etat [genre]="m.genre" [texte]="m.texte" />
      }

      <div class="tableau-defilant" tabindex="0" role="region" aria-labelledby="legende-invitations">
        <table class="tableau">
          <caption id="legende-invitations" class="visuellement-cache">Invitations envoyées</caption>
          <thead>
            <tr>
              <th scope="col">Téléphone</th>
              <th scope="col">Rôle</th>
              <th scope="col">Statut</th>
              <th scope="col">Envoyée le</th>
              <th scope="col">Expire le</th>
              <th scope="col">Personne</th>
              <th scope="col"><span class="visuellement-cache">Action</span></th>
            </tr>
          </thead>
          <tbody>
            @for (invitation of invitations(); track invitation.id) {
              <tr>
                <td>{{ invitation.telephone }}</td>
                <td>{{ libelleRole(invitation.role) }}</td>
                <td>
                  <span [class]="'pastille ' + pastille(invitation).classe">
                    <lc-icone [nom]="pastille(invitation).icone" />
                    <span>{{ libelleStatut(invitation.statut) }}</span>
                  </span>
                </td>
                <td>{{ invitation.date_creation | dateHeure }}</td>
                <td>{{ invitation.expire_le | dateHeure }}</td>
                <td>{{ personne(invitation) }}</td>
                <td>
                  @if (invitation.statut === 'en_attente') {
                    <button
                      type="button"
                      class="bouton bouton--secondaire"
                      [disabled]="envoiEnCours()"
                      [attr.aria-label]="'Annuler l’invitation envoyée au ' + invitation.telephone"
                      (click)="annuler(invitation)"
                    >
                      Annuler
                    </button>
                  }
                </td>
              </tr>
            }
          </tbody>
        </table>
      </div>

      @if (chargement()) {
        <p role="status">Chargement…</p>
      } @else if (invitations().length === 0 && messageListe() === null) {
        <p class="liste-vide" role="status">Aucune invitation pour le moment.</p>
      }

      @if (suivant() !== null && !chargement()) {
        <button type="button" class="bouton bouton--secondaire" (click)="suivante()">
          Afficher les invitations suivantes
        </button>
      }
    </lc-cadre>
  `,
})
export class InvitationsPage implements OnInit {
  private readonly service = inject(InvitationsService);
  private readonly fenetres = inject(FenetreService);

  protected readonly libelles = LIBELLES_ROLE;
  protected readonly formulaire = new FormGroup({
    telephone: new FormControl('', { nonNullable: true }),
    role: new FormControl<RoleMunaseb>('agent_assurance_munaseb', { nonNullable: true }),
  });
  /** Envoi, ou fenêtre de confirmation ouverte : un seul à la fois. */
  protected readonly envoiEnCours = signal(false);
  protected readonly erreurTelephone = signal<string | null>(null);
  protected readonly messageEnvoi = signal<MessageAffiche | null>(null);

  protected readonly invitations = signal<Invitation[]>([]);
  protected readonly suivant = signal<string | null>(null);
  protected readonly chargement = signal(false);
  protected readonly messageListe = signal<MessageAffiche | null>(null);

  ngOnInit(): void {
    void this.charger(null);
  }

  protected libelleRole(role: string): string {
    return estRoleMunaseb(role) ? LIBELLES_ROLE[role] : role;
  }

  protected libelleStatut(statut: StatutInvitation): string {
    return libelleStatutInvitation(statut);
  }

  protected pastille(invitation: Invitation): { classe: string; icone: NomIcone } {
    return PASTILLES[invitation.statut] ?? PASTILLES.expiree;
  }

  /** Nom et prénom : seulement après l'acceptation (§2.10). */
  protected personne(invitation: Invitation): string {
    const nom = [invitation.accepte_par_nom, invitation.accepte_par_prenom]
      .filter((m) => m !== null && m.trim() !== '')
      .join(' ');
    return nom === '' ? '—' : nom;
  }

  async inviter(): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    const { telephone, role } = this.formulaire.getRawValue();
    this.messageEnvoi.set(null);
    this.erreurTelephone.set(telephone.trim() === '' ? MESSAGES_INVITATION.telephoneVide : null);
    if (this.erreurTelephone() !== null) {
      return;
    }
    this.envoiEnCours.set(true);
    try {
      if (role === 'responsable') {
        // Un responsable peut inviter et retirer des accès : confirmation d'abord (porteur, A3).
        const issue = await this.confirmer<InvitationEnvoyee>({
          titre: 'Inviter un responsable',
          texte:
            'Un responsable peut inviter des personnes et retirer des accès dans votre structure. Envoyer cette invitation ?',
          action: 'Inviter comme responsable',
          envoyer: () => this.service.inviter(telephone, role),
        });
        if (issue === undefined) {
          return;
        }
        if (issue.ok) {
          this.apresEnvoi(issue.resultat);
        } else {
          this.erreurEnvoi(issue.erreur);
        }
        return;
      }
      this.apresEnvoi(await this.service.inviter(telephone, role));
    } catch (erreur) {
      this.erreurEnvoi(erreur);
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  async annuler(invitation: Invitation): Promise<void> {
    if (this.envoiEnCours()) {
      return;
    }
    this.envoiEnCours.set(true);
    this.messageListe.set(null);
    try {
      const issue = await this.confirmer<void>({
        titre: 'Annuler l’invitation',
        texte: `L'invitation envoyée au ${invitation.telephone} ne pourra plus être acceptée.`,
        action: 'Annuler l’invitation',
        envoyer: () => this.service.annuler(invitation.id),
      });
      if (issue === undefined) {
        return;
      }
      if (issue.ok) {
        this.messageListe.set({ genre: 'info', texte: MESSAGES_INVITATION.annulee });
        await this.charger(null, false);
        return;
      }
      await this.erreurAnnulation(issue.erreur);
    } finally {
      this.envoiEnCours.set(false);
    }
  }

  actualiser(): void {
    this.messageListe.set(null);
    void this.charger(null);
  }

  /** Page suivante : `avant` = `suivant` de la page précédente (§2.10). */
  suivante(): void {
    void this.charger(this.suivant());
  }

  private async confirmer<R>(donnees: DonneesConfirmation<R>): Promise<IssueFenetre<R> | undefined> {
    const ref = this.fenetres.ouvrir<IssueFenetre<R>, DonneesConfirmation<R>, FenetreConfirmationComponent>(
      FenetreConfirmationComponent,
      donnees,
      'titre-confirmation',
    );
    return firstValueFrom(ref.closed);
  }

  private apresEnvoi(reponse: InvitationEnvoyee): void {
    // Même texte que le numéro ait un compte ou non (§2.10) ; date du jour du Burkina, comme le SMS.
    this.messageEnvoi.set({
      genre: 'info',
      texte: `${MESSAGES_INVITATION.envoyee} Valable jusqu'au ${formaterDateBurkina(reponse.expire_le)}.`,
    });
    this.formulaire.reset();
    void this.charger(null);
  }

  private erreurEnvoi(erreur: unknown): void {
    const code = codeErreur(erreur);
    // jeton_invalide et acces_refuse : l'intercepteur a déjà changé d'écran.
    if (code === 'jeton_invalide' || code === 'acces_refuse') {
      return;
    }
    if (code === 'telephone_invalide') {
      this.erreurTelephone.set(MESSAGES_INVITATION.telephoneInvalide);
      return;
    }
    const texte = code === null ? undefined : PAR_CODE_ENVOI[code];
    this.messageEnvoi.set({ genre: 'erreur', texte: texte ?? messagePourErreur(decoderErreur(erreur)) });
  }

  private async erreurAnnulation(erreur: unknown): Promise<void> {
    const decodee = decoderErreur(erreur);
    if (decodee.genre === 'api' && (decodee.code === 'jeton_invalide' || decodee.code === 'acces_refuse')) {
      return;
    }
    let texte = messagePourErreur(decodee);
    let recharger = false;
    if (decodee.genre === 'api') {
      if (decodee.code === 'invitation_inconnue') {
        [texte, recharger] = [MESSAGES_INVITATION.inconnue, true];
      } else if (decodee.code === 'invitation_expiree') {
        [texte, recharger] = [MESSAGES_INVITATION.expiree, true];
      } else if (decodee.code === 'transition_invalide') {
        const actuel = decodee.corps.statut_actuel;
        texte = `Cette invitation a changé entre-temps. Elle est maintenant : ${
          actuel === undefined ? 'inconnu' : libelleStatutInvitation(actuel).toLowerCase()
        }.`;
        recharger = true;
      }
    }
    if (recharger) {
      await this.charger(null, false);
    }
    this.messageListe.set({ genre: 'erreur', texte });
  }

  private async charger(avant: string | null, effacerMessage = true): Promise<void> {
    if (avant === null) {
      this.invitations.set([]);
      this.suivant.set(null);
    }
    this.chargement.set(true);
    if (effacerMessage) {
      this.messageListe.set(null);
    }
    try {
      const page = await this.service.lister(avant);
      this.invitations.update((deja) => [...deja, ...page.invitations]);
      this.suivant.set(page.suivant);
    } catch (erreur) {
      const code = codeErreur(erreur);
      if (code !== 'jeton_invalide' && code !== 'acces_refuse') {
        this.messageListe.set({ genre: 'erreur', texte: messagePourErreur(decoderErreur(erreur)) });
      }
    } finally {
      this.chargement.set(false);
    }
  }
}
