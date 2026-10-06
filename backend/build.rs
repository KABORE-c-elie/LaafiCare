// Sans ce script, Cargo ne sait pas qu'un fichier ajouté dans `migrations/`
// doit invalider le cache de `sqlx::migrate!()` (macro qui intègre le SQL à
// la compilation) : une nouvelle migration peut alors être silencieusement
// ignorée au démarrage tant qu'aucun fichier .rs n'a changé. Exactement le
// bug rencontré en ajoutant la migration OTP (0002) -- doc officielle,
// docs.rs/sqlx/0.9.0/sqlx/macro.migrate.html, section "recompilation".
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
