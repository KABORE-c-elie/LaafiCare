//! Connexion d'un administrateur LaafiCare et réenrôlement de son TOTP
//! (étape 3.6.4, décisions A1 à A7 du 2026-10-15).
//!
//! Le TOTP est obligatoire pour un administrateur (Y4) : la connexion ne
//! donne jamais de session sans code. Mot de passe correct et email
//! vérifié (logique commune, `auth_professionnel::verifier_identifiants`) :
//! - TOTP actif : jeton intermédiaire (5 min), échangé contre la paire de
//!   jetons par `auth_professionnel::terminer_connexion` (A5) ;
//! - pas de TOTP actif, fenêtre de réenrôlement en cours (migration 0019) :
//!   jeton de réenrôlement (10 min), qui ne sert qu'à enregistrer un
//!   nouveau TOTP ;
//! - pas de TOTP actif, pas de fenêtre : `IdentifiantsInvalides`, la même
//!   réponse qu'un mauvais mot de passe. Sinon un mot de passe volé
//!   suffirait à enregistrer son propre TOTP et à prendre le compte.
//!
//! La fenêtre n'est ouverte que par un outil local de l'équipe (création du
//! premier administrateur, désactivation d'un TOTP, étape 3.6.5), jamais
//! par une route. Après la confirmation, aucune session n'est ouverte (A3) :
//! la personne se reconnecte avec son nouveau TOTP, par la seule voie qui
//! mène à une session administrateur (mot de passe puis code).

use sqlx::PgPool;
use sqlx::types::Uuid;

use crate::auth_professionnel::{self, ErreurAuthProfessionnel, JetonIntermediaire};
use crate::config::CleTotp;
use crate::jwt::{JwtService, SorteJeton};
use crate::totp::{self, ActivationEnCours, Autorisation, ErreurTotp};

/// Durée de la fenêtre de réenrôlement ouverte par l'outil local. Source :
/// NIST SP 800-63B rév. 4, §4.1.3, un code d'association d'un nouvel
/// authentificateur « SHALL be valid for a maximum of 10 minutes ».
/// Constante du code, jamais un paramètre de l'outil (décision du
/// 2026-10-15) : l'outil est lancé avec l'administrateur présent ou au
/// téléphone, et relancé si la fenêtre expire.
pub const FENETRE_REENROLEMENT_MINUTES: i32 = 10;

/// Résultat d'un mot de passe correct. Jamais de session directe (Y4).
/// `Debug` dérivé sans risque : `JetonIntermediaire` affiche `***`.
#[derive(Debug)]
pub enum ConnexionAdministrateur {
    /// TOTP actif : jeton `laaficare:second_facteur_administrateur`.
    SecondFacteur { jeton_intermediaire: JetonIntermediaire },
    /// Pas de TOTP actif, fenêtre en cours : jeton
    /// `laaficare:reenrolement_administrateur`.
    Reenrolement { jeton_reenrolement: JetonIntermediaire },
}

/// Connexion d'un administrateur (`POST /api/administrateurs/connexion`).
/// Mêmes erreurs que la connexion professionnelle (mêmes messages).
pub async fn se_connecter(
    pool: &PgPool,
    jwt: &JwtService,
    email: &str,
    mot_de_passe: &str,
) -> Result<ConnexionAdministrateur, ErreurAuthProfessionnel> {
    let compte =
        auth_professionnel::verifier_identifiants(pool, "administrateur_laaficare", email, mot_de_passe).await?;

    let sorte = if compte.second_facteur {
        SorteJeton::SecondFacteurAdministrateur
    } else if compte.reenrolement_autorise {
        SorteJeton::ReenrolementAdministrateur
    } else {
        // Le mot de passe a déjà été vérifié (Argon2id) : même temps de
        // réponse qu'un mauvais mot de passe, même réponse.
        return Err(ErreurAuthProfessionnel::IdentifiantsInvalides);
    };
    let jeton = JetonIntermediaire::depuis(
        jwt.emettre(sorte, compte.compte_id, compte.version_jeton, None, None)
            .map_err(|_| auth_professionnel::interne("émission du jeton administrateur"))?,
    );
    Ok(match sorte {
        SorteJeton::SecondFacteurAdministrateur => ConnexionAdministrateur::SecondFacteur { jeton_intermediaire: jeton },
        _ => ConnexionAdministrateur::Reenrolement { jeton_reenrolement: jeton },
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ErreurReenrolement {
    /// `commencer` : mot de passe faux ou compte verrouillé (A7). L'email
    /// n'est pas resaisi ici, d'où un message propre.
    #[error("Mot de passe incorrect.")]
    MotDePasseIncorrect,
    /// `confirmer` : code faux, aucune demande en attente, fenêtre expirée
    /// ou compte verrouillé : une seule réponse (comme F3).
    #[error("Code incorrect.")]
    CodeInvalide,
    /// Seulement l'étape en échec, un texte fixe : jamais le mot de passe
    /// ni le code saisis.
    #[error("erreur interne : {0}")]
    Interne(&'static str),
}

/// Le détail d'une erreur de `totp.rs` n'est pas journalisé : il ne
/// garantit pas l'absence de la saisie.
fn interne(etape: &'static str) -> ErreurReenrolement {
    tracing::error!(etape, "réenrôlement administrateur : erreur interne");
    ErreurReenrolement::Interne(etape)
}

/// Premier temps du réenrôlement : le mot de passe est redemandé (Z7, A2 a)
/// et compte dans le verrouillage. Une demande déjà en attente est
/// remplacée (X3 : QR code perdu). Appelé derrière l'extracteur
/// `ReenrolementAdministrateur`, qui a vérifié la fenêtre.
pub async fn commencer_reenrolement(
    pool: &PgPool,
    cle: &CleTotp,
    compte_id: Uuid,
    mot_de_passe: &str,
) -> Result<ActivationEnCours, ErreurReenrolement> {
    totp::commencer_activation(pool, cle.octets(), compte_id, mot_de_passe)
        .await
        .map_err(|erreur| match erreur {
            ErreurTotp::Refuse => ErreurReenrolement::MotDePasseIncorrect,
            ErreurTotp::Interne(_) => interne("début du réenrôlement"),
        })
}

/// Second temps : un premier code correct active le TOTP et vide la
/// fenêtre dans la même transaction (`Autorisation::ReenrolementAdministrateur`).
/// Renvoie les codes de secours en clair : seul moment où ils sont
/// visibles (Z6). Aucune session n'est ouverte (A3).
pub async fn confirmer_reenrolement(
    pool: &PgPool,
    cle: &CleTotp,
    compte_id: Uuid,
    code: &str,
) -> Result<Vec<String>, ErreurReenrolement> {
    totp::confirmer_activation(pool, cle.octets(), compte_id, code, Autorisation::ReenrolementAdministrateur)
        .await
        .map_err(|erreur| match erreur {
            ErreurTotp::Refuse => ErreurReenrolement::CodeInvalide,
            ErreurTotp::Interne(_) => interne("confirmation du réenrôlement"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth_professionnel::{ConnexionProfessionnelle, SaisieSecondFacteur, terminer_connexion};
    use crate::mot_de_passe;

    // Base temporaire par test ; données fictives.

    const BON: &str = "Exemple#2026";
    const MAUVAIS: &str = "Mauvais#2026";

    fn jwt() -> JwtService {
        JwtService::new(&"a".repeat(32))
    }

    fn cle() -> CleTotp {
        CleTotp::depuis_octets([7; 32])
    }

    fn maintenant() -> u64 {
        jsonwebtoken::get_current_timestamp()
    }

    /// Compte du type demandé, email vérifié, mot de passe `BON`.
    async fn compte(pool: &PgPool, telephone: &str, email: &str, type_compte: &str) -> Uuid {
        let utilisateur_id: Uuid = sqlx::query_scalar(
            "INSERT INTO utilisateur (telephone, email) VALUES ($1, $2) RETURNING id",
        )
        .bind(telephone)
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap();
        sqlx::query("UPDATE utilisateur SET email_verifie_le = now() WHERE id = $1")
            .bind(utilisateur_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO compte (utilisateur_id, type_compte, mot_de_passe_hash) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(utilisateur_id)
        .bind(type_compte)
        .bind(mot_de_passe::hacher(BON).unwrap())
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn administrateur(pool: &PgPool, telephone: &str, email: &str) -> Uuid {
        compte(pool, telephone, email, "administrateur_laaficare").await
    }

    /// Fenêtre comme l'ouvrira l'outil local (horloge de PostgreSQL) ; un
    /// nombre de minutes négatif donne une fenêtre déjà expirée.
    async fn ouvrir_fenetre(pool: &PgPool, compte_id: Uuid, minutes: i32) {
        sqlx::query("UPDATE compte SET reenrolement_autorise_jusqu_au = now() + make_interval(mins => $2) WHERE id = $1")
            .bind(compte_id)
            .bind(minutes)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn fenetre_vide(pool: &PgPool, compte_id: Uuid) -> bool {
        sqlx::query_scalar("SELECT reenrolement_autorise_jusqu_au IS NULL FROM compte WHERE id = $1")
            .bind(compte_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn statut_totp(pool: &PgPool, compte_id: Uuid) -> Vec<String> {
        sqlx::query_scalar("SELECT statut FROM second_facteur_totp WHERE compte_id = $1 ORDER BY statut")
            .bind(compte_id)
            .fetch_all(pool)
            .await
            .unwrap()
    }

    fn generateur(activation: &ActivationEnCours) -> totp_rs::Totp {
        totp_rs::Builder::new()
            .with_secret(totp_rs::Secret::try_from_base32(&activation.cle_base32).unwrap())
            .build()
            .unwrap()
    }

    fn refuse(resultat: Result<ConnexionAdministrateur, ErreurAuthProfessionnel>) -> bool {
        matches!(resultat, Err(ErreurAuthProfessionnel::IdentifiantsInvalides))
    }

    /// Réenrôlement complet par les fonctions publiques : renvoie le
    /// générateur du nouveau TOTP et les codes de secours.
    async fn reenroler(pool: &PgPool, compte_id: Uuid) -> (totp_rs::Totp, Vec<String>) {
        let activation = commencer_reenrolement(pool, &cle(), compte_id, BON).await.unwrap();
        let generateur = generateur(&activation);
        let codes = confirmer_reenrolement(pool, &cle(), compte_id, &generateur.generate(maintenant()).to_string())
            .await
            .unwrap();
        (generateur, codes)
    }

    #[sqlx::test]
    #[ignore]
    async fn sans_fenetre_ni_totp_aucun_jeton_meme_avec_le_bon_mot_de_passe(pool: PgPool) {
        let jwt = jwt();
        let compte_id = administrateur(&pool, "+22670110001", "adm.sans@example.org").await;
        assert!(refuse(se_connecter(&pool, &jwt, "adm.sans@example.org", BON).await), "jamais autorisé");

        ouvrir_fenetre(&pool, compte_id, -1).await;
        assert!(refuse(se_connecter(&pool, &jwt, "adm.sans@example.org", BON).await), "fenêtre expirée");
        // Même réponse qu'un mauvais mot de passe.
        assert!(refuse(se_connecter(&pool, &jwt, "adm.sans@example.org", MAUVAIS).await));
    }

    #[sqlx::test]
    #[ignore]
    async fn fenetre_ouverte_donne_un_jeton_de_reenrolement(pool: PgPool) {
        let jwt = jwt();
        let compte_id = administrateur(&pool, "+22670110002", "adm.fenetre@example.org").await;
        ouvrir_fenetre(&pool, compte_id, FENETRE_REENROLEMENT_MINUTES).await;

        let ConnexionAdministrateur::Reenrolement { jeton_reenrolement } =
            se_connecter(&pool, &jwt, "adm.fenetre@example.org", BON).await.unwrap()
        else {
            panic!("réenrôlement attendu");
        };
        let claims = jwt.verifier(jeton_reenrolement.en_clair(), SorteJeton::ReenrolementAdministrateur).unwrap();
        assert_eq!(claims.sub, compte_id);
        assert_eq!(format!("{jeton_reenrolement:?}"), "***");
    }

    /// Parcours complet : réenrôlement, fenêtre vidée, puis reconnexion par
    /// la seule voie qui ouvre une session (mot de passe puis code).
    #[sqlx::test]
    #[ignore]
    async fn reenrolement_puis_reconnexion_avec_le_nouveau_totp(pool: PgPool) {
        let jwt = jwt();
        let compte_id = administrateur(&pool, "+22670110003", "adm.parcours@example.org").await;
        ouvrir_fenetre(&pool, compte_id, FENETRE_REENROLEMENT_MINUTES).await;

        let (generateur, codes) = reenroler(&pool, compte_id).await;
        assert_eq!(codes.len(), 10);
        assert!(fenetre_vide(&pool, compte_id).await, "fenêtre vidée par la confirmation");
        assert_eq!(statut_totp(&pool, compte_id).await, ["actif"]);
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1")
            .bind(compte_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(sessions, 0, "aucune session à la confirmation (A3)");

        let ConnexionAdministrateur::SecondFacteur { jeton_intermediaire } =
            se_connecter(&pool, &jwt, "adm.parcours@example.org", BON).await.unwrap()
        else {
            panic!("second facteur attendu");
        };
        let iat = jwt.verifier(jeton_intermediaire.en_clair(), SorteJeton::SecondFacteurAdministrateur).unwrap().iat;
        // Pas suivant : le code de confirmation ne peut pas resservir.
        let code = generateur.generate(maintenant() + 30).to_string();
        let paire = terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(code)).await;
        let Ok(paire) = paire else { panic!("session attendue") };
        assert!(jwt.verifier(&paire.jeton_acces, SorteJeton::Administrateur).is_ok());
    }

    async fn sessions(pool: &PgPool, compte_id: Uuid) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM session WHERE compte_id = $1")
            .bind(compte_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Y4 : aucun chemin sans code vérifié n'ouvre de session. Après chacun
    /// des six chemins, 0 session en base ; seul le bon code en ouvre une.
    #[sqlx::test]
    #[ignore]
    async fn aucun_chemin_sans_code_n_ouvre_de_session(pool: PgPool) {
        let jwt = jwt();
        let email = "adm.chemins@example.org";
        let compte_id = administrateur(&pool, "+22670110009", email).await;

        // 1. Connexion sans TOTP ni fenêtre : refusée.
        assert!(refuse(se_connecter(&pool, &jwt, email, BON).await));
        assert_eq!(sessions(&pool, compte_id).await, 0, "chemin 1 : sans TOTP ni fenêtre");

        // 2. Connexion pendant la fenêtre : jeton de réenrôlement seulement.
        ouvrir_fenetre(&pool, compte_id, FENETRE_REENROLEMENT_MINUTES).await;
        assert!(matches!(
            se_connecter(&pool, &jwt, email, BON).await,
            Ok(ConnexionAdministrateur::Reenrolement { .. })
        ));
        assert_eq!(sessions(&pool, compte_id).await, 0, "chemin 2 : connexion pendant la fenêtre");

        // 3. Début du réenrôlement.
        let activation = commencer_reenrolement(&pool, &cle(), compte_id, BON).await.unwrap();
        assert_eq!(sessions(&pool, compte_id).await, 0, "chemin 3 : commencer");

        // 4. Confirmation.
        let generateur = generateur(&activation);
        confirmer_reenrolement(&pool, &cle(), compte_id, &generateur.generate(maintenant()).to_string())
            .await
            .unwrap();
        assert_eq!(sessions(&pool, compte_id).await, 0, "chemin 4 : confirmer");

        // 5. Connexion avec un TOTP actif : jeton intermédiaire seulement.
        let ConnexionAdministrateur::SecondFacteur { jeton_intermediaire } =
            se_connecter(&pool, &jwt, email, BON).await.unwrap()
        else {
            panic!("second facteur attendu");
        };
        assert_eq!(sessions(&pool, compte_id).await, 0, "chemin 5 : connexion avec TOTP actif");

        // 6. Second temps avec un code faux, puis un code de secours faux.
        let iat = jwt.verifier(jeton_intermediaire.en_clair(), SorteJeton::SecondFacteurAdministrateur).unwrap().iat;
        let bon_code = generateur.generate(maintenant() + 30).to_string();
        let code_faux = if bon_code == "000000" { "000001" } else { "000000" };
        for saisie in [SaisieSecondFacteur::Code(code_faux.into()), SaisieSecondFacteur::CodeSecours("AAAAA-AAAAA".into())] {
            assert!(matches!(
                terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &saisie).await,
                Err(crate::auth_professionnel::ErreurSecondFacteur::CodeInvalide)
            ));
        }
        assert_eq!(sessions(&pool, compte_id).await, 0, "chemin 6 : codes faux");

        // Le bon code ouvre une session, une seule.
        let Ok(_paire) = terminer_connexion(&pool, &jwt, &cle(), compte_id, iat, &SaisieSecondFacteur::Code(bon_code)).await
        else {
            panic!("session attendue avec le bon code");
        };
        assert_eq!(sessions(&pool, compte_id).await, 1);
    }

    /// Les codes de secours ne sont renvoyés que par la confirmation : un
    /// second appel est refusé, et la base n'en garde que les empreintes.
    #[sqlx::test]
    #[ignore]
    async fn codes_de_secours_renvoyes_une_seule_fois(pool: PgPool) {
        let compte_id = administrateur(&pool, "+22670110004", "adm.codes@example.org").await;
        ouvrir_fenetre(&pool, compte_id, FENETRE_REENROLEMENT_MINUTES).await;
        let (generateur, codes) = reenroler(&pool, compte_id).await;

        assert!(matches!(
            confirmer_reenrolement(&pool, &cle(), compte_id, &generateur.generate(maintenant() + 30).to_string()).await,
            Err(ErreurReenrolement::CodeInvalide)
        ));

        let empreintes: Vec<String> = sqlx::query_scalar(
            "SELECT cs.hash FROM code_secours cs JOIN second_facteur_totp t ON t.id = cs.second_facteur_id \
             WHERE t.compte_id = $1",
        )
        .bind(compte_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(empreintes.len(), 10);
        for empreinte in &empreintes {
            assert!(empreinte.starts_with("$argon2id$"));
            assert!(!codes.iter().any(|code| empreinte.contains(code.as_str())));
        }
    }

    /// Fenêtre expirée entre `commencer` et `confirmer` : la confirmation
    /// est refusée, le TOTP reste en attente.
    #[sqlx::test]
    #[ignore]
    async fn fenetre_expiree_avant_la_confirmation(pool: PgPool) {
        let compte_id = administrateur(&pool, "+22670110005", "adm.expiree@example.org").await;
        ouvrir_fenetre(&pool, compte_id, FENETRE_REENROLEMENT_MINUTES).await;
        let activation = commencer_reenrolement(&pool, &cle(), compte_id, BON).await.unwrap();

        ouvrir_fenetre(&pool, compte_id, -1).await;
        let code = generateur(&activation).generate(maintenant()).to_string();
        assert!(matches!(
            confirmer_reenrolement(&pool, &cle(), compte_id, &code).await,
            Err(ErreurReenrolement::CodeInvalide)
        ));
        assert_eq!(statut_totp(&pool, compte_id).await, ["en_attente"]);
        assert!(!fenetre_vide(&pool, compte_id).await, "rien n'est modifié");
    }

    #[sqlx::test]
    #[ignore]
    async fn mot_de_passe_faux_au_debut_du_reenrolement(pool: PgPool) {
        let compte_id = administrateur(&pool, "+22670110006", "adm.mdp@example.org").await;
        ouvrir_fenetre(&pool, compte_id, FENETRE_REENROLEMENT_MINUTES).await;
        let erreur = commencer_reenrolement(&pool, &cle(), compte_id, MAUVAIS).await.err().unwrap();
        assert!(matches!(erreur, ErreurReenrolement::MotDePasseIncorrect));
        assert_eq!(erreur.to_string(), "Mot de passe incorrect.");
        assert!(statut_totp(&pool, compte_id).await.is_empty());
    }

    /// Chaque sorte de compte n'a que sa route : un administrateur ne se
    /// connecte pas comme professionnel, ni l'inverse.
    #[sqlx::test]
    #[ignore]
    async fn chaque_compte_sur_sa_seule_route(pool: PgPool) {
        let jwt = jwt();
        let administrateur_id = administrateur(&pool, "+22670110007", "adm.route@example.org").await;
        ouvrir_fenetre(&pool, administrateur_id, FENETRE_REENROLEMENT_MINUTES).await;
        compte(&pool, "+22670110008", "pro.route@example.org", "professionnel").await;

        assert!(matches!(
            auth_professionnel::se_connecter(&pool, &jwt, "adm.route@example.org", BON).await,
            Err(ErreurAuthProfessionnel::IdentifiantsInvalides)
        ));
        assert!(refuse(se_connecter(&pool, &jwt, "pro.route@example.org", BON).await));
        // Contrôle : la route professionnelle accepte bien le professionnel.
        assert!(matches!(
            auth_professionnel::se_connecter(&pool, &jwt, "pro.route@example.org", BON).await,
            Ok(ConnexionProfessionnelle::Complete(_))
        ));
    }

    /// Une erreur interne provoquée ne contient jamais la saisie.
    #[sqlx::test]
    #[ignore]
    async fn erreur_interne_sans_donnee_saisie(pool: PgPool) {
        pool.close().await;
        let mot_de_passe = "MotDePasse#Secret1";
        let erreur = commencer_reenrolement(&pool, &cle(), Uuid::from_u128(1), mot_de_passe).await.err().unwrap();
        assert!(matches!(erreur, ErreurReenrolement::Interne(_)), "{erreur:?}");
        let code = "123456";
        let erreur_code = confirmer_reenrolement(&pool, &cle(), Uuid::from_u128(1), code).await.err().unwrap();
        for texte in [erreur.to_string(), format!("{erreur:?}"), erreur_code.to_string(), format!("{erreur_code:?}")] {
            assert!(!texte.contains(mot_de_passe) && !texte.contains(code), "{texte}");
        }
    }
}
