//! Réinitialisation du mot de passe d'un compte professionnel ou
//! administrateur par un lien envoyé par email (étape 3.12, décisions R1 à
//! R21 du porteur, 2026-10-08 et 2026-10-21). Les patients gardent la
//! réinitialisation par SMS (`auth_patient.rs`, R20).
//!
//! Sources : OWASP *Forgot Password Cheat Sheet* ; NIST SP 800-63B rév. 4,
//! §4.2 « Issued Recovery Codes » et §4.6 « Account Notifications ».
//!
//! **Demande** (`demander`) : la route répond 204 tout de suite ; tout le
//! travail (recherche du compte, limites, lien, email) se fait dans une
//! tâche de fond (R11). OWASP : « Ensure that responses return in a
//! consistent amount of time […] This could be achieved by using
//! asynchronous calls ». Un lien ne part que vers un compte actif dont
//! l'email est vérifié (R12 ; NIST §4.2 : une adresse de récupération « SHALL
//! be verified »).
//!
//! **Réinitialisation** (`reinitialiser`) : lien à usage unique, 30 minutes,
//! lié à l'adresse visée ; nouveau mot de passe, verrouillage levé,
//! `version_jeton` augmentée, toutes les sessions fermées, autres liens
//! annulés, TOTP jamais touché, aucune connexion automatique (R3, R7, R15) ;
//! puis alerte « mot de passe modifié » (R17).
//!
//! Ordre des verrous : le compte, puis ses jetons d'email, puis ses sessions
//! (le compte avant le reste, CLAUDE.md §12). Le jeton présenté est d'abord
//! lu sans verrou, pour connaître son compte.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::alerte_securite::{self, Evenement};
use crate::auth_professionnel::nature_sqlx;
use crate::email::EmailSender;
use crate::jeton_email::{self, VerificationEmise};
use crate::mot_de_passe::{self, MotDePasseNonConforme};
use crate::session::{self, MotifRevocation};

/// Tâches de fond de demande simultanées, au plus (R11 ; 10, décision du
/// porteur du 2026-10-21 : les tâches sont légères, mais un vrai fournisseur
/// d'email peut être lent). Une tâche ne prend une connexion à la base que
/// le temps de ses deux courtes requêtes, et la rend avant l'envoi de
/// l'email. Au-delà, la demande est ignorée, avec la même réponse 204.
pub const TACHES_DE_FOND_MAX: usize = 10;

/// Délai maximal de l'envoi de l'email dans la tâche de fond (valeur
/// proposée le 2026-10-21, sans source qui en donne une) : 30 secondes, bien
/// plus que le temps normal d'un envoi, et petit devant la validité du lien
/// (30 minutes). Au-delà, l'envoi est tenu pour échoué, c'est journalisé
/// sans l'adresse, et la tâche se termine : elle ne garde jamais son permis.
pub const DELAI_ENVOI_EMAIL: Duration = Duration::from_secs(30);

/// Compteur borné des tâches de fond, partagé par le serveur : un permis se
/// prend sans attendre, et se rend à la fin de la tâche (`Drop`). Compteur
/// atomique de la bibliothèque standard : le sémaphore de tokio exige la
/// fonctionnalité `sync`, non activée dans le Cargo.toml.
#[derive(Debug, Default)]
pub struct LimiteTaches {
    en_cours: AtomicUsize,
}

/// Permis d'une tâche ; rendu à sa libération.
pub struct Permis(Arc<LimiteTaches>);

impl Drop for Permis {
    fn drop(&mut self) {
        self.0.en_cours.fetch_sub(1, Ordering::AcqRel);
    }
}

impl LimiteTaches {
    /// Un permis si moins de `TACHES_DE_FOND_MAX` tâches tournent ; sinon
    /// `None`. `fetch_update` (doc de `AtomicUsize`) : le test et
    /// l'incrément se font en une seule opération, même avec des demandes
    /// simultanées.
    pub fn prendre(limite: &Arc<Self>) -> Option<Permis> {
        limite
            .en_cours
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < TACHES_DE_FOND_MAX).then_some(n + 1))
            .ok()
            .map(|_| Permis(Arc::clone(limite)))
    }
}

/// Sorte de compte visée par la demande (R10) : une même personne peut avoir
/// un compte professionnel et un compte administrateur avec le même email.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SorteCompte {
    Professionnel,
    Administrateur,
}

impl SorteCompte {
    fn type_compte(self) -> &'static str {
        match self {
            SorteCompte::Professionnel => "professionnel",
            SorteCompte::Administrateur => "administrateur_laaficare",
        }
    }
}

// ---------------------------------------------------------------------
// Demande (R11 à R14, R18)
// ---------------------------------------------------------------------

/// Lance la demande en tâche de fond et rend la main aussitôt (R11). Plus de
/// place : demande ignorée et journalisée, sans l'email.
pub fn demander(
    pool: PgPool,
    email_sender: Arc<dyn EmailSender>,
    url_application_web: Arc<str>,
    limite: &Arc<LimiteTaches>,
    sorte: SorteCompte,
    email: String,
) {
    let Some(permis) = LimiteTaches::prendre(limite) else {
        tracing::warn!(?sorte, "mot de passe oublié : trop de demandes en cours, demande ignorée");
        return;
    };
    tokio::spawn(async move {
        // Le permis est rendu à la fin de cette tâche, quoi qu'il arrive :
        // erreur, envoi échoué ou délai dépassé.
        let _permis = permis;
        if let Err(e) = traiter_demande(&pool, email_sender, &url_application_web, sorte, &email, DELAI_ENVOI_EMAIL).await {
            tracing::error!(?sorte, nature = %nature_sqlx(&e), "mot de passe oublié : erreur dans la tâche de fond");
        }
    });
}

/// Le travail de la demande. Rien n'est envoyé pour un email inconnu, non
/// vérifié ou un compte désactivé (R12) ; un compte verrouillé reçoit le
/// lien (OWASP : « Accounts should not be locked out in response to a
/// forgotten password attack »). L'email part après la transaction, avec
/// le délai `delai`.
pub async fn traiter_demande(
    pool: &PgPool,
    email_sender: Arc<dyn EmailSender>,
    url_application_web: &str,
    sorte: SorteCompte,
    email: &str,
    delai: Duration,
) -> Result<(), sqlx::Error> {
    // Même recherche que la connexion (J6).
    let compte: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.id FROM compte c JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE lower(u.email) = lower(btrim($1)) AND c.type_compte = $2 AND c.statut = 'actif' \
           AND u.email_verifie_le IS NOT NULL",
    )
    .bind(email)
    .bind(sorte.type_compte())
    .fetch_optional(pool)
    .await?;
    let Some(compte_id) = compte else {
        return Ok(());
    };

    let mut tx = pool.begin().await?;
    let emise = jeton_email::emettre_reinitialisation(&mut tx, compte_id).await?;
    tx.commit().await?;
    if let Some(emise) = emise {
        let (destinataire, corps) = message_lien(url_application_web, &emise);
        // L'attente se fait hors des fils de tokio (`spawn_blocking`) : elle
        // ne bloque jamais le serveur.
        let resultat = tokio::task::spawn_blocking(move || {
            envoyer_avec_delai(email_sender, destinataire, SUJET_LIEN, corps, delai)
        })
        .await;
        match resultat {
            Ok(Envoi::Envoye) => {}
            Ok(Envoi::Echec) => tracing::error!("mot de passe oublié : échec d'envoi de l'email"),
            Ok(Envoi::DelaiDepasse) => tracing::error!("mot de passe oublié : délai d'envoi de l'email dépassé"),
            Err(_) => tracing::error!("mot de passe oublié : envoi de l'email interrompu"),
        }
    }
    Ok(())
}

const SUJET_LIEN: &str = "LaafiCare : réinitialisation de votre mot de passe";

/// Destinataire et corps de l'email du lien (R18). Jeton après `#` (R6 ;
/// RFC 3986 §3.5, le fragment n'est jamais envoyé au serveur).
fn message_lien(url_application_web: &str, emise: &VerificationEmise) -> (String, String) {
    let corps = format!(
        "Bonjour,\n\n\
         Pour choisir un nouveau mot de passe, ouvrez ce lien puis suivez les indications :\n\n\
         {url_application_web}/reinitialiser-mot-de-passe#jeton={}\n\n\
         Ce lien est valable {} minutes et ne sert qu'une fois. Si vous n'avez rien demandé, ignorez ce \
         message : votre mot de passe actuel reste valable.",
        emise.jeton.en_clair(),
        jeton_email::DUREE_REINITIALISATION_MINUTES,
    );
    (emise.email.clone(), corps)
}

/// Issue d'un envoi borné dans le temps.
#[derive(Debug, PartialEq, Eq)]
enum Envoi {
    Envoye,
    Echec,
    DelaiDepasse,
}

/// Envoi borné dans le temps, avec la bibliothèque standard seule
/// (`tokio::time` exige la fonctionnalité `time`, non activée) : l'envoi,
/// synchrone (`EmailSender::envoyer`), tourne dans son propre fil, et
/// l'appelant attend son résultat au plus `delai` (`Receiver::recv_timeout`,
/// doc de `std::sync::mpsc`). Au-delà, l'appelant repart ; le fil bloqué
/// finit seul, sans plus rien retenir (son résultat est ignoré).
fn envoyer_avec_delai(
    email_sender: Arc<dyn EmailSender>,
    destinataire: String,
    sujet: &'static str,
    corps: String,
    delai: Duration,
) -> Envoi {
    let (emetteur, recepteur) = mpsc::channel();
    std::thread::spawn(move || {
        let reussi = email_sender.envoyer(&destinataire, sujet, &corps).is_ok();
        // L'appelant est peut-être déjà reparti : l'erreur est sans objet.
        let _ = emetteur.send(reussi);
    });
    match recepteur.recv_timeout(delai) {
        Ok(true) => Envoi::Envoye,
        Ok(false) => Envoi::Echec,
        Err(_) => Envoi::DelaiDepasse,
    }
}

// ---------------------------------------------------------------------
// Réinitialisation (R14 à R17)
// ---------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ErreurReinitialisation {
    /// Lien inconnu, déjà utilisé, remplacé, expiré, ou email changé depuis
    /// l'envoi : jamais distingués (R14, même règle que E6).
    #[error("Ce lien n'est plus valable. Demandez-en un nouveau.")]
    LienInvalide,
    #[error("Mot de passe non conforme.")]
    MotDePasseNonConforme(MotDePasseNonConforme),
    /// R16 : administrateurs seulement (décision du porteur, 2026-10-18).
    #[error("Le mot de passe administrateur doit être différent de celui du compte patient.")]
    MotDePasseIdentiquePatient,
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

fn interne(etape: &'static str, erreur: &sqlx::Error) -> ErreurReinitialisation {
    tracing::error!(etape, nature = %nature_sqlx(erreur), "réinitialisation du mot de passe : erreur interne");
    ErreurReinitialisation::Interne(etape)
}

/// `POST /api/mot-de-passe/reinitialiser` (R10) : le jeton désigne son
/// compte, professionnel ou administrateur.
pub async fn reinitialiser(
    pool: &PgPool,
    email_sender: &dyn EmailSender,
    contact_securite: &str,
    jeton: &str,
    nouveau_mot_de_passe: &str,
) -> Result<(), ErreurReinitialisation> {
    let Some(octets) = session::decoder(jeton.trim()) else {
        return Err(ErreurReinitialisation::LienInvalide);
    };
    let empreinte = session::empreinte(&octets);

    // Lecture sans verrou, pour connaître le compte (verrouillé ensuite en
    // premier). Lien lié à l'adresse visée (majuscules près, J6).
    let ligne: Option<(Uuid, Uuid, String, Uuid)> = sqlx::query_as(
        "SELECT j.id, c.id, c.type_compte, c.utilisateur_id FROM jeton_email j \
         JOIN compte c ON c.id = j.compte_id JOIN utilisateur u ON u.id = c.utilisateur_id \
         WHERE j.empreinte = $1 AND j.objet = 'reinitialisation_mot_de_passe' \
           AND j.utilise_le IS NULL AND j.annule_le IS NULL AND j.expire_le > now() \
           AND lower(u.email) = lower(j.email_vise) AND u.email_verifie_le IS NOT NULL AND c.statut = 'actif'",
    )
    .bind(&empreinte)
    .fetch_optional(pool)
    .await
    .map_err(|e| interne("recherche du lien", &e))?;
    let Some((jeton_id, compte_id, type_compte, utilisateur_id)) = ligne else {
        return Err(ErreurReinitialisation::LienInvalide);
    };

    // Règles du mot de passe avant toute écriture : un refus laisse le lien
    // valable, la personne corrige et renvoie.
    mot_de_passe::controler(nouveau_mot_de_passe).map_err(ErreurReinitialisation::MotDePasseNonConforme)?;
    if type_compte == "administrateur_laaficare" {
        let hash_patient: Option<String> =
            sqlx::query_scalar("SELECT mot_de_passe_hash FROM compte WHERE utilisateur_id = $1 AND type_compte = 'patient'")
                .bind(utilisateur_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| interne("lecture du compte patient", &e))?;
        if let Some(hash_patient) = hash_patient {
            let identique = mot_de_passe::verifier_async(nouveau_mot_de_passe, &hash_patient)
                .await
                .map_err(|_| ErreurReinitialisation::Interne("comparaison avec le mot de passe patient"))?;
            if identique {
                return Err(ErreurReinitialisation::MotDePasseIdentiquePatient);
            }
        }
    }
    // Haché avant la transaction : Argon2id ne garde pas de verrous.
    // Version asynchrone pour ne pas bloquer Tokio.
    let hash = mot_de_passe::hacher_async(nouveau_mot_de_passe)
        .await
        .map_err(|_| ErreurReinitialisation::Interne("hachage"))?;

    let mut tx = pool.begin().await.map_err(|e| interne("transaction", &e))?;
    sqlx::query("SELECT id FROM compte WHERE id = $1 FOR UPDATE")
        .bind(compte_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| interne("verrou du compte", &e))?;
    // Usage unique (R3) : sous le verrou du compte, deux utilisations du
    // même lien se suivent, la seconde ne trouve plus de lien en attente.
    let consomme: Option<Uuid> = sqlx::query_scalar(
        "UPDATE jeton_email SET utilise_le = now() \
         WHERE id = $1 AND utilise_le IS NULL AND annule_le IS NULL AND expire_le > now() RETURNING id",
    )
    .bind(jeton_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| interne("utilisation du lien", &e))?;
    if consomme.is_none() {
        return Err(ErreurReinitialisation::LienInvalide);
    }
    // R3 : une réinitialisation réussie annule tous les autres liens en
    // attente du compte (OWASP : « invalidate […] outstanding password reset
    // and recovery links »).
    sqlx::query(
        "UPDATE jeton_email SET annule_le = now() \
         WHERE compte_id = $1 AND objet = 'reinitialisation_mot_de_passe' AND utilise_le IS NULL AND annule_le IS NULL",
    )
    .bind(compte_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("annulation des autres liens", &e))?;
    // R7, R15 : verrouillage levé, `version_jeton` augmentée, sessions
    // fermées (OWASP : « After successful recovery, invalidate existing
    // sessions ») ; TOTP et codes de secours jamais touchés. Même requête que
    // la réinitialisation patient.
    sqlx::query(
        "UPDATE compte SET mot_de_passe_hash = $1, tentatives_echouees = 0, verrouille_jusqua = NULL, \
         version_jeton = version_jeton + 1 WHERE id = $2",
    )
    .bind(&hash)
    .bind(compte_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| interne("nouveau mot de passe", &e))?;
    session::fermer_toutes(&mut tx, compte_id, MotifRevocation::ReinitialisationMotDePasse, false)
        .await
        .map_err(|e| interne("fermeture des sessions", &e))?;
    tx.commit().await.map_err(|e| interne("validation de la transaction", &e))?;

    // R17, après la validation en base ; un échec n'annule rien. Aucune
    // session n'est ouverte : la personne se reconnecte (OWASP : « Don't
    // automatically log the user in »).
    alerte_securite::prevenir(pool, email_sender, contact_securite, compte_id, Evenement::MotDePasseReinitialise).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_professionnel::{self, ConnexionProfessionnelle, ErreurAuthProfessionnel};
    use crate::email::EmailsRecus;
    use crate::jwt::{JwtService, SorteJeton};

    // Base temporaire par test ; données fictives.

    const ANCIEN: &str = "Ancien#2026";
    const NOUVEAU: &str = "Nouveau#2026";
    const CONTACT: &str = "securite@example.org";
    const URL: &str = "http://localhost:4200";

    /// Identité, compte patient et compte du type demandé ; email vérifié si
    /// demandé. Renvoie le compte (professionnel ou administrateur).
    async fn compte(pool: &PgPool, telephone: &str, email: &str, type_compte: &str, verifie: bool) -> Uuid {
        let u: Uuid = sqlx::query_scalar("INSERT INTO utilisateur (nom, prenom, telephone, email) VALUES ('KABRE', 'Ali', $1, $2) RETURNING id")
            .bind(telephone)
            .bind(email)
            .fetch_one(pool)
            .await
            .unwrap();
        if verifie {
            sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1").bind(u).execute(pool).await.unwrap();
        }
        sqlx::query("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, 'patient', $2)")
            .bind(u)
            .bind(mot_de_passe::hacher("Patient#2026").unwrap())
            .execute(pool)
            .await
            .unwrap();
        sqlx::query_scalar("INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, $3) RETURNING id")
            .bind(u)
            .bind(type_compte)
            .bind(mot_de_passe::hacher(ANCIEN).unwrap())
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Demande traitée directement (sans tâche de fond), emails gardés.
    async fn demande(pool: &PgPool, emails: &Arc<EmailsRecus>, sorte: SorteCompte, email: &str) -> Result<(), sqlx::Error> {
        traiter_demande(pool, emails.clone(), URL, sorte, email, DELAI_ENVOI_EMAIL).await
    }

    fn jeton_du_lien(corps: &str) -> String {
        let lien = corps.lines().find(|l| l.contains("/reinitialiser-mot-de-passe#jeton=")).expect("lien attendu");
        assert!(!lien.contains('?'), "{lien}");
        lien.split("#jeton=").nth(1).unwrap().trim().to_string()
    }

    async fn compter(pool: &PgPool, requete: &'static str, compte_id: Uuid) -> i64 {
        sqlx::query_scalar(requete).bind(compte_id).fetch_one(pool).await.unwrap()
    }

    /// Reculer la date des liens du compte (pour passer le délai d'une
    /// minute, R13).
    async fn vieillir_liens(pool: &PgPool, compte_id: Uuid) {
        sqlx::query("UPDATE jeton_email SET date_creation = date_creation - interval '2 minutes' WHERE compte_id = $1")
            .bind(compte_id)
            .execute(pool)
            .await
            .unwrap();
    }

    /// R12 : rien n'est envoyé pour un email inconnu, non vérifié, ou un
    /// compte désactivé ; un compte verrouillé reçoit le lien.
    #[sqlx::test]
    #[ignore]
    async fn qui_recoit_un_lien(pool: PgPool) {
        let emails = Arc::new(EmailsRecus::default());
        compte(&pool, "+22670300001", "non.verifie@example.org", "professionnel", false).await;
        let desactive = compte(&pool, "+22670300002", "desactive@example.org", "professionnel", true).await;
        sqlx::query("UPDATE compte SET statut = 'desactive' WHERE id = $1").bind(desactive).execute(&pool).await.unwrap();
        let verrouille = compte(&pool, "+22670300003", "verrouille@example.org", "professionnel", true).await;
        sqlx::query("UPDATE compte SET verrouille_jusqua = now() + interval '15 minutes' WHERE id = $1")
            .bind(verrouille)
            .execute(&pool)
            .await
            .unwrap();

        for email in ["inconnu@example.org", "non.verifie@example.org", "desactive@example.org"] {
            demande(&pool, &emails,SorteCompte::Professionnel, email).await.unwrap();
        }
        assert!(emails.0.lock().unwrap().is_empty());

        // Verrouillé : lien envoyé ; majuscules et espaces autour ignorés (J6).
        demande(&pool, &emails,SorteCompte::Professionnel, " Verrouille@Example.org ").await.unwrap();
        let envoyes = emails.0.lock().unwrap().clone();
        assert_eq!(envoyes.len(), 1);
        assert_eq!(envoyes[0].0, "verrouille@example.org");
        assert!(envoyes[0].1.contains("valable 30 minutes"));
        // Pas de tiret cadratin (décision du porteur, 2026-10-21).
        assert!(!envoyes[0].1.contains('\u{2014}'));
        // Mauvaise sorte de compte : rien.
        demande(&pool, &emails,SorteCompte::Administrateur, "verrouille@example.org").await.unwrap();
        assert_eq!(emails.0.lock().unwrap().len(), 1);
    }

    /// R5 et R13 : une minute entre deux liens ; 3 liens valables au plus.
    #[sqlx::test]
    #[ignore]
    async fn limites_d_envoi(pool: PgPool) {
        let emails = Arc::new(EmailsRecus::default());
        let compte_id = compte(&pool, "+22670300011", "limites@example.org", "professionnel", true).await;
        demande(&pool, &emails,SorteCompte::Professionnel, "limites@example.org").await.unwrap();
        demande(&pool, &emails,SorteCompte::Professionnel, "limites@example.org").await.unwrap();
        assert_eq!(emails.0.lock().unwrap().len(), 1, "moins d'une minute : rien");
        for _ in 0..3 {
            vieillir_liens(&pool, compte_id).await;
            demande(&pool, &emails,SorteCompte::Professionnel, "limites@example.org").await.unwrap();
        }
        assert_eq!(emails.0.lock().unwrap().len(), 3, "3 liens valables au plus");
        // Les liens précédents restent valables (R3) : 3 en attente.
        assert_eq!(
            compter(&pool, "SELECT count(*) FROM jeton_email WHERE compte_id = $1 AND utilise_le IS NULL AND annule_le IS NULL", compte_id).await,
            3
        );
    }

    /// Parcours complet : lien, réinitialisation ; sessions fermées et ancien
    /// jeton d'accès refusé ; verrouillage levé ; TOTP intact ; autres liens
    /// annulés ; ancien mot de passe refusé, nouveau accepté ; alerte avec
    /// la date et le contact ; usage unique.
    #[sqlx::test]
    #[ignore]
    async fn reinitialisation_complete(pool: PgPool) {
        let emails = Arc::new(EmailsRecus::default());
        let jwt = JwtService::new(&"a".repeat(32));
        let compte_id = compte(&pool, "+22670300021", "complet@example.org", "professionnel", true).await;
        let session_avant = session::ouvrir(&pool, compte_id, None).await.unwrap().en_paire(&jwt).unwrap();
        sqlx::query(
            "INSERT INTO second_facteur_totp (compte_id, secret_chiffre, nonce, version_cle, statut, date_activation) \
             VALUES ($1, decode(repeat('00', 36), 'hex'), decode(repeat('00', 12), 'hex'), 1, 'actif', now())",
        )
        .bind(compte_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE compte SET tentatives_echouees = 3 WHERE id = $1").bind(compte_id).execute(&pool).await.unwrap();

        demande(&pool, &emails,SorteCompte::Professionnel, "complet@example.org").await.unwrap();
        vieillir_liens(&pool, compte_id).await;
        demande(&pool, &emails,SorteCompte::Professionnel, "complet@example.org").await.unwrap();
        let (premier, second) = {
            let e = emails.0.lock().unwrap();
            (jeton_du_lien(&e[0].1), jeton_du_lien(&e[1].1))
        };

        // Mot de passe non conforme : refusé, le lien reste valable.
        assert!(matches!(
            reinitialiser(&pool, &*emails,CONTACT, &premier, "court").await,
            Err(ErreurReinitialisation::MotDePasseNonConforme(_))
        ));
        reinitialiser(&pool, &*emails,CONTACT, &premier, NOUVEAU).await.unwrap();

        // Usage unique, et l'autre lien est annulé.
        for jeton in [&premier, &second] {
            assert!(matches!(reinitialiser(&pool, &*emails,CONTACT, jeton, NOUVEAU).await, Err(ErreurReinitialisation::LienInvalide)));
        }
        // Sessions fermées, ancienne version : l'ancien jeton n'est plus bon.
        assert_eq!(
            compter(&pool, "SELECT count(*) FROM session WHERE compte_id = $1 AND motif_revocation = 'reinitialisation_mot_de_passe'", compte_id).await,
            1
        );
        let claims = jwt.verifier(&session_avant.jeton_acces, SorteJeton::ProfessionnelSansAffectation).unwrap();
        assert_eq!(compter(&pool, "SELECT version_jeton::bigint FROM compte WHERE id = $1", compte_id).await, i64::from(claims.ver) + 1);
        // Verrouillage levé, TOTP intact.
        assert_eq!(compter(&pool, "SELECT tentatives_echouees::bigint FROM compte WHERE id = $1", compte_id).await, 0);
        assert_eq!(compter(&pool, "SELECT count(*) FROM second_facteur_totp WHERE compte_id = $1 AND statut = 'actif'", compte_id).await, 1);
        // Ancien mot de passe refusé ; nouveau accepté (le TOTP reste exigé).
        assert!(matches!(
            auth_professionnel::se_connecter(&pool, &jwt, "complet@example.org", ANCIEN).await,
            Err(ErreurAuthProfessionnel::IdentifiantsInvalides)
        ));
        assert!(matches!(
            auth_professionnel::se_connecter(&pool, &jwt, "complet@example.org", NOUVEAU).await,
            Ok(ConnexionProfessionnelle::SecondFacteur { .. })
        ));

        // Alerte : la dernière, avec la date d'utilisation du lien et le contact.
        let alerte = emails.0.lock().unwrap().last().unwrap().1.clone();
        let quand: String = sqlx::query_scalar(
            "SELECT to_char(max(utilise_le) AT TIME ZONE 'Africa/Ouagadougou', 'DD/MM/YYYY \"à\" HH24:MI') \
             FROM jeton_email WHERE compte_id = $1",
        )
        .bind(compte_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(alerte.contains("compte professionnel LaafiCare"), "{alerte}");
        assert!(alerte.contains(&format!("le {quand} (heure du Burkina Faso)")) && alerte.contains(CONTACT), "{alerte}");
        assert!(!alerte.contains("http"));
    }

    /// R14 : lien expiré, mal formé, ou email changé depuis l'envoi.
    #[sqlx::test]
    #[ignore]
    async fn liens_invalides(pool: PgPool) {
        let emails = Arc::new(EmailsRecus::default());
        let compte_id = compte(&pool, "+22670300031", "lien@example.org", "professionnel", true).await;
        demande(&pool, &emails,SorteCompte::Professionnel, "lien@example.org").await.unwrap();
        let jeton = jeton_du_lien(&emails.0.lock().unwrap()[0].1);

        for faux in ["", "abc", &"0".repeat(64)] {
            assert!(matches!(reinitialiser(&pool, &*emails,CONTACT, faux, NOUVEAU).await, Err(ErreurReinitialisation::LienInvalide)));
        }
        sqlx::query("UPDATE utilisateur SET email = 'autre@example.org' FROM compte c WHERE c.id = $1 AND utilisateur.id = c.utilisateur_id")
            .bind(compte_id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(reinitialiser(&pool, &*emails,CONTACT, &jeton, NOUVEAU).await, Err(ErreurReinitialisation::LienInvalide)));

        // Expiré (30 minutes).
        let autre = compte(&pool, "+22670300032", "expire@example.org", "professionnel", true).await;
        demande(&pool, &emails,SorteCompte::Professionnel, "expire@example.org").await.unwrap();
        let jeton = jeton_du_lien(&emails.0.lock().unwrap().last().unwrap().1);
        sqlx::query("UPDATE jeton_email SET date_creation = now() - interval '31 minutes', expire_le = now() - interval '1 minute' WHERE compte_id = $1")
            .bind(autre)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(reinitialiser(&pool, &*emails,CONTACT, &jeton, NOUVEAU).await, Err(ErreurReinitialisation::LienInvalide)));
    }

    /// R16 : un administrateur ne reprend pas le mot de passe de son compte
    /// patient ; un professionnel le peut.
    #[sqlx::test]
    #[ignore]
    async fn administrateur_et_mot_de_passe_patient(pool: PgPool) {
        let emails = Arc::new(EmailsRecus::default());
        compte(&pool, "+22670300041", "adm.reinit@example.org", "administrateur_laaficare", true).await;
        demande(&pool, &emails,SorteCompte::Administrateur, "adm.reinit@example.org").await.unwrap();
        let jeton = jeton_du_lien(&emails.0.lock().unwrap()[0].1);
        assert!(matches!(
            reinitialiser(&pool, &*emails,CONTACT, &jeton, "Patient#2026").await,
            Err(ErreurReinitialisation::MotDePasseIdentiquePatient)
        ));
        reinitialiser(&pool, &*emails,CONTACT, &jeton, NOUVEAU).await.unwrap();
        assert!(emails.0.lock().unwrap().last().unwrap().1.contains("compte administrateur LaafiCare"));

        compte(&pool, "+22670300042", "pro.reinit@example.org", "professionnel", true).await;
        demande(&pool, &emails,SorteCompte::Professionnel, "pro.reinit@example.org").await.unwrap();
        let jeton = jeton_du_lien(&emails.0.lock().unwrap().last().unwrap().1);
        reinitialiser(&pool, &*emails,CONTACT, &jeton, "Patient#2026").await.unwrap();
    }

    /// Une réinitialisation et une connexion simultanées : aucune erreur
    /// interne, aucun interblocage.
    #[sqlx::test]
    #[ignore]
    async fn reinitialisation_et_connexion_simultanees(pool: PgPool) {
        let emails = Arc::new(EmailsRecus::default());
        compte(&pool, "+22670300051", "simultane@example.org", "professionnel", true).await;
        demande(&pool, &emails,SorteCompte::Professionnel, "simultane@example.org").await.unwrap();
        let jeton = jeton_du_lien(&emails.0.lock().unwrap()[0].1);
        let (p1, p2) = (pool.clone(), pool.clone());
        let reinit = tokio::spawn(async move { reinitialiser(&p1, &EmailsRecus::default(), CONTACT, &jeton, NOUVEAU).await.is_ok() });
        let connexion = tokio::spawn(async move {
            let jwt = JwtService::new(&"a".repeat(32));
            auth_professionnel::se_connecter(&p2, &jwt, "simultane@example.org", ANCIEN).await
        });
        assert!(reinit.await.unwrap());
        assert!(!matches!(connexion.await.unwrap(), Err(ErreurAuthProfessionnel::Interne(_))));
    }

    /// Fournisseur d'email bloqué : ne répond qu'au bout de `attente`.
    struct EmailLent(Duration);

    impl EmailSender for EmailLent {
        fn envoyer(&self, _: &str, _: &str, _: &str) -> Result<(), crate::email::ErreurEmail> {
            std::thread::sleep(self.0);
            Ok(())
        }
        fn simule(&self) -> bool {
            true
        }
    }

    /// Délai d'envoi : un envoi plus lent que le délai est tenu pour échoué,
    /// sans attendre sa fin ; un envoi rapide réussit.
    #[test]
    fn envoi_borne_dans_le_temps() {
        let debut = std::time::Instant::now();
        let issue = envoyer_avec_delai(
            Arc::new(EmailLent(Duration::from_secs(5))),
            "lent@example.org".into(),
            SUJET_LIEN,
            "corps".into(),
            Duration::from_millis(100),
        );
        assert_eq!(issue, Envoi::DelaiDepasse);
        assert!(debut.elapsed() < Duration::from_secs(2), "l'appelant n'attend pas la fin de l'envoi");
        let issue = envoyer_avec_delai(Arc::new(EmailsRecus::default()), "a@example.org".into(), SUJET_LIEN, "c".into(), Duration::from_secs(5));
        assert_eq!(issue, Envoi::Envoye);
    }

    /// Une tâche de fond dont l'envoi dépasse le délai rend son permis :
    /// toutes les places sont libres ensuite, et le lien est bien enregistré.
    #[sqlx::test]
    #[ignore]
    async fn tache_bloquee_rend_son_permis(pool: PgPool) {
        let compte_id = compte(&pool, "+22670300061", "lent@example.org", "professionnel", true).await;
        let limite = Arc::new(LimiteTaches::default());
        let permis = LimiteTaches::prendre(&limite).unwrap();
        let tache = tokio::spawn({
            let pool = pool.clone();
            async move {
                let _permis = permis;
                traiter_demande(
                    &pool,
                    Arc::new(EmailLent(Duration::from_secs(5))),
                    URL,
                    SorteCompte::Professionnel,
                    "lent@example.org",
                    Duration::from_millis(200),
                )
                .await
            }
        });
        let debut = std::time::Instant::now();
        tache.await.unwrap().unwrap();
        assert!(debut.elapsed() < Duration::from_secs(4), "la tâche n'attend pas l'envoi bloqué");
        let permis: Vec<_> = (0..TACHES_DE_FOND_MAX).map(|_| LimiteTaches::prendre(&limite).unwrap()).collect();
        assert_eq!(permis.len(), TACHES_DE_FOND_MAX, "le permis de la tâche est rendu");
        assert_eq!(compter(&pool, "SELECT count(*) FROM jeton_email WHERE compte_id = $1", compte_id).await, 1);
    }

    /// R11 : au plus `TACHES_DE_FOND_MAX` permis ; un permis rendu libère une
    /// place.
    #[test]
    fn limite_des_taches_de_fond() {
        let limite = Arc::new(LimiteTaches::default());
        let permis: Vec<_> = (0..TACHES_DE_FOND_MAX).map(|_| LimiteTaches::prendre(&limite).unwrap()).collect();
        assert!(LimiteTaches::prendre(&limite).is_none());
        drop(permis);
        assert!(LimiteTaches::prendre(&limite).is_some());
    }
}
