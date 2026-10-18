# LaafiCare

**Une plateforme numérique de santé pour le Burkina Faso.** « Laafi » signifie *santé, bien-être* en mooré.

LaafiCare relie, autour d'un **identifiant unique du patient** (le NIP), les personnes qui soignent et accompagnent un malade : médecins, infirmiers et sages-femmes, pharmaciens, biologistes, et l'assurance maladie.

Projet porté par **Calliste Elie KABORE**, ingénieur en électronique et informatique industrielle, qui en assure seul la conception et le développement.

> Ce document s'adresse aussi à des personnes qui n'ont jamais programmé. Chaque terme technique est expliqué à sa première apparition.

---

## Sommaire

1. [Le problème](#1-le-problème)
2. [Qui utilise LaafiCare](#2-qui-utilise-laaficare)
3. [Comment c'est construit](#3-comment-cest-construit)
4. [La sécurité en cinq minutes](#4-la-sécurité-en-cinq-minutes)
5. [État d'avancement](#5-état-davancement)
6. [Organisation du dépôt](#6-organisation-du-dépôt)
7. [Installer, lancer et tester](#7-installer-lancer-et-tester)
8. [Pour aller plus loin](#8-pour-aller-plus-loin)
9. [Sources](#9-sources)
10. [Auteur](#10-auteur)

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
| Connexion | **JWT** | Après connexion, l'application reçoit deux *jetons* : un **jeton d'accès** signé par le serveur, valable 15 minutes, qu'elle présente à chaque demande, et un **jeton de renouvellement**, qui en obtient un nouveau sans redemander le mot de passe. |
| SMS et paiement | API Orange BF, Telecel ; Orange Money, Moov Money | Les canaux que les patients interrogés utilisent tous déjà. |

**Les migrations.** La structure de la base (ses tables et leurs règles) est décrite dans des fichiers numérotés, les *migrations* ([`backend/migrations/`](backend/migrations/)). Le serveur applique à son démarrage celles qui ne l'ont pas encore été, dans l'ordre. Une migration déjà appliquée ne doit plus jamais changer : SQLx garde son empreinte, et le serveur refuserait de démarrer.

Le détail des tables est dans [`docs/modele-donnees.md`](docs/modele-donnees.md), celui des routes dans [`docs/api.md`](docs/api.md).

## 4. La sécurité en cinq minutes

Chaque choix ci-dessous suit une source reconnue, citée dans le code et détaillée dans [`docs/securite.md`](docs/securite.md).

**Les mots de passe ne sont jamais enregistrés.** Le serveur n'en garde qu'une *empreinte* (un *hachage*) : une transformation à sens unique, qui permet de vérifier un mot de passe sans pouvoir le retrouver. L'algorithme est **Argon2id**, recommandé par l'OWASP, et volontairement lent et gourmand en mémoire pour décourager les essais en masse. Fichier : [`mot_de_passe.rs`](backend/src/mot_de_passe.rs).

**Règles d'un mot de passe** : de 8 à 128 caractères, avec au moins une lettre, un chiffre et un caractère spécial. Les lettres accentuées sont acceptées et ramenées à une forme unique (*normalisation NFC*), pour qu'un même mot de passe tapé sur deux téléphones différents soit reconnu. La référence est la norme américaine NIST SP 800-63B révision 4 ; deux écarts à cette norme sont assumés et expliqués dans [`docs/securite.md`](docs/securite.md).

**Le verrouillage** : après 5 essais faux, le compte est bloqué 15 minutes. Chaque essai est compté *avant* d'être vérifié, pour que des essais lancés tous en même temps ne puissent pas dépasser la limite. Fichier : [`verrouillage.rs`](backend/src/verrouillage.rs).

**Le code reçu par SMS (OTP)** : un code à 6 chiffres, valable 5 minutes et utilisable une seule fois. Il ne sert qu'au patient, pour vérifier que le numéro lui appartient à l'inscription, et pour retrouver l'accès en cas de mot de passe oublié. Il est limité, pour qu'on ne puisse ni le deviner ni faire envoyer des SMS sans fin : 60 secondes au moins entre deux codes, 5 codes par heure pour un même numéro, et 5 essais par code (puis un seul essai par nouveau code, tant qu'aucun code juste n'a été saisi). Au-delà, le serveur répond « trop de demandes » et indique combien de secondes attendre. Fichier : [`otp.rs`](backend/src/otp.rs).

**Les jetons de connexion** : chaque sorte de jeton (patient, professionnel, administrateur, étape intermédiaire du second facteur…) n'ouvre que les routes qui lui sont destinées. Après une réinitialisation du mot de passe, tous les jetons émis avant elle sont refusés : un voleur de jeton perd l'accès dès que la personne change son mot de passe. Fichiers : [`jwt.rs`](backend/src/jwt.rs), [`extracteur_jwt.rs`](backend/src/extracteur_jwt.rs).

**Les sessions** : chaque connexion ouvre une *session*, une par appareil. Le jeton d'accès ne vit que 15 minutes ; le jeton de renouvellement en obtient un nouveau, et **change à chaque usage**. Si un ancien jeton de renouvellement, déjà remplacé, est présenté à nouveau (par exemple après un vol), le serveur ferme la session. Le patient reste connecté jusqu'à 30 jours sans retaper son mot de passe ; il peut se déconnecter d'un appareil, ou de tous à la fois. Le serveur ne garde qu'une empreinte du jeton de renouvellement, jamais le jeton lui-même. Sources : RFC 9700 (bonnes pratiques de sécurité OAuth 2.0), NIST SP 800-63B. Fichier : [`session.rs`](backend/src/session.rs).

**Le second facteur (TOTP)** : un code à 6 chiffres qui change toutes les 30 secondes, affiché par une application comme Google Authenticator, en plus du mot de passe. Il est facultatif pour les patients et les professionnels, et obligatoire pour l'équipe LaafiCare. Le secret partagé avec l'application est enregistré **chiffré**, un code n'est accepté qu'une fois, et 10 codes de secours, à usage unique, permettent de se connecter sans le téléphone. Fichier : [`totp.rs`](backend/src/totp.rs).

**Ce que LaafiCare refuse de révéler.** Taper un numéro ou un email ne dit jamais si la personne est inscrite. Numéro inconnu, mauvais mot de passe ou compte bloqué : la réponse est toujours la même (« Numéro ou mot de passe incorrect. »), et le serveur met le même temps à répondre dans tous les cas. Sinon, n'importe qui pourrait savoir qu'une personne est suivie par LaafiCare, ce qui est déjà une information de santé.

**Ce qui ne s'efface pas.** L'historique des décisions sur une demande de remboursement ou sur un établissement ne peut être ni modifié ni supprimé : c'est la base elle-même qui l'interdit.

**Les règles médicales non négociables** (à venir avec les modules médicaux) : les allergies s'affichent toujours en premier et en rouge ; un résultat d'analyse n'est visible qu'après double validation ; les résultats sensibles ne sont jamais envoyés automatiquement.

**Les couleurs ont un sens, dans toutes les applications**, quel que soit leur thème :

| Couleur | Réservée à |
|---|---|
| Rouge | les alertes de santé (danger) : allergie, tension trop élevée, signe de danger |
| Orange | les valeurs de santé à surveiller (vigilance) |
| Couleur neutre du thème | les erreurs de saisie (mot de passe incorrect, numéro invalide, code expiré…), toujours avec une icône et un texte |

La couleur ne porte jamais seule une information : chaque alerte a aussi un texte ou une icône, pour les personnes qui distinguent mal les couleurs (norme d'accessibilité WCAG 2.2, critère 1.4.1).

## 5. État d'avancement

Le projet a commencé par une phase de cadrage : un cahier des charges et des diagrammes (cas d'utilisation, modèle de données, séquences), établis à partir des entretiens. Le développement démarre maintenant, en commençant par le serveur et par un périmètre volontairement réduit : la mutuelle MUNASEB.

| Partie | État |
|---|---|
| Cahier des charges | rédigé ; mise à jour en cours (nouvelle architecture, nouveaux rôles) |
| Diagrammes | cas d'utilisation, diagramme de classes et modèle relationnel à jour ([`docs/diagrammes/`](docs/diagrammes/)) ; les diagrammes de séquence ne sont pas encore dans le dépôt |
| Maquettes | chaque application a son propre thème, défini dans son dossier |
| **Serveur : patients** | **fonctionne** : inscription par SMS, connexion, mot de passe oublié, verrouillage, NIP, limites du code SMS, profil du patient connecté (`GET /api/patients/moi`), sessions (jeton de 15 min, renouvellement, déconnexion d'un appareil ou de tous) |
| **Serveur : MUNASEB** | contrats et périodes d'adhésion, demandes de remboursement de bout en bout (création, prise en charge, validation, rejet, paiement), notifications, historique ; **routes des agents hors service** : elles reposent encore sur l'ancien compte agent, supprimé lors du passage aux comptes séparés (l'ancienne route de connexion agent est supprimée). Elles seront rebranchées à l'étape 3.9, mais **on ne pourra les tester qu'après les étapes 3.7, 3.8 et 3.10 réduites à la MUNASEB** (création et validation de la structure MUNASEB, invitation d'un agent), qui permettent de créer un agent. |
| **Serveur : backoffice** | base prête, routes à venir (voir ci-dessous) |
| Serveur : connexion professionnelle | **écrite** (`POST /api/professionnels/connexion`, puis `POST /api/professionnels/connexion/second-facteur` quand le second facteur est actif) : email et mot de passe, email obligatoirement vérifié, code TOTP ou code de secours, jeton intermédiaire à usage unique, session de 12 h au plus. **Pas encore testable dans Postman** : aucun compte professionnel ne peut être créé avant la création d'un établissement (étape 3.7) ; les tests automatiques la vérifient. |
| **Serveur : connexion administrateur** | **fonctionne** (`POST /api/administrateurs/connexion`, second temps, réenrôlement) : second facteur toujours exigé ; un administrateur sans second facteur ne peut en enregistrer un que pendant une fenêtre de 10 minutes ouverte par l'équipe. **Testable dans Postman** après l'outil du premier administrateur (voir « Créer le premier administrateur ») |
| **Outils de l'équipe** | **fonctionnent**, sur l'ordinateur du serveur seulement (jamais par le réseau) : `creer_premier_administrateur`, `creer_administrateur` (un administrateur en crée un autre), `desactiver_totp` (téléphone perdu) ; chaque création, reprise ou désactivation est tracée |
| Applications Angular et Flutter | en cours, chacune dans sa branche (voir « Organisation du dépôt ») |

**Le backoffice** est la partie qui gère les comptes et les établissements : création d'un établissement par son responsable, validation par l'équipe LaafiCare (licence), invitation des employés, administrateurs.
- **Ce qui existe déjà** : les tables dans la base (comptes séparés, établissements, pièces justificatives, décisions de l'équipe, affectations, invitations, profil administrateur) avec leurs règles, et la **double authentification** (second facteur TOTP : chiffrement du secret, activation, vérification des codes, codes de secours).
- **Ce qui marche déjà** : la connexion des administrateurs et les outils de l'équipe (premier administrateur, administrateurs suivants, désactivation d'un second facteur).
- **Ce qui n'est pas encore utilisable** : aucune route pour les établissements, les décisions, le choix de l'affectation ou les invitations (étapes 3.7 à 3.10). On ne peut donc encore ni créer un établissement, ni inviter un agent, ni se connecter comme professionnel.
- **Les détails** : les tables et leurs règles dans [`docs/modele-donnees.md`](docs/modele-donnees.md), la double authentification dans [`docs/securite.md`](docs/securite.md), les routes prévues dans [`docs/api.md`](docs/api.md) (section 4).

Les modules médicaux (dossier, ordonnances, pharmacie, laboratoire, suivi de grossesse) viendront ensuite. La liste détaillée des décisions encore ouvertes est dans [`docs/points-ouverts.md`](docs/points-ouverts.md).

## 6. Organisation du dépôt

Le code est conservé avec **Git**, un outil qui garde l'historique de chaque modification. Le dépôt a quatre *branches* (des versions parallèles du projet) :

| Branche | Contenu |
|---|---|
| `main` | la version en production. **Vide jusqu'à la première mise en production** : elle ne contient qu'un commit d'initialisation. |
| `dev` | le développement : le serveur (`backend/`), la documentation (`docs/`) et les fichiers de la racine. Les branches `web` et `mobile` y sont fusionnées par le porteur. |
| `web` | les applications web Angular, dans `web/`. |
| `mobile` | l'application mobile Flutter, dans `mobile/`. |

Pour travailler sur les trois à la fois, le porteur utilise trois *dossiers de travail* reliés au même dépôt (commande `git worktree`), chacun ouvert sur sa branche :

| Dossier | Branche | Ce qu'on y modifie |
|---|---|---|
| `LaafiCare` | `dev` | `backend/`, `docs/`, `README.md`, `CLAUDE.md` |
| `LaafiCare-web` | `web` | `web/` |
| `LaafiCare-mobile` | `mobile` | `mobile/` |

Contenu du dossier `LaafiCare` :

| Chemin | Contenu |
|---|---|
| `backend/` | le serveur Rust : code (`src/`), migrations de la base (`migrations/`), tests (`tests/`), configuration (`docker-compose.yml`, `.env.example`) |
| `docs/` | la documentation : routes (`api.md`), sécurité, modèle de données, points ouverts, diagrammes, cahier des charges |
| `CLAUDE.md` | le journal détaillé de toutes les décisions |

## 7. Installer, lancer et tester

Ces étapes concernent le serveur. Les commandes se tapent dans un terminal (PowerShell sous Windows). **Toutes les commandes `docker compose` et `cargo` se lancent depuis le dossier `backend/`**, où se trouve le fichier `docker-compose.yml`.

### Ce qu'il faut installer

- **Git**, pour récupérer le code.
- **Rust**, version 1.88 au moins, avec `rustup` (site officiel : rustup.rs). Vérification : `rustc --version`.
- **Docker Desktop**, qui fait tourner PostgreSQL sur votre machine sans l'installer directement.
- **Postman** (facultatif), pour envoyer des demandes au serveur à la main.

### Récupérer le code

Le dépôt est **privé** : il faut avoir reçu un accès du porteur.

```
git clone https://github.com/KABORE-c-elie/LaafiCare.git
cd LaafiCare
git checkout dev
```

### Préparer la configuration

1. Copier le modèle de configuration :
   ```
   cd backend
   copy .env.example .env
   ```
2. Ouvrir `backend/.env` et remplacer chaque `A_GENERER`. Le fichier explique comment générer chaque secret. Tant qu'un `A_GENERER` reste dans `DATABASE_URL`, `JWT_SECRET` ou `TOTP_CLE_CHIFFREMENT`, le serveur refuse de démarrer : c'est voulu. **Attention** : `POSTGRES_PASSWORD` est lu par Docker, pas par le serveur ; Docker accepterait `A_GENERER` comme mot de passe sans rien dire. Choisissez-le vous-même, et reprenez la même valeur dans `DATABASE_URL`.

| Variable | Rôle | Exemple **fictif** |
|---|---|---|
| `POSTGRES_PASSWORD` | mot de passe de la base de développement, lu par Docker | une valeur choisie |
| `DATABASE_URL` | adresse de la base, avec ce même mot de passe | `postgres://utilisateur:motdepasse@127.0.0.1:5433/laaficare` |
| `ENVIRONNEMENT` | `developpement` sur votre machine, `production` sur le vrai serveur ; obligatoire | `developpement` |
| `SERVER_PORT` | port du serveur | `8080` |
| `JWT_SECRET` | signe les jetons de connexion, 32 caractères au moins | 64 caractères hexadécimaux générés |
| `TOTP_CLE_CHIFFREMENT` | chiffre les secrets du second facteur, exactement 64 caractères hexadécimaux | une autre valeur générée |

Le fichier `.env` contient des secrets : il n'est jamais envoyé dans Git.

**Vous aviez déjà un `.env` ?** Ajoutez-y la ligne `ENVIRONNEMENT=developpement` (nouvelle variable, obligatoire) : sans elle, le serveur et les outils refusent de démarrer. En `production`, ils refusent aussi tant que les SMS et les emails sont seulement simulés, ce qui est le cas aujourd'hui.

### Lancer

1. **Démarrer Docker Desktop** (l'application, depuis le menu Démarrer) et attendre qu'il indique qu'il tourne. Sans lui, rien ne marche.
2. Depuis `backend/` :
   ```
   docker compose up -d     # démarre PostgreSQL en arrière-plan
   docker compose ps        # vérifie : la base doit apparaître « running »
   cargo run                # compile et lance le serveur (les outils : cargo run --bin <nom>)
   ```

**La première compilation prend plusieurs minutes** : Rust fabrique d'abord toutes les bibliothèques utilisées. Les suivantes sont bien plus rapides. Ce n'est pas un blocage.

Au démarrage, le serveur applique les migrations manquantes. Pour vérifier qu'il répond, ouvrir `http://127.0.0.1:8080/health` dans un navigateur (avec le port choisi) : la page affiche `"status":"ok"`.

Le serveur s'arrête avec `Ctrl+C` dans le terminal où il tourne. PostgreSQL s'arrête avec `docker compose down` ; les données sont gardées pour la prochaine fois.

> ⚠️ **`docker compose down -v` efface toute la base de développement**, comptes de test compris : identités, comptes patients, contrats, demandes, tout. Rien ne peut être récupéré. À n'utiliser que pour repartir volontairement d'une base vide.

### Un premier essai dans Postman

En développement, **aucun SMS n'est envoyé** : le code apparaît dans le terminal où tourne le serveur, sur une ligne contenant `SMS (dev, non envoyé réellement)` et `Votre code LaafiCare : …`. Utilisez un numéro **fictif**.

| Étape | Demande | Corps (JSON) | Réponse attendue |
|---|---|---|---|
| 1. Demander un code | `POST http://127.0.0.1:8080/api/patients/otp` | `{"telephone": "70 00 00 00"}` | 204, et le code dans le terminal du serveur |
| 2. S'inscrire | `POST http://127.0.0.1:8080/api/patients/inscription` | téléphone, code reçu, nom, prénom, date et lieu de naissance, mot de passe (voir `docs/api.md`, 2.2.2) | 201, avec `jeton_acces` et `jeton_renouvellement` |
| 3. Se connecter | `POST http://127.0.0.1:8080/api/patients/connexion` | `{"telephone": "70 00 00 00", "mot_de_passe": "…"}` | 200, avec `jeton_acces` et `jeton_renouvellement` |
| 4. Lire son profil | `GET http://127.0.0.1:8080/api/patients/moi`, avec l'en-tête `Authorization: Bearer <jeton_acces>` | aucun | 200 et le profil (NIP, nom, prénom…) |
| 5. Renouveler | `POST http://127.0.0.1:8080/api/sessions/renouveler` | `{"jeton_renouvellement": "…"}` | 200 et une **nouvelle** paire ; l'ancien jeton de renouvellement ne sert plus |
| 6. Se déconnecter | `POST http://127.0.0.1:8080/api/sessions/deconnexion` | `{"jeton_renouvellement": "…"}` (le dernier reçu) | 204 ; refaire l'étape 4 avec le même `jeton_acces` : 401 |

Le jeton d'accès expire après 15 minutes : au-delà, l'étape 4 répond 401, et l'étape 5 en donne un nouveau.

Toutes les routes, leurs réponses et leurs erreurs sont décrites dans [`docs/api.md`](docs/api.md).

### Créer le premier administrateur

Les comptes administrateur ne se créent jamais par le réseau : on lance un **outil** sur l'ordinateur du serveur. **Lancez-le depuis PowerShell** : les mots de passe et les codes se tapent sans s'afficher, ce que le terminal de Git Bash ne permet pas. En développement, **aucun email n'est envoyé** : il s'affiche dans le terminal de l'outil, entre deux lignes de tirets.

1. La personne doit d'abord avoir un **compte patient** : faire les étapes 1 et 2 du tableau précédent, avec un numéro fictif.
2. Le serveur doit avoir été lancé au moins une fois (il met la base à jour). Puis, depuis `backend/` :
   ```
   cargo run --bin creer_premier_administrateur
   ```
3. Répondre aux questions : le téléphone du compte patient, un email fictif (par exemple `admin@example.org`), la CNIB (facultative : « B » est déjà écrit, taper les 8 chiffres ou appuyer sur Entrée), puis deux fois le mot de passe administrateur (différent de celui du compte patient).
4. L'outil affiche l'email simulé, avec un **code de vérification** de 64 caractères : le copier, puis le coller quand l'outil le demande.
5. L'outil ouvre une **fenêtre de 10 minutes**. Pendant ce temps, dans Postman :

| Étape | Demande | Corps (JSON) | Réponse attendue |
|---|---|---|---|
| a. Se connecter | `POST http://127.0.0.1:8080/api/administrateurs/connexion` | `{"email": "admin@example.org", "mot_de_passe": "…"}` | 200, `"etape": "reenrolement"` et un `jeton_reenrolement` |
| b. Commencer | `POST …/api/administrateurs/reenrolement/commencer`, en-tête `Authorization: Bearer <jeton_reenrolement>` | `{"mot_de_passe": "…"}` | 200, `url_otpauth` et `cle_base32` |
| c. Enregistrer | dans une application d'authentification (Google Authenticator, par exemple) : ajouter un compte « par clé », et taper `cle_base32` | — | l'application affiche un code à 6 chiffres |
| d. Confirmer | `POST …/api/administrateurs/reenrolement/confirmer`, même en-tête | `{"code": "123456"}` (le code affiché) | 200 et **10 codes de secours**, montrés une seule fois : les noter |
| e. Se reconnecter | `POST …/api/administrateurs/connexion` | comme en a. | 200, `"etape": "second_facteur"` et un `jeton_intermediaire` |
| f. Donner le code | `POST …/api/administrateurs/connexion/second-facteur`, en-tête `Authorization: Bearer <jeton_intermediaire>` | `{"code": "…"}` (un nouveau code de l'application) | 200, `"etape": "complete"` et la paire de jetons |

Si l'outil s'arrête en cours, ou si les 10 minutes passent : relancer `cargo run --bin creer_premier_administrateur`. Il reprend (mot de passe administrateur demandé), renvoie un code si l'email n'est pas encore vérifié, puis rouvre une fenêtre.

**Les autres outils de l'équipe**, lancés par un administrateur qui donne son email, son mot de passe et un code de son application :
- `cargo run --bin creer_administrateur` : créer un autre administrateur (la personne est présente, a un compte patient, et tape elle-même son mot de passe), ou reprendre un administrateur qui n'a jamais enregistré de second facteur ;
- `cargo run --bin desactiver_totp` : retirer le second facteur d'une personne qui a perdu son téléphone et ses codes de secours, après avoir vérifié sa CNIB. Ses sessions sont fermées ; pour un administrateur, une nouvelle fenêtre de 10 minutes est ouverte.

**Gardez toujours au moins deux administrateurs avec un second facteur actif** : `desactiver_totp` exige un autre administrateur que la personne visée.

**Tester l'application mobile sur un téléphone Android branché en USB** : le serveur reste sur `127.0.0.1`, il n'est jamais ouvert au réseau local. L'outil `adb` (fourni avec Android Studio) relaie le port du téléphone vers celui de l'ordinateur :

```
adb reverse tcp:8080 tcp:8080
```

Le premier port est celui du téléphone, le second celui de l'ordinateur (aide de l'outil, `adb --help` : `reverse REMOTE LOCAL`). L'application sur le téléphone appelle alors `http://127.0.0.1:8080`, et la demande arrive au serveur de l'ordinateur. Le relais tombe quand on débranche le téléphone : relancer la commande à chaque branchement.

### Tester

Un *test* est un petit programme qui vérifie automatiquement qu'une règle est respectée. Depuis `backend/` :

```
cargo test                                   # tests sans base de données
cargo test --no-fail-fast -- --include-ignored   # tous les tests, PostgreSQL démarré
cargo test --lib -- --include-ignored totp        # seulement ceux dont le nom contient « totp »
```

`--no-fail-fast` fait tourner tous les groupes de tests même quand l'un d'eux échoue ; sans lui, Cargo s'arrête au premier groupe en échec et les suivants ne sont jamais lancés.

Les tests qui utilisent la base créent chacun une base temporaire, supprimée ensuite : la base de développement n'est jamais touchée.

**Échec attendu pour l'instant** : un seul test échoue, `extracteur_jwt::tests::agent_valide_puis_retire_en_base` (ancien contrôle de l'agent MUNASEB, réécrit à l'étape 3.9). Tout autre échec est un vrai problème.

### En cas de problème

| Ce qu'on voit | Cause | Que faire |
|---|---|---|
| Les tests en base échouent tous avec `PoolTimedOut` | PostgreSQL ne répond pas : Docker Desktop n'est pas démarré | démarrer Docker Desktop, puis `docker compose up -d` et `docker compose ps` |
| Le serveur s'arrête aussitôt avec « configuration invalide » | il reste un `A_GENERER` dans `backend/.env`, une valeur mal formée (secret trop court, clé qui n'est pas en hexadécimal), ou `ENVIRONNEMENT` manque | compléter `.env` (voir « Préparer la configuration ») |
| « envoi de SMS simulé : refusé en production » | `ENVIRONNEMENT=production` sur une machine de développement | mettre `ENVIRONNEMENT=developpement` |
| Un outil répond « base pas à jour » | le serveur n'a pas encore appliqué les dernières migrations | lancer `cargo run` une fois, puis relancer l'outil |
| Un outil s'arrête quand il demande un mot de passe | il est lancé depuis le terminal de Git Bash | le relancer depuis PowerShell |
| Le serveur s'arrête car la base est injoignable | Docker n'est pas démarré, ou le mot de passe de `DATABASE_URL` n'est pas celui de `POSTGRES_PASSWORD` | démarrer Docker ; mettre le même mot de passe dans les deux variables |
| Réponse 429 `trop_de_demandes` | trop de codes demandés, ou trop d'essais de code | attendre le nombre de secondes indiqué par l'en-tête `Retry-After`, ou demander un nouveau code |
| Réponse 401 `jeton_invalide` sur une route ordinaire | jeton d'accès expiré (15 min), d'une autre sorte, émis avant un changement de mot de passe, ou session fermée | renouveler (`POST /api/sessions/renouveler`) ; si le renouvellement répond aussi 401, se reconnecter |

## 8. Pour aller plus loin

- [`docs/securite.md`](docs/securite.md) : chaque choix de sécurité, sa source et son fichier.
- [`docs/modele-donnees.md`](docs/modele-donnees.md) : les tables et leurs règles.
- [`docs/api.md`](docs/api.md) : les routes du serveur, avec des exemples ; c'est le contrat sur lequel s'appuient les applications web et mobile.
- [`docs/points-ouverts.md`](docs/points-ouverts.md) : les décisions encore à prendre, du plus urgent au moins urgent.
- [`CLAUDE.md`](CLAUDE.md) : le journal détaillé de toutes les décisions du projet.

## 9. Sources

- **NIST SP 800-63B, révision 4** (août 2025) : mots de passe, OTP, second facteur, limitation des essais.
- **OWASP** : *Password Storage Cheat Sheet* (Argon2id et ses paramètres), *File Upload Cheat Sheet* (pièces justificatives), *Multifactor Authentication Cheat Sheet* (limites d'envoi des codes), *Forgot Password Cheat Sheet* (réinitialisation par email, à venir).
- **RFC 4226** (HOTP) et **RFC 6238** (TOTP) : codes à usage unique et second facteur.
- **RFC 7518** §3.2 : taille minimale du secret de signature des jetons.
- **RFC 8725** §3.12 : sortes de jetons séparées. **RFC 6750** §3.1 : refus d'un jeton invalide ou révoqué. **RFC 6585** §4 : réponse 429 et en-tête `Retry-After`.
- **WCAG 2.2** (W3C, 12 décembre 2024), critère 1.4.1 : la couleur ne porte jamais seule une information.
- **Loi n° 001-2021/AN du 30 mars 2021** portant protection des personnes à l'égard du traitement des données à caractère personnel (Burkina Faso) : formalités auprès de la CIL, hébergement des données de santé (voir `docs/points-ouverts.md`).
- **UIT**, plan national de numérotage du Burkina Faso (communication de l'ARCEP du 4 mai 2023) : format des numéros de téléphone.
- **Documentation PostgreSQL 18** et **documentation Git** (gitattributes, gitignore).
- Documentation officielle des bibliothèques Rust utilisées : `axum`, `sqlx`, `argon2`, `jsonwebtoken`, `totp-rs`, `aes-gcm`, `unicode-normalization`, `rand`.
- **Carnet de santé mère-enfant** du Ministère de la Santé du Burkina Faso : calendrier des CPN, seuils d'alerte.
- **Mémoire sur la plateforme MUNASEB** : circuit d'un remboursement d'assurance.

## 10. Auteur

**Calliste Elie KABORE**, porteur du projet, maîtrise d'ouvrage et maîtrise d'œuvre.
