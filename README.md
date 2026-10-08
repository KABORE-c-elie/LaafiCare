# LaafiCare

**Une plateforme numérique de santé pour le Burkina Faso.** « Laafi » signifie *santé, bien-être* en mooré.

LaafiCare relie, autour d'un **identifiant unique du patient** (le NIP), les personnes qui soignent et accompagnent un malade : médecins, infirmiers et sages-femmes, pharmaciens, biologistes, et l'assurance maladie.

Projet porté par **Calliste Elie KABORE**, ingénieur en électronique et informatique industrielle, qui en assure seul la conception et le développement.

> Ce document s'adresse aussi à des personnes qui n'ont jamais programmé. Chaque terme technique est expliqué à sa première apparition, et repris dans le [glossaire](docs/glossaire.md).

---

## Sommaire

1. [Le problème](#1-le-problème)
2. [Qui utilise LaafiCare](#2-qui-utilise-laaficare)
3. [Comment c'est construit](#3-comment-cest-construit)
4. [La sécurité en cinq minutes](#4-la-sécurité-en-cinq-minutes)
5. [État d'avancement](#5-état-davancement)
6. [Installer, lancer et tester](#6-installer-lancer-et-tester)
7. [Pour aller plus loin](#7-pour-aller-plus-loin)
8. [Sources](#8-sources)
9. [Auteur](#9-auteur)

---

## 1. Le problème

Au Burkina Faso, le parcours de soins repose presque entièrement sur le papier, Excel ou Word : dossier médical, suivi de grossesse, assurance maladie. Des entretiens menés sur le terrain (un médecin généraliste, un biologiste, un assistant pharmacien et cinq patients à Ouagadougou) en montrent les conséquences :

- un médecin perd environ **15 minutes par consultation** à reconstituer l'histoire médicale du patient ;
- **1 patient sur 5** interrogés a dû refaire et repayer des analyses dont les résultats s'étaient perdus ;
- **2 patients sur 5** ont eu une réaction à un médicament qui aurait pu être évitée, faute d'allergie vérifiée ;
- un pharmacien désigne la **saisie manuelle du patient** comme première source d'erreur ;
- un patient a attendu **plus d'un an** un remboursement d'assurance, sans réponse ;
- le carnet national de santé mère-enfant, qui prévoit 4 consultations prénatales (CPN), reste **entièrement papier** : aucune alerte en cas de rendez-vous manqué ou de tension trop élevée ;
- une pharmacie estime perdre **4 millions de FCFA par an** en médicaments périmés.

**Objectif de référence** : ramener le délai de remboursement de l'assurance d'environ un mois à **moins de 72 heures**, comme l'a déjà obtenu la mutuelle étudiante burkinabè MUNASEB avec sa propre plateforme.

## 2. Qui utilise LaafiCare

Le système comprend cinq applications, qui parlent toutes au même serveur :

| Application | Pour qui | Support |
|---|---|---|
| App mobile Patient | les patients | téléphone (Android, iOS) |
| App web Hôpital | médecins, infirmiers, sages-femmes | navigateur |
| App web Pharmacie | pharmaciens | navigateur |
| App web Laboratoire | biologistes | navigateur |
| App web Assurance | agents d'assurance maladie | navigateur |

Une sixième, pour l'état civil des mairies, est prévue dans une version ultérieure.

**Les rôles.** Dans un établissement, chacun a un rôle précis : médecin, infirmier (dont la sage-femme, qui est une spécialité d'infirmier), pharmacien, biologiste, agent d'accueil, major (responsable des infirmiers et des salles), chef de service, directeur, agent de la mutuelle MUNASEB, ou responsable de la structure. L'équipe LaafiCare, elle, valide les établissements.

**Une personne, plusieurs comptes.** Un médecin peut aussi tomber malade : il est alors patient. LaafiCare sépare donc :

- **l'identité** d'une personne (nom, prénom, téléphone) ;
- ses **comptes**, chacun avec son propre mot de passe : un compte *patient*, éventuellement un compte *professionnel* et, pour l'équipe LaafiCare, un compte *administrateur* ;
- ses **affectations** : un compte professionnel est rattaché à un ou plusieurs établissements, avec un rôle dans chacun.

Tout professionnel a d'abord un compte patient : c'est par lui que son numéro de téléphone a été vérifié.

## 3. Comment c'est construit

```mermaid
flowchart LR
    M["App mobile Patient<br/>(Flutter)"] --> API
    W["Apps web Hôpital, Pharmacie,<br/>Laboratoire, Assurance<br/>(Angular)"] --> API
    API["Serveur LaafiCare<br/>(Rust + Axum)"] --> DB[("Base de données<br/>PostgreSQL")]
    API --> SMS["SMS<br/>(Orange BF, Telecel)"]
    API --> PAY["Paiement<br/>(Orange Money, Moov Money)"]
```

Les applications ne parlent jamais directement à la base de données : elles passent par le **serveur**, qui expose une **API** (une liste d'adresses web précises, appelées *routes*, auxquelles une application envoie des demandes). Le serveur vérifie chaque demande avant de lire ou d'écrire quoi que ce soit.

| Brique | Choix | Pourquoi |
|---|---|---|
| Serveur | **Rust**, avec la bibliothèque **Axum** | Rust empêche, dès la fabrication du programme, toute une famille d'erreurs de mémoire qui font planter un serveur ou exposent des données. Pour des données de santé, aucune panne ni fuite n'est acceptable. |
| Accès à la base | **SQLx** | Bibliothèque Rust pour PostgreSQL. Elle fournit aussi les migrations (voir ci-dessous) et les tests en base. |
| Base de données | **PostgreSQL** 18 | Base relationnelle robuste : elle garantit elle-même de nombreuses règles (une valeur unique, une date obligatoire…), en plus du code. |
| Applications web | **Angular** | Cadre de développement web adapté aux applications de gestion utilisées toute la journée. |
| Application mobile | **Flutter** | Une seule base de code pour Android et iOS, et un fonctionnement possible hors connexion. |
| Connexion | **JWT** | Après connexion, l'application reçoit un *jeton* signé par le serveur, qu'elle présente à chaque demande. |
| SMS et paiement | API Orange BF, Telecel ; Orange Money, Moov Money | Les canaux que les patients interrogés utilisent tous déjà. |

**Les migrations.** La structure de la base (ses tables et leurs règles) est décrite dans des fichiers numérotés, les *migrations* ([`backend/migrations/`](backend/migrations/)). Le serveur applique à son démarrage celles qui ne l'ont pas encore été, dans l'ordre. Une migration déjà appliquée ne doit plus jamais changer : SQLx garde son empreinte, et le serveur refuserait de démarrer.

Le détail des tables est dans [`docs/modele-donnees.md`](docs/modele-donnees.md), celui des routes dans [`docs/api.md`](docs/api.md).

## 4. La sécurité en cinq minutes

Chaque choix ci-dessous suit une source reconnue, citée dans le code et détaillée dans [`docs/securite.md`](docs/securite.md).

**Les mots de passe ne sont jamais enregistrés.** Le serveur n'en garde qu'une *empreinte* (un *hachage*) : une transformation à sens unique, qui permet de vérifier un mot de passe sans pouvoir le retrouver. L'algorithme est **Argon2id**, recommandé par l'OWASP, et volontairement lent et gourmand en mémoire pour décourager les essais en masse. Fichier : [`mot_de_passe.rs`](backend/src/mot_de_passe.rs).

**Règles d'un mot de passe** : de 8 à 128 caractères, avec au moins une lettre, un chiffre et un caractère spécial. Les lettres accentuées sont acceptées et ramenées à une forme unique (*normalisation NFC*), pour qu'un même mot de passe tapé sur deux téléphones différents soit reconnu. La référence est la norme américaine NIST SP 800-63B révision 4 ; deux écarts à cette norme sont assumés et expliqués dans [`docs/securite.md`](docs/securite.md).

**Le verrouillage** : après 5 essais faux, le compte est bloqué 15 minutes. Chaque essai est compté *avant* d'être vérifié, pour que des essais lancés tous en même temps ne puissent pas dépasser la limite. Fichier : [`verrouillage.rs`](backend/src/verrouillage.rs).

**Le code reçu par SMS (OTP)** : un code à 6 chiffres, valable 5 minutes et utilisable une seule fois. Il ne sert qu'au patient, pour vérifier que le numéro lui appartient à l'inscription, et pour retrouver l'accès en cas de mot de passe oublié. Fichier : [`otp.rs`](backend/src/otp.rs).

**Le second facteur (TOTP)** : un code à 6 chiffres qui change toutes les 30 secondes, affiché par une application comme Google Authenticator, en plus du mot de passe. Il est facultatif pour les patients et les professionnels, et obligatoire pour l'équipe LaafiCare. Le secret partagé avec l'application est enregistré **chiffré**, un code n'est accepté qu'une fois, et 10 codes de secours, à usage unique, permettent de se connecter sans le téléphone. Fichier : [`totp.rs`](backend/src/totp.rs).

**Ce que LaafiCare refuse de révéler.** Taper un numéro ou un email ne dit jamais si la personne est inscrite. Numéro inconnu, mauvais mot de passe ou compte bloqué : la réponse est toujours la même (« Numéro ou mot de passe incorrect. »), et le serveur met le même temps à répondre dans tous les cas. Sinon, n'importe qui pourrait savoir qu'une personne est suivie par LaafiCare, ce qui est déjà une information de santé.

**Ce qui ne s'efface pas.** L'historique des décisions sur une demande de remboursement ou sur un établissement ne peut être ni modifié ni supprimé : c'est la base elle-même qui l'interdit.

**Les règles médicales non négociables** (à venir avec les modules médicaux) : les allergies s'affichent toujours en premier et en rouge ; un résultat d'analyse n'est visible qu'après double validation ; les résultats sensibles ne sont jamais envoyés automatiquement.

## 5. État d'avancement

Le projet a commencé par une phase de cadrage : un cahier des charges et des diagrammes (cas d'utilisation, modèle de données, séquences), établis à partir des entretiens. Le développement démarre maintenant, en commençant par le serveur et par un périmètre volontairement réduit : la mutuelle MUNASEB.

| Partie | État |
|---|---|
| Cahier des charges | rédigé ; mise à jour en cours (nouvelle architecture, nouveaux rôles) |
| Diagrammes | réalisés ; diagramme de classes et modèle relationnel à régénérer |
| Maquettes | deux premiers écrans mobiles |
| **Serveur : patients** | inscription par SMS, connexion, mot de passe oublié, verrouillage, NIP |
| **Serveur : MUNASEB** | contrats et périodes d'adhésion, demandes de remboursement de bout en bout (création, prise en charge, validation, rejet, paiement), notifications, historique ; **routes des agents hors service** jusqu'aux étapes 3.6 et 3.9 (elles reposent encore sur l'ancien compte agent, supprimé lors du passage aux comptes séparés) |
| **Serveur : comptes et établissements** | structure de la base en place (comptes séparés, établissements, pièces justificatives, affectations, invitations) |
| **Serveur : second facteur** | fait : chiffrement du secret, activation, codes, codes de secours |
| Serveur : connexion professionnelle | **prochaine étape** (étape 3.6) |
| Applications Angular et Flutter | pas encore commencées |

Les modules médicaux (dossier, ordonnances, pharmacie, laboratoire, suivi de grossesse) viendront ensuite. La liste détaillée des décisions encore ouvertes est dans [`docs/points-ouverts.md`](docs/points-ouverts.md).

## 6. Installer, lancer et tester

Ces étapes concernent le serveur, seule partie écrite pour l'instant. Les commandes se tapent dans un terminal (PowerShell sous Windows).

### Ce qu'il faut installer

- **Rust**, version 1.88 au moins, avec `rustup` (site officiel : rustup.rs). Vérification : `rustc --version`.
- **Docker Desktop**, qui fait tourner PostgreSQL sur votre machine sans l'installer directement.
- **Git**, pour récupérer le code.

### Préparer la configuration

1. Copier le modèle de configuration :
   ```
   cd backend
   copy .env.example .env
   ```
2. Ouvrir `backend/.env` et remplacer chaque `A_GENERER`. Le fichier explique comment générer chaque secret. Tant qu'un `A_GENERER` reste, le serveur refuse de démarrer : c'est voulu.

| Variable | Rôle | Exemple **fictif** |
|---|---|---|
| `POSTGRES_PASSWORD` | mot de passe de la base de développement, lu par Docker | une valeur choisie |
| `DATABASE_URL` | adresse de la base, avec ce même mot de passe | `postgres://utilisateur:motdepasse@127.0.0.1:5433/laaficare` |
| `SERVER_PORT` | port du serveur | `8080` |
| `JWT_SECRET` | signe les jetons de connexion, 32 caractères au moins | 64 caractères hexadécimaux générés |
| `TOTP_CLE_CHIFFREMENT` | chiffre les secrets du second facteur, exactement 64 caractères hexadécimaux | une autre valeur générée |

Le fichier `.env` contient des secrets : il n'est jamais envoyé dans Git.

### Lancer

```
docker compose up -d     # démarre PostgreSQL en arrière-plan
cargo run                # compile et lance le serveur
```

Au démarrage, le serveur applique les migrations manquantes. Pour vérifier qu'il répond, ouvrir `http://127.0.0.1:8080/health` dans un navigateur (avec le port choisi).

### Tester

Un *test* est un petit programme qui vérifie automatiquement qu'une règle est respectée. Depuis `backend/` :

```
cargo test                                   # tests sans base de données
cargo test --no-fail-fast -- --include-ignored   # tous les tests, PostgreSQL démarré
cargo test --lib -- --include-ignored totp        # seulement ceux dont le nom contient « totp »
```

`--no-fail-fast` fait tourner tous les groupes de tests même quand l'un d'eux échoue ; sans lui, Cargo s'arrête au premier groupe en échec et les suivants ne sont jamais lancés.

Les tests qui utilisent la base créent chacun une base temporaire, supprimée ensuite : la base de développement n'est jamais touchée.

**Échecs attendus pour l'instant** : deux tests échouent, `auth_professionnel` (ancienne connexion professionnelle, réécrite à l'étape 3.6) et `extracteur_jwt` (ancien contrôle de l'agent MUNASEB, réécrit à l'étape 3.9). Tout autre échec est un vrai problème.

## 7. Pour aller plus loin

- [`docs/securite.md`](docs/securite.md) : chaque choix de sécurité, sa source et son fichier.
- [`docs/modele-donnees.md`](docs/modele-donnees.md) : les tables et leurs règles.
- [`docs/api.md`](docs/api.md) : les routes du serveur, avec des exemples ; c'est le contrat sur lequel s'appuient les applications web et mobile.
- [`docs/points-ouverts.md`](docs/points-ouverts.md) : les décisions encore à prendre.
- [`docs/glossaire.md`](docs/glossaire.md) : les termes techniques et médicaux.
- [`CLAUDE.md`](CLAUDE.md) : le journal détaillé de toutes les décisions du projet.

## 8. Sources

- **NIST SP 800-63B, révision 4** (août 2025) : mots de passe, OTP, second facteur, limitation des essais.
- **OWASP** : *Password Storage Cheat Sheet* (Argon2id et ses paramètres), *File Upload Cheat Sheet* (pièces justificatives).
- **RFC 4226** (HOTP) et **RFC 6238** (TOTP) : codes à usage unique et second facteur.
- **RFC 7518** §3.2 : taille minimale du secret de signature des jetons.
- **UIT**, plan national de numérotage du Burkina Faso (communication de l'ARCEP du 4 mai 2023) : format des numéros de téléphone.
- **Documentation PostgreSQL 18** et **documentation Git** (gitattributes, gitignore).
- Documentation officielle des bibliothèques Rust utilisées : `axum`, `sqlx`, `argon2`, `jsonwebtoken`, `totp-rs`, `aes-gcm`, `unicode-normalization`, `rand`.
- **Carnet de santé mère-enfant** du Ministère de la Santé du Burkina Faso : calendrier des CPN, seuils d'alerte.
- **Mémoire sur la plateforme MUNASEB** : circuit d'un remboursement d'assurance.

## 9. Auteur

**Calliste Elie KABORE**, porteur du projet, maîtrise d'ouvrage et maîtrise d'œuvre.
