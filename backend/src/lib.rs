//! Bibliothèque partagée entre le binaire serveur (`main.rs`) et les outils
//! locaux d'administration (`src/bin/*.rs`, ex. création du premier
//! administrateur). `routes_*` reste hors de cette bibliothèque : ces
//! modules dépendent de `AppState` (défini dans `main.rs`, propre au serveur
//! HTTP), aucun outil local n'en a besoin.

pub mod adresse_email;
pub mod affectation;
pub mod alerte_securite;
pub mod auth_administrateur;
pub mod auth_patient;
pub mod auth_professionnel;
pub mod compte_professionnel;
pub mod config;
pub mod console;
pub mod contrat_assurance_munaseb;
pub mod db;
pub mod decision_structure;
pub mod demande_remboursement_munaseb;
pub mod email;
pub mod espace_patient_munaseb;
pub mod intervention_compte;
pub mod invitation;
pub mod jeton_email;
pub mod jwt;
pub mod membres;
pub mod mot_de_passe;
pub mod nip;
pub mod notification;
pub mod otp;
pub mod outils_administrateur;
pub mod patient;
pub mod reinitialisation;
pub mod session;
pub mod sms;
pub mod structure;
pub mod telephone;
pub mod totp;
pub mod verification_email;
pub mod verrouillage;
