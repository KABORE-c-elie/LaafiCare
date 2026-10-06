//! Normalisation des numéros de téléphone, côté serveur (décision C3,
//! 2026-10-03) : l'app mobile et l'app web pourraient convertir
//! différemment, et un patient ne pourrait plus se connecter d'un appareil
//! à l'autre. Le format stocké est le format international `+226XXXXXXXX`
//! (contrainte `telephone_format`, migration 0001).
//!
//! Source pour le Burkina Faso : plan national de numérotage publié par
//! l'UIT (communication de l'ARCEP du 4.V.2023) -- numéro national de 8
//! chiffres, longueur minimale et maximale. Plusieurs numéros nationaux
//! commencent par 0 (préfixes 03, 05, 06…) : ce 0 fait partie du numéro et
//! n'est jamais retiré.

/// Indicatif du Burkina Faso (UIT, plan de numérotage +226).
const INDICATIF_BURKINA: &str = "226";
/// Longueur du numéro national burkinabè (UIT, communication ARCEP du
/// 4.V.2023 : « Longueur maximale 8, Longueur minimale 8 »).
const LONGUEUR_NUMERO_NATIONAL: usize = 8;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("numéro de téléphone invalide")]
pub struct TelephoneInvalide;

/// Caractères de la propriété Unicode *Dash* (Unicode 18.0.0,
/// `PropList.txt`, 31 points de code) : toutes les sortes de tirets qu'un
/// clavier de téléphone peut insérer, pas seulement le tiret simple
/// (décision T2).
fn est_tiret(c: char) -> bool {
    matches!(
        c,
        '\u{002D}'
            | '\u{058A}'
            | '\u{05BE}'
            | '\u{1400}'
            | '\u{1806}'
            | '\u{2010}'..='\u{2015}'
            | '\u{2053}'
            | '\u{207B}'
            | '\u{208B}'
            | '\u{2212}'
            | '\u{2E17}'
            | '\u{2E1A}'
            | '\u{2E3A}'..='\u{2E3B}'
            | '\u{2E40}'
            | '\u{2E5D}'
            | '\u{301C}'
            | '\u{3030}'
            | '\u{30A0}'
            | '\u{FE31}'..='\u{FE32}'
            | '\u{FE58}'
            | '\u{FE63}'
            | '\u{FF0D}'
            | '\u{10D6E}'
            | '\u{10EAD}'
    )
}

/// Ramène un numéro saisi au format `+<indicatif><numéro>`.
///
/// Dans cet ordre (décisions C3, T1 à T3) :
/// 1. retirer les espaces (au sens Unicode, espace insécable comprise), les
///    points et tous les tirets ; jamais les parenthèses ;
/// 2. 8 chiffres → `+226` devant ;
/// 3. `00226` + 8 chiffres, ou `226` + 8 chiffres → `+226` + 8 chiffres ;
/// 4. `+226` → exactement 8 chiffres après ;
/// 5. autre `+` (numéro étranger) → `+` suivi de 7 à 15 chiffres, le premier
///    différent de 0 (même règle que la contrainte SQL) ;
/// 6. tout le reste est invalide.
pub fn normaliser(saisie: &str) -> Result<String, TelephoneInvalide> {
    let nettoye: String = saisie
        .chars()
        .filter(|&c| !c.is_whitespace() && c != '.' && !est_tiret(c))
        .collect();

    let (avec_plus, chiffres) = match nettoye.strip_prefix('+') {
        Some(reste) => (true, reste),
        None => (false, nettoye.as_str()),
    };
    // Chiffres ASCII seulement : les parenthèses, lettres ou autres signes
    // rendent le numéro invalide.
    if chiffres.is_empty() || !chiffres.bytes().all(|b| b.is_ascii_digit()) {
        return Err(TelephoneInvalide);
    }

    // Numéro international sans le `+`.
    let international = if avec_plus {
        chiffres.to_string()
    } else if chiffres.len() == LONGUEUR_NUMERO_NATIONAL {
        format!("{INDICATIF_BURKINA}{chiffres}")
    } else if let Some(reste) = chiffres.strip_prefix("00").filter(|r| r.starts_with(INDICATIF_BURKINA)) {
        reste.to_string()
    } else if chiffres.starts_with(INDICATIF_BURKINA) {
        chiffres.to_string()
    } else {
        return Err(TelephoneInvalide);
    };

    let valide = if let Some(national) = international.strip_prefix(INDICATIF_BURKINA) {
        national.len() == LONGUEUR_NUMERO_NATIONAL
    } else {
        (7..=15).contains(&international.len()) && !international.starts_with('0')
    };

    if valide { Ok(format!("+{international}")) } else { Err(TelephoneInvalide) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(saisie: &str) -> Result<String, TelephoneInvalide> {
        normaliser(saisie)
    }

    #[test]
    fn formes_locales_et_internationales_du_meme_numero() {
        let attendu = Ok("+22670123456".to_string());
        for saisie in [
            "70123456",
            "70 12 34 56",
            "70-12-34-56",
            "70.12.34.56",
            "70\u{a0}12\u{a0}34\u{a0}56", // espaces insécables
            "70\u{2011}12\u{2011}34\u{2011}56", // tirets insécables
            "70\u{2013}12\u{2014}34\u{2212}56", // demi-cadratin, cadratin, signe moins
            "+226 70 12 34 56",
            "+22670123456",
            "00226 70 12 34 56",
            "0022670123456",
            "226 70 12 34 56",
            "22670123456",
        ] {
            assert_eq!(n(saisie), attendu, "saisie : {saisie:?}");
        }
    }

    #[test]
    fn le_zero_initial_du_numero_national_est_garde() {
        // Préfixe 05 attribué à Orange Burkina Faso (UIT, ARCEP 21.II.2022).
        assert_eq!(n("05 12 34 56"), Ok("+22605123456".to_string()));
        assert_eq!(n("+226 05 12 34 56"), Ok("+22605123456".to_string()));
    }

    #[test]
    fn numero_etranger_garde_tel_quel() {
        assert_eq!(n("+33 6 12 34 56 78"), Ok("+33612345678".to_string()));
    }

    #[test]
    fn numeros_invalides() {
        for saisie in [
            "",
            "   ",
            "7012345",          // 7 chiffres
            "701234567",        // 9 chiffres
            "+226 7012345",     // 7 chiffres après +226
            "+226 701234567",   // 9 chiffres après +226
            "(70) 12 34 56",    // parenthèses non retirées (décision T2)
            "70 12 34 5A",
            "0033612345678",    // 00 suivi d'un autre indicatif : non prévu
            "+0612345678",      // indicatif commençant par 0
            "+12345",           // trop court
            "+1234567890123456", // trop long
        ] {
            assert_eq!(n(saisie), Err(TelephoneInvalide), "saisie : {saisie:?}");
        }
    }
}
