//! Hachage et vérification du mot de passe des rôles non-Patient (section 11).
//!
//! Argon2id : décision actée (remplace BCrypt initialement prévu en BNF-01),
//! recommandé en premier par la fiche OWASP « Password Storage Cheat Sheet »
//! (Argon2id, minimum m=19 MiB, t=2, p=1 ; BCrypt réservé aux systèmes
//! hérités où Argon2/scrypt sont indisponibles — pas le cas ici, projet
//! neuf). Paramètres explicites en constantes nommées plutôt que les
//! valeurs par défaut du crate, même raisonnement que MAX_TENTATIVES dans
//! db.rs : une exigence de sécurité doit être visible et ajustable, pas
//! cachée dans un comportement implicite.
//!
//! Doc suivie : docs.rs/argon2/0.6.0 (Params::new, Argon2::new,
//! PasswordHasher::hash_password, PasswordVerifier::verify_password) et
//! docs.rs/password-hash/0.6.1 (variantes d'erreur).

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{Error, PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use unicode_normalization::UnicodeNormalization;

// Longueurs, référence NIST SP 800-63B révision 4 (26 août 2025), §3.1.1.2,
// qui remplace la révision 3 (§5.1.1.2) citée auparavant.
//
// ÉCART ASSUMÉ (décision du porteur, 2026-10-02) : la révision 4 exige
// « a minimum of 15 characters » pour un mot de passe utilisé comme seul
// facteur, et n'autorise 8 caractères que « as part of multi-factor
// authentication ». Le second facteur (TOTP) reste facultatif pour les
// patients et les professionnels : 8 caractères pour tous les comptes est
// donc un écart volontaire au texte, documenté dans CLAUDE.md (section 14).
pub const LONGUEUR_MOT_DE_PASSE_MIN: usize = 8;

// Révision 4 : « SHOULD permit a maximum password length of at least 64
// characters » ; 128 respecte ce minimum et borne le coût du hachage d'une
// requête démesurée (décision N3).
pub const LONGUEUR_MOT_DE_PASSE_MAX: usize = 128;

/// Forme NFC du mot de passe. Révision 4 : « the verifier SHOULD apply the
/// normalization process for stabilized strings using the Normalization
/// Form Canonical Composition (NFC) […]. This process is applied before
/// hashing ». Un même mot de passe accentué peut être codé différemment
/// selon le clavier (« é » en un caractère, ou « e » + accent) : sans
/// normalisation, le patient ne pourrait plus se connecter depuis un autre
/// appareil. Crate `unicode-normalization` 0.1.25 (Unicode Standard Annex
/// #15), `UnicodeNormalization::nfc`.
fn normaliser(mot_de_passe: &str) -> String {
    mot_de_passe.nfc().collect()
}

/// Règle de mot de passe non respectée. Le nom sérialisé (snake_case) est
/// celui que les applications reçoivent dans `regles_non_respectees`, pour
/// afficher une liste à cocher plutôt que de corriger règle par règle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegleMotDePasse {
    LongueurMin,
    LongueurMax,
    CaractereDeControle,
    Lettre,
    Chiffre,
    CaractereSpecial,
}

#[derive(Debug, thiserror::Error)]
#[error("mot de passe non conforme : {regles_non_respectees:?}")]
pub struct MotDePasseNonConforme {
    pub regles_non_respectees: Vec<RegleMotDePasse>,
}

/// Contrôle la règle des mots de passe (décisions du 2026-10-02/03, section
/// 14 du CLAUDE.md), à la création ou au changement d'un mot de passe
/// seulement -- jamais à la connexion. Renvoie TOUTES les règles non
/// respectées.
///
/// ÉCART ASSUMÉ à NIST SP 800-63B rév. 4 §3.1.1.2 (« SHALL NOT impose other
/// composition rules ») : la présence d'une lettre, d'un chiffre et d'un
/// caractère spécial est une règle décidée par le porteur.
///
/// Définitions (doc Rust, `char`, propriétés Unicode officielles) :
/// - lettre : `is_alphabetic()`, propriété Unicode *Alphabetic* -- les
///   lettres accentuées (é, ç, ô) et non latines comptent ;
/// - chiffre : `is_ascii_digit()`, 0 à 9 seulement (`¾`, `①`, `²` n'en sont
///   pas : ils comptent comme caractères spéciaux) ;
/// - caractère spécial : tout le reste, sauf les espaces (`is_whitespace`,
///   permises mais non comptées) et les caractères de contrôle
///   (`is_control`, refusés : impossibles à taper de façon fiable sur un
///   autre appareil).
///
/// Ordre : normalisation NFC, puis contrôles sur la forme normalisée.
/// Aucune espace n'est retirée : le mot de passe est pris tel que saisi.
pub fn controler(mot_de_passe: &str) -> Result<(), MotDePasseNonConforme> {
    let normalise = normaliser(mot_de_passe);
    let longueur = normalise.chars().count();
    let mut regles_non_respectees = Vec::new();

    if normalise.chars().any(char::is_control) {
        regles_non_respectees.push(RegleMotDePasse::CaractereDeControle);
    }
    if longueur < LONGUEUR_MOT_DE_PASSE_MIN {
        regles_non_respectees.push(RegleMotDePasse::LongueurMin);
    }
    if longueur > LONGUEUR_MOT_DE_PASSE_MAX {
        regles_non_respectees.push(RegleMotDePasse::LongueurMax);
    }
    if !normalise.chars().any(char::is_alphabetic) {
        regles_non_respectees.push(RegleMotDePasse::Lettre);
    }
    if !normalise.chars().any(|c| c.is_ascii_digit()) {
        regles_non_respectees.push(RegleMotDePasse::Chiffre);
    }
    let est_special =
        |c: char| !c.is_alphabetic() && !c.is_ascii_digit() && !c.is_whitespace() && !c.is_control();
    if !normalise.chars().any(est_special) {
        regles_non_respectees.push(RegleMotDePasse::CaractereSpecial);
    }

    if regles_non_respectees.is_empty() {
        Ok(())
    } else {
        Err(MotDePasseNonConforme { regles_non_respectees })
    }
}

/// Longueur d'un mot de passe telle que la règle la mesure : en caractères
/// (points de code), après normalisation NFC. Révision 4 : « Each Unicode
/// code point SHALL be counted as a single character when evaluating
/// password length. » Jamais `len()`, qui compte des octets : un caractère
/// accentué en vaut deux.
pub fn longueur(mot_de_passe: &str) -> usize {
    normaliser(mot_de_passe).chars().count()
}

// Minimum OWASP pour Argon2id : m=19456 KiB (19 MiB), t=2, p=1. `m_cost` est
// en Kio, pas en Mio (doc Params::new : « memory size in 1 KiB blocks »).
const M_COST_KIB: u32 = 19_456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

// Construit l'instance Argon2id une fois par appel : `Params::new` retourne
// un `Result` (les bornes m/t/p sont vérifiées), donc pas de `const`
// possible. Coût négligeable (pas de calcul cryptographique ici, juste la
// validation des paramètres), pas d'intérêt à la partager entre appels pour
// un module qui ne sert pas des milliers de requêtes/seconde.
fn instance() -> Result<Argon2<'static>, Error> {
    let params = Params::new(M_COST_KIB, T_COST, P_COST, None)?;
    // V0x13 = Argon2 v19, la version recommandée par la doc du crate
    // (`Argon2::default()` l'utilise aussi) — pas la v16 historique.
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hache un mot de passe en clair. Le résultat est une chaîne au format PHC
/// (`$argon2id$v=19$m=...,t=...,p=...$sel$hash`) : algorithme, paramètres
/// et sel sont encodés dedans, donc un seul champ `mot_de_passe_hash` suffit
/// en base, et relever `M_COST_KIB`/`T_COST` plus tard n'invalide pas les
/// hachages déjà stockés (recommandation OWASP : re-hacher à la prochaine
/// connexion plutôt qu'une migration en masse).
///
/// Le sel est généré aléatoirement par `hash_password` (feature `getrandom`
/// du crate, activée par défaut) : jamais fourni à la main.
pub fn hacher(mot_de_passe: &str) -> Result<String, Error> {
    let hash = instance()?.hash_password(normaliser(mot_de_passe).as_bytes())?;
    Ok(hash.to_string())
}

/// Vérifie un mot de passe en clair contre un hachage stocké (format PHC).
///
/// `Ok(false)` pour un mot de passe qui ne correspond pas (issue normale,
/// pas une erreur applicative) ; `Err` seulement pour un hachage
/// illisible/corrompu en base. La doc du crate précise que `verify_password`
/// utilise les paramètres encodés dans `hash`, pas ceux de l'instance
/// `Argon2` appelante — donc `instance()` sert seulement à obtenir un
/// objet sur lequel appeler la méthode, ses m/t/p n'entrent pas en jeu ici.
pub fn verifier(mot_de_passe: &str, hash: &str) -> Result<bool, Error> {
    let hash_analyse = PasswordHash::new(hash)?;
    match instance()?.verify_password(normaliser(mot_de_passe).as_bytes(), &hash_analyse) {
        Ok(()) => Ok(true),
        Err(Error::PasswordInvalid) => Ok(false),
        Err(erreur) => Err(erreur),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hache_puis_verifie_avec_le_bon_mot_de_passe() {
        let hash = hacher("un-mot-de-passe-correct").unwrap();
        assert!(verifier("un-mot-de-passe-correct", &hash).unwrap());
    }

    #[test]
    fn refuse_un_mauvais_mot_de_passe() {
        let hash = hacher("un-mot-de-passe-correct").unwrap();
        assert!(!verifier("un-autre-mot-de-passe", &hash).unwrap());
    }

    #[test]
    fn deux_hachages_du_meme_mot_de_passe_sont_differents() {
        // Sel aléatoire à chaque appel : deux hachages du même mot de passe
        // ne doivent jamais être identiques (sinon deux comptes avec le
        // même mot de passe seraient visiblement identiques en base).
        let a = hacher("identique").unwrap();
        let b = hacher("identique").unwrap();
        assert_ne!(a, b);
        assert!(verifier("identique", &a).unwrap());
        assert!(verifier("identique", &b).unwrap());
    }

    #[test]
    fn le_hachage_commence_par_argon2id() {
        let hash = hacher("peu-importe").unwrap();
        assert!(hash.starts_with("$argon2id$"));
    }

    #[test]
    fn meme_mot_de_passe_accentue_quel_que_soit_son_codage() {
        // « é » en un seul point de code (U+00E9) ou en « e » + accent
        // combinant (U+0065 U+0301) : deux saisies d'un même mot de passe
        // selon le clavier. La normalisation NFC les rend identiques.
        let compose = "mot-de-passe-\u{e9}t\u{e9}";
        let decompose = "mot-de-passe-e\u{301}te\u{301}";
        assert_ne!(compose.as_bytes(), decompose.as_bytes());
        let hash = hacher(compose).unwrap();
        assert!(verifier(decompose, &hash).unwrap());
    }

    #[test]
    fn longueur_comptee_en_caracteres_apres_normalisation() {
        // 7 caractères, mais 8 octets (« é » en vaut deux) : `len()` les
        // aurait comptés 8 et acceptés à tort.
        assert_eq!("motdep\u{e9}".len(), 8);
        assert_eq!(longueur("motdep\u{e9}"), 7);
        // Forme décomposée (9 octets, 8 points de code) : toujours 7
        // caractères une fois normalisée.
        assert_eq!(longueur("motdepe\u{301}"), 7);
    }

    fn regles(mot_de_passe: &str) -> Vec<RegleMotDePasse> {
        controler(mot_de_passe).err().map(|e| e.regles_non_respectees).unwrap_or_default()
    }

    #[test]
    fn mot_de_passe_conforme() {
        assert!(controler("Kabore-2026").is_ok());
        // Lettre accentuée : c'est une lettre.
        assert!(controler("\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}1!").is_ok());
        // Espaces permises.
        assert!(controler("mon fils Ali 12 !").is_ok());
        // Un emoji compte comme caractère spécial.
        assert!(controler("Bonjour1\u{1f49d}").is_ok());
    }

    #[test]
    fn regles_non_respectees_toutes_listees() {
        assert_eq!(
            regles("abc"),
            vec![RegleMotDePasse::LongueurMin, RegleMotDePasse::Chiffre, RegleMotDePasse::CaractereSpecial]
        );
        assert_eq!(regles("12345678!"), vec![RegleMotDePasse::Lettre]);
    }

    #[test]
    fn espace_permise_mais_pas_caractere_special() {
        assert_eq!(regles("mon fils Ali 12"), vec![RegleMotDePasse::CaractereSpecial]);
    }

    #[test]
    fn chiffre_de_0_a_9_seulement() {
        // « ² » et « ¾ » ne sont pas des chiffres ; ils comptent comme
        // caractères spéciaux.
        assert_eq!(regles("Bonjour\u{b2}\u{be}"), vec![RegleMotDePasse::Chiffre]);
    }

    #[test]
    fn caractere_de_controle_refuse() {
        assert_eq!(regles("Bonjour1!\t"), vec![RegleMotDePasse::CaractereDeControle]);
    }

    #[test]
    fn longueurs_limites_apres_normalisation() {
        // 7 caractères (« é » décomposé en 2 points de code, mais 1 après
        // NFC) : trop court.
        assert_eq!(regles("Abcde\u{301}1!"), vec![RegleMotDePasse::LongueurMin]);
        assert!(controler("Abcdef1!").is_ok());
        let max = format!("Ab1!{}", "x".repeat(LONGUEUR_MOT_DE_PASSE_MAX - 4));
        assert!(controler(&max).is_ok());
        assert_eq!(regles(&format!("{max}x")), vec![RegleMotDePasse::LongueurMax]);
    }

    #[test]
    fn noms_envoyes_aux_applications() {
        assert_eq!(
            serde_json::to_string(&vec![RegleMotDePasse::Chiffre, RegleMotDePasse::CaractereSpecial]).unwrap(),
            r#"["chiffre","caractere_special"]"#
        );
    }

    #[test]
    fn hachage_corrompu_renvoie_une_erreur_pas_un_faux() {
        let resultat = verifier("peu-importe", "pas-un-hachage-valide");
        assert!(resultat.is_err());
    }
}
