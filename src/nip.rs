//! Numéro d'Identification Patient (NIP) — nouveau format (décision actée) :
//! 16 chiffres, purement numérique, sans lettre ni tiret. 15 chiffres issus
//! d'une séquence PostgreSQL globale (pas par année -- un identifiant de
//! santé ne doit rien encoder de significatif, comme le NHS Number
//! britannique, le NPI américain ou l'IHI australien) + 1 chiffre de
//! contrôle Luhn (même principe que l'IHI), pour détecter une faute de
//! frappe sans aller vérifier en base.
//!
//! Algorithme de Luhn vérifié contre un exemple chiffré (pas recodé de
//! mémoire, section 13) : brevet original de Hans Peter Luhn (US2950048A)
//! pour le principe, complété par l'exemple travaillé de la page Wikipédia
//! "Luhn algorithm" pour lever l'ambiguïté d'implémentation du brevet
//! (sens de parcours, position doublée) -- `1789372997` → chiffre de
//! contrôle `4`, reproduit exactement par `tests::exemple_officiel_luhn`
//! ci-dessous.

use sqlx::PgPool;

/// Calcule le chiffre de contrôle de Luhn d'une charge utile numérique.
/// Parcourt les chiffres de `payload` de droite à gauche : les positions
/// impaires (1, 3, 5... en partant de la droite) sont doublées, avec
/// soustraction de 9 si le doublement dépasse 9 (équivalent à sommer les
/// deux chiffres du résultat, doc Wikipédia). Le chiffre de contrôle est le
/// complément à 10 de la somme totale, modulo 10.
fn chiffre_controle_luhn(payload: &str) -> u8 {
    let somme: u32 = payload
        .chars()
        .rev()
        .enumerate()
        .map(|(i, c)| {
            let chiffre = c
                .to_digit(10)
                .expect("payload doit être composé uniquement de chiffres ASCII");
            if i % 2 == 0 {
                let double = chiffre * 2;
                if double > 9 { double - 9 } else { double }
            } else {
                chiffre
            }
        })
        .sum();
    ((10 - (somme % 10)) % 10) as u8
}

/// Génère un nouveau NIP : `nextval('nip_seq')` (séquence PostgreSQL, voir
/// migration) zéro-complétée sur 15 chiffres, suivie de son chiffre de
/// contrôle Luhn.
pub async fn generer(pool: &PgPool) -> Result<String, sqlx::Error> {
    let valeur: i64 = sqlx::query_scalar("SELECT nextval('nip_seq')")
        .fetch_one(pool)
        .await?;
    let payload = format!("{valeur:015}");
    let cc = chiffre_controle_luhn(&payload);
    Ok(format!("{payload}{cc}"))
}

/// Valide un NIP fourni (format + chiffre de contrôle), sans accès à la
/// base -- utile pour rejeter immédiatement un NIP mal formé (faute de
/// frappe, QR code corrompu) avant toute requête.
pub fn valider(nip: &str) -> bool {
    if nip.len() != 16 || !nip.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (payload, chiffre_fourni) = nip.split_at(15);
    let attendu = chiffre_controle_luhn(payload);
    // Les 16 octets sont déjà vérifiés ASCII chiffre ci-dessus : le parse
    // ne peut pas échouer, `unwrap_or` reste une garde défensive.
    chiffre_fourni.parse::<u8>().unwrap_or(u8::MAX) == attendu
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exemple_officiel_luhn() {
        // Exemple travaillé de la doc Wikipédia "Luhn algorithm" (voir
        // commentaire de module) : 1789372997 -> chiffre de contrôle 4.
        assert_eq!(chiffre_controle_luhn("1789372997"), 4);
    }

    #[test]
    fn valide_le_nip_derive_de_l_exemple_officiel() {
        // Le payload de l'exemple Wikipédia (1789372997, 10 chiffres)
        // complété à gauche par des zéros pour obtenir les 15 chiffres du
        // format réel : ces zéros ne changent pas le résultat du calcul de
        // Luhn (ils valent 0 qu'ils soient doublés ou non).
        let payload = format!("{:0>15}", "1789372997");
        assert_eq!(chiffre_controle_luhn(&payload), 4);
        assert!(valider(&format!("{payload}4")));
    }

    #[test]
    fn valide_un_nip_correct() {
        let payload = "000000000000001";
        let cc = chiffre_controle_luhn(payload);
        let nip = format!("{payload}{cc}");
        assert!(valider(&nip));
    }

    #[test]
    fn refuse_un_chiffre_de_controle_incorrect() {
        let payload = "000000000000001";
        let cc = chiffre_controle_luhn(payload);
        let mauvais_cc = (cc + 1) % 10;
        let nip = format!("{payload}{mauvais_cc}");
        assert!(!valider(&nip));
    }

    #[test]
    fn refuse_une_longueur_incorrecte() {
        assert!(!valider("123"));
        assert!(!valider(&"1".repeat(17)));
    }

    #[test]
    fn refuse_des_caracteres_non_numeriques() {
        assert!(!valider("BF1234567890123X"));
    }

    // Base temporaire par test, jamais la base de développement : voir
    // « Tests et environnement », section 12 du CLAUDE.md. Avantage en plus :
    // la séquence `nip_seq` de la base de développement n'avance plus à
    // chaque exécution du test.
    #[sqlx::test]
    #[ignore]
    async fn generer_produit_un_nip_valide_et_unique_en_base(pool: PgPool) {
        let a = generer(&pool).await.unwrap();
        let b = generer(&pool).await.unwrap();

        assert_eq!(a.len(), 16);
        assert!(valider(&a), "le NIP généré doit passer sa propre validation");
        assert!(valider(&b));
        assert_ne!(a, b, "deux appels doivent produire des NIP différents (séquence)");
    }
}
