//! Bibliothèque partagée entre le binaire serveur (`main.rs`) et les outils
//! locaux d'administration (`src/bin/*.rs`, ex. création de compte agent).
//! `routes_*` reste hors de cette bibliothèque : ces modules dépendent de
//! `AppState` (défini dans `main.rs`, propre au serveur HTTP), aucun outil
//! local n'en a besoin.

pub mod auth_agent_assurance_munaseb;
pub mod auth_patient;
pub mod auth_professionnel;
pub mod config;
pub mod contrat_assurance_munaseb;
pub mod db;
pub mod demande_remboursement_munaseb;
pub mod jwt;
pub mod mot_de_passe;
pub mod nip;
pub mod notification;
pub mod otp;
pub mod sms;
pub mod telephone;
pub mod totp;
pub mod verrouillage;
