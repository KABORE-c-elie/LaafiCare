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

**Le serveur résiste aux abus.** Une demande ne peut pas dépasser 256 Kio (5 Mio pour une pièce justificative) ; les calculs de mots de passe se font à part, un par cœur au plus, sans bloquer les autres utilisateurs ; chaque réponse porte un numéro (`X-Request-ID`) qui permet de retrouver une erreur dans les journaux, où n'apparaît aucune donnée personnelle (ni adresse complète, ni numéro envoyé par l'appelant).

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
| **Serveur : MUNASEB** | contrats et périodes d'adhésion, demandes de remboursement de bout en bout (création, prise en charge, validation, rejet, paiement), notifications, historique. **Routes des agents rebranchées** (étape 3.9) : un agent se connecte comme professionnel, active son affectation d'agent MUNASEB, et chacune de ses décisions est enregistrée avec cette affectation. **Testables dans Postman de bout en bout** (création et validation de la structure MUNASEB, invitation d'un agent, puis contrats). **Contrats côté agent** (étape 5) : adhésion, renouvellement, recherche par numéro de carte, paramètres de la mutuelle avec historique. |
| Serveur : affectations | **écrites** : liste des affectations d'un professionnel, activation (choisir l'établissement et le rôle dans lesquels il travaille), changement d'établissement sans redonner le mot de passe ; à chaque demande, le serveur vérifie que l'affectation est active et l'établissement validé, sous licence |
| **Serveur : backoffice** | **création d'un établissement MUNASEB** (étape 3.7, réduite) : un patient crée la structure, devient son responsable, crée au passage son compte professionnel, fait vérifier son email par un lien, puis soumet sa demande ; suivi de ses demandes. **Décisions de l'équipe** (étape 3.8, réduite) : liste et détail des demandes, validation avec une licence (sans fin, ou jusqu'à une date), refus avec motif ; un administrateur ne décide jamais sur sa propre structure. **Invitations** (étape 3.10, réduite) : le responsable invite un agent par son numéro (SMS), la personne accepte ou refuse depuis son compte patient ; annulation, expiration après 7 jours |
| Serveur : connexion professionnelle | **fonctionne** (`POST /api/professionnels/connexion`, puis `POST /api/professionnels/connexion/second-facteur` quand le second facteur est actif) : email et mot de passe, email obligatoirement vérifié, code TOTP ou code de secours, jeton intermédiaire à usage unique, session de 12 h au plus. Un compte professionnel naît quand un patient crée son établissement ; son email se vérifie par un lien (renvoyable une fois par minute). **Mot de passe oublié** (étape 3.12) : un lien valable 30 minutes, envoyé à l'email vérifié, pour les professionnels et les administrateurs ; toutes les sessions sont fermées après le changement, et une alerte part par email. |
| **Serveur : connexion administrateur** | **fonctionne** (`POST /api/administrateurs/connexion`, second temps, réenrôlement) : second facteur toujours exigé ; un administrateur sans second facteur ne peut en enregistrer un que pendant une fenêtre de 10 minutes ouverte par l'équipe. **Testable dans Postman** après l'outil du premier administrateur (voir « Créer le premier administrateur ») |
| **Outils de l'équipe** | **fonctionnent**, sur l'ordinateur du serveur seulement (jamais par le réseau) : `creer_premier_administrateur`, `creer_administrateur` (un administrateur en crée un autre), `desactiver_totp` (téléphone perdu), `intervenir_compte` (compte compromis : retirer un accès, bloquer, réactiver), `importer_catalogue` (catalogue national des médicaments) ; chaque action est tracée |
| Applications Angular et Flutter | en cours, chacune dans sa branche (voir « Organisation du dépôt ») |

**Le backoffice** est la partie qui gère les comptes et les établissements : création d'un établissement par son responsable, validation par l'équipe LaafiCare (licence), invitation des employés, administrateurs.
- **Ce qui existe déjà** : les tables dans la base (comptes séparés, établissements, pièces justificatives, décisions de l'équipe, affectations, invitations, profil administrateur) avec leurs règles, et la **double authentification** (second facteur TOTP : chiffrement du secret, activation, vérification des codes, codes de secours).
- **Ce qui marche déjà** : la connexion des administrateurs et les outils de l'équipe (premier administrateur, administrateurs suivants, désactivation d'un second facteur) ; la création et la soumission d'un établissement MUNASEB, avec la vérification de l'email du responsable ; la validation ou le refus de la demande par l'équipe (routes administrateur, avec second facteur).
- **Ce qui marche aussi** : le parcours complet de la MUNASEB, de la création à l'agent qui traite les remboursements (structure créée, validée, agent invité, invitation acceptée, email vérifié, connexion, affectation activée).
- **Ce qui marche encore** : l'abandon d'une demande en brouillon ou refusée par son créateur ; la liste des membres d'un établissement et le retrait de l'accès d'un agent ou d'un autre responsable par un responsable (ses sessions sur cet établissement se ferment aussitôt, et un email le prévient).
- **Espace patient MUNASEB** (étape 6) : le patient voit ses droits (couvert, en carence, expiré, suspendu, avec le solde de sa période), ses demandes de remboursement et leur détail (motif d'un rejet compris), et ses notifications, qu'il peut marquer comme lues. Il ne voit jamais que ses propres données.
- **Contrats MUNASEB** (étape 5) : un agent enregistre l'adhésion d'un étudiant (NIP et date de naissance, numéro de carte, paiement en espèces ou par mobile money), retrouve un contrat par son numéro de carte, le renouvelle chaque année (nouveau numéro de carte, délai de grâce de 10 jours), et lit les valeurs de la mutuelle (cotisation, fiche, délai de grâce, carence, plafond), que seul le **responsable** de la MUNASEB peut changer, chaque changement étant gardé dans un historique. Après 5 adhésions sans patient en 15 minutes, un agent doit attendre avant de réessayer. Plus besoin de SQL pour créer un contrat.
- **Pharmacies** (itération ouverte le 2026-11-16) : un patient crée sa pharmacie, dépose son autorisation d'ouverture et d'exploitation (PDF, JPEG ou PNG, 5 Mo au plus), puis la soumet ; l'équipe LaafiCare consulte la pièce et valide ; le responsable invite ses pharmaciens. **Stock** : le pharmacien enregistre l'arrivée d'un lot (numéro, date d'expiration, prix d'achat), vend au comptoir (le lot qui périme le plus tôt sort d'abord, un lot périmé ne se vend jamais), déclare les pertes, fait un inventaire rapide, et voit ses alertes (stock sous le seuil, lots bientôt périmés) et les médicaments de même molécule disponibles. Rien ne s'efface : une erreur se corrige par une annulation, tracée. Le **responsable** fixe les prix de vente et le délai d'alerte de péremption. Une requête renvoyée après une coupure ne crée rien deux fois. **Bénéfice net** : le responsable saisit ses dépenses (loyer, salaires, électricité, eau, transport, impôts et taxes, entretien, autres) et voit, par semaine, mois, année ou entre deux dates, ses ventes, la marge brute, les pertes, les dépenses, le bénéfice net et la valeur du stock restant (un tableau de gestion, pas la comptabilité officielle). **Le catalogue des médicaments reste vide** tant que la liste nationale officielle n'est pas déposée (outil `importer_catalogue`).
- **Auxiliaire de pharmacie** (2026-11-17) : le responsable d'une pharmacie peut inviter un auxiliaire. Il enregistre les entrées, les ventes, les pertes et les inventaires, mais ne voit jamais les prix d'achat, les dépenses ni le bénéfice.
- **Module médecin** (ouvert le 2026-11-17) : un patient peut créer une **clinique** (avec ses deux autorisations du Ministère de la Santé) ; son responsable invite des médecins, des infirmiers (sages-femmes comprises) et des agents d'accueil. Chaque professionnel remplit son **numéro d'ordre** et sa **spécialité**, que l'équipe LaafiCare contrôle. Le patient peut indiquer son sexe (facultatif).
- **Ce qui n'est pas encore utilisable** : la suspension d'un contrat, les ayants droit et les tarifs selon le circuit de soins (en attente des réponses de la MUNASEB, voir [`docs/points-ouverts.md`](docs/points-ouverts.md)). Les hôpitaux et laboratoires attendent la liste de leurs pièces justificatives.
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
| `DATABASE_URL` | adresse de la base avec le rôle **propriétaire** et ce même mot de passe ; sert seulement à la commande `migrer` et aux tests | `postgres://laaficare:motdepasse@127.0.0.1:5433/laaficare` |
| `DATABASE_URL_SERVEUR` | adresse de la base pour le **serveur**, avec le rôle `laaficare_serveur` (voir « Rôles de la base » ci-dessous) | `postgres://laaficare_serveur:autremotdepasse@127.0.0.1:5433/laaficare` |
| `DATABASE_URL_OUTILS` | adresse de la base pour les **outils** de l'équipe, avec le rôle `laaficare_outils` | `postgres://laaficare_outils:encoreunautre@127.0.0.1:5433/laaficare` |
| `ENVIRONNEMENT` | `developpement` sur votre machine, `production` sur le vrai serveur ; obligatoire | `developpement` |
| `URL_APPLICATION_WEB` | adresse de l'application web, sans `/` final : les liens envoyés par email y mènent ; obligatoire ; `https://` exigé en production | `http://localhost:4200` |
| `CONTACT_SECURITE` | adresse email de l'équipe, donnée dans les alertes de sécurité (second facteur enregistré ou désactivé) ; obligatoire ; en production, une vraie adresse (les adresses d'exemple sont refusées) | `securite@example.org` |
| `SERVER_PORT` | port du serveur | `8080` |
| `SERVER_HOST` | facultatif : adresse d'écoute ; `127.0.0.1` si absente (joignable depuis cette machine seulement) ; `0.0.0.0` seulement en production, derrière un proxy HTTPS | `127.0.0.1` |
| `DATABASE_MAX_CONNECTIONS`, `DATABASE_MIN_CONNECTIONS`, `DATABASE_ACQUIRE_TIMEOUT_SECS`, `DATABASE_IDLE_TIMEOUT_SECS`, `DATABASE_MAX_LIFETIME_SECS` | facultatives : taille du pool de connexions du serveur et ses délais (5, 0, 5 s, 10 min, 30 min si absentes) ; une valeur fausse empêche le démarrage | `5` |
| `JWT_SECRET` | signe les jetons de connexion, 32 caractères au moins | 64 caractères hexadécimaux générés |
| `TOTP_CLE_CHIFFREMENT` | chiffre les secrets du second facteur, exactement 64 caractères hexadécimaux | une autre valeur générée |

Le fichier `.env` contient des secrets : il n'est jamais envoyé dans Git.

**Vous aviez déjà un `.env` ?** Ajoutez-y les lignes `ENVIRONNEMENT=developpement`, `URL_APPLICATION_WEB=http://localhost:4200` et `CONTACT_SECURITE=securite@example.org` (variables obligatoires), ainsi que `DATABASE_URL_SERVEUR` et `DATABASE_URL_OUTILS` (voir « Rôles de la base ») : sans elles, le serveur et les outils refusent de démarrer. En `production`, ils refusent aussi tant que les SMS et les emails sont seulement simulés, ce qui est le cas aujourd'hui.

**Votre base de développement contient déjà une structure ?** La migration 0022 rend obligatoires le numéro d'urgence, la région et l'email d'une structure : elle refuse de s'appliquer sur une base qui contient déjà une structure, et le serveur ne démarre pas. Repartez alors d'une base vide (`docker compose down -v`, qui **efface toute la base de développement**), puis relancez `docker compose up -d` et `cargo run --bin migrer`.

### Rôles de la base

Le serveur ne se connecte pas à la base avec tous les droits. Il utilise un **rôle limité**, `laaficare_serveur` : il peut lire et écrire les données, mais il ne peut ni créer ou modifier des tables, ni effacer les traces (historique des demandes, décisions de l'équipe…), ni désactiver les protections de la base. Les outils de l'équipe utilisent un second rôle, `laaficare_outils`. Seule la commande `migrer` (voir « Lancer ») utilise le rôle propriétaire, qui a tous les droits.

**Créer les rôles, une seule fois.** Sur une base toute neuve, Docker les crée tout seul au premier démarrage. Sur une base qui existait déjà, lancer depuis `backend/`, Docker démarré :
```
docker exec -i laaficare-db psql -U laaficare -d laaficare -f - < sql/roles.sql
```
Relancer cette commande ne casse rien : un rôle qui existe déjà est laissé tel quel.

**Donner un mot de passe à chaque rôle.** Ouvrir la console de la base :
```
docker exec -it laaficare-db psql -U laaficare -d laaficare
```
puis taper, une ligne après l'autre :
```
\password laaficare_serveur
\password laaficare_outils
\q
```
Chaque `\password` demande le mot de passe deux fois, **sans l'afficher**. Choisissez deux mots de passe différents, puis reportez-les dans `DATABASE_URL_SERVEUR` et `DATABASE_URL_OUTILS` du fichier `.env`.

### Lancer

1. **Démarrer Docker Desktop** (l'application, depuis le menu Démarrer) et attendre qu'il indique qu'il tourne. Sans lui, rien ne marche.
2. Depuis `backend/` :
   ```
   docker compose up -d     # démarre PostgreSQL en arrière-plan
   docker compose ps        # vérifie : la base doit apparaître « running »
   cargo run --bin migrer   # met la base à jour
   cargo run                # compile et lance le serveur (les outils : cargo run --bin <nom>)
   ```

**`cargo run --bin migrer`** applique à la base les changements de structure du projet (les « migrations », dans `backend/migrations/`). Elle se connecte avec le rôle propriétaire (`DATABASE_URL`), le seul qui en a le droit. Lancez-la **après chaque récupération du code** (`git pull`) qui apporte une nouvelle migration, puis démarrez le serveur. La relancer quand tout est déjà à jour ne fait rien de plus. Le serveur, lui, **refuse de démarrer sur une base en retard** et l'écrit dans le terminal (« base pas à jour : lancer `cargo run --bin migrer` »).

**La première compilation prend plusieurs minutes** : Rust fabrique d'abord toutes les bibliothèques utilisées. Les suivantes sont bien plus rapides. Ce n'est pas un blocage.

Au démarrage, le serveur vérifie que la base est à jour. Pour vérifier qu'il répond, ouvrir `http://127.0.0.1:8080/health` dans un navigateur (avec le port choisi) : la page affiche `"status":"ok"`.

Le serveur s'arrête avec `Ctrl+C` dans le terminal où il tourne (ou le signal d'arrêt de Docker ou de systemd) : il n'accepte plus de nouvelle demande, laisse finir celles en cours, ferme ses connexions à la base, puis écrit « serveur arrêté proprement ». PostgreSQL s'arrête avec `docker compose down` ; les données sont gardées pour la prochaine fois.

> ⚠️ **`docker compose down -v` efface toute la base de développement**, comptes de test compris : identités, comptes patients, contrats, demandes, tout. Rien ne peut être récupéré. À n'utiliser que pour repartir volontairement d'une base vide.

### Un premier essai dans Postman

On peut aussi tester par les applications elles-mêmes : l'application web Angular (branche `web`, qui passe par le proxy de développement d'Angular, sans réglage CORS côté serveur) et l'application mobile Flutter (branche `mobile`).

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

### Créer un établissement MUNASEB

À la suite du tableau précédent, avec le `jeton_acces` du patient (en-tête `Authorization: Bearer <jeton_acces>`). En développement, **aucun email n'est envoyé** : il s'affiche dans le terminal du serveur, avec un lien de la forme `http://localhost:4200/verifier-email#jeton=…`.

| Étape | Demande | Corps (JSON) | Réponse attendue |
|---|---|---|---|
| 1. Créer | `POST http://127.0.0.1:8080/api/structures` | type `munaseb`, nom, deux numéros, email de la structure, région, province, commune, latitude et longitude au Burkina Faso, un email professionnel **fictif** et un mot de passe professionnel (exemple complet : `docs/api.md`, 2.8) | 201, `"statut": "brouillon"`, `"email_a_verifier": true`, et l'email dans le terminal du serveur |
| 2. Soumettre trop tôt | `POST …/api/structures/<structure_id>/soumettre` | aucun | 403 `email_non_verifie` |
| 3. Vérifier l'email | `POST …/api/professionnels/email/verifier` (sans jeton d'accès) | `{"jeton": "…"}` : les 64 caractères après `#jeton=` dans le lien | 204 (la page web le fera elle-même) |
| 4. Soumettre | même demande qu'à l'étape 2 | aucun | 204 |
| 5. Suivre | `GET …/api/structures/mes-demandes` | aucun | 200, la demande `"statut": "en_attente"` |
| 6. Se connecter comme professionnel | `POST …/api/professionnels/connexion` | l'email et le mot de passe professionnels | 200, `"etape": "complete"` ; la liste des affectations montre `responsable`, non exerçable avant la validation |
| 7. Faire valider | avec un **autre** compte que celui du responsable : un administrateur (voir « Créer le premier administrateur »), connecté avec son second facteur, envoie `POST …/api/administrateurs/structures/<structure_id>/valider` | `{}` (licence sans fin) ou `{"licence_expire_le": "AAAA-MM-JJ"}` | 204 ; un email simulé « demande traitée » s'affiche dans le terminal ; l'affectation `responsable` devient exerçable |
| 8. Inviter un agent | le responsable se connecte, active son affectation `responsable` (`POST …/api/professionnels/affectations/<id>/activer`), puis `POST …/api/professionnels/invitations` | `{"telephone": "<numéro fictif d'un autre compte patient>", "role": "agent_assurance_munaseb"}` | 201 ; le SMS d'invitation s'affiche dans le terminal du serveur |
| 9. Accepter | connecté avec le compte patient de l'invité : `GET …/api/patients/invitations`, puis `POST …/api/patients/invitations/<id>/accepter` | `{"email_professionnel": "<email fictif>", "mot_de_passe_professionnel": "…"}` | 200 ; un lien de vérification s'affiche dans le terminal (le vérifier comme à l'étape 3) ; l'agent peut ensuite se connecter et activer son affectation |
| 10. Retirer un accès | le responsable, son affectation `responsable` active : `GET …/api/professionnels/membres`, puis `POST …/api/professionnels/membres/<affectation_id>/desactiver` | aucun | 200 avec la liste (sans téléphone ni email), puis 204 ; un email simulé « accès retiré » s'affiche dans le terminal ; le jeton d'accès de l'agent reçoit ensuite 401 `jeton_invalide`. Retirer le dernier responsable donne 409 `dernier_responsable` |

Abandonner une demande en brouillon ou refusée : `POST …/api/structures/<structure_id>/abandonner` avec le jeton patient du créateur (204) ; la personne peut ensuite en créer une nouvelle.

### Ouvrir une pharmacie

Même parcours que la MUNASEB ci-dessus (étapes 1 à 9), avec `"type": "pharmacie"` à la création, et une pièce à déposer **avant de soumettre** :

| Étape | Demande | Corps | Réponse attendue |
|---|---|---|---|
| 1. Soumettre trop tôt | `POST …/api/structures/<structure_id>/soumettre` | aucun | 409 `pieces_manquantes`, avec `["autorisation_ouverture_exploitation"]` |
| 2. Déposer l'autorisation | `POST …/api/structures/<structure_id>/pieces/autorisation_ouverture_exploitation?numero=2025-001&date_piece=2025-01-10` | dans Postman, **Body → binary**, un petit PDF **fictif** | 201 avec le type reconnu (`application/pdf`) et l'empreinte |
| 3. Mauvais fichier | même demande avec un fichier texte | — | 415 `format_refuse` |
| 4. Soumettre | même demande qu'à l'étape 1 | aucun | 204 |
| 5. Vérifier la pièce (équipe) | `GET …/api/administrateurs/structures/<structure_id>`, puis `GET …/api/administrateurs/pieces/<piece_id>/fichier` | aucun | la pièce dans `pieces`, puis le fichier téléchargé |
| 6. Valider, inviter | comme pour la MUNASEB, avec `"role": "pharmacien"` | — | 204, puis 201 |

### Tenir le stock d'une pharmacie

Il faut d'abord **un catalogue de test**, avec des médicaments **fictifs**. L'outil `importer_catalogue` lit un **fichier CSV**, jamais un PDF : la liste officielle (un PDF) est recopiée dans un tableur, relue, puis enregistrée au format CSV.

**Format exact du fichier CSV** :

| Règle | Détail |
|---|---|
| Encodage | UTF-8 (dans un tableur : « CSV UTF-8 ») |
| Séparateur | point-virgule `;` (celui des tableurs réglés en français) ; une virgule est refusée |
| Guillemets | aucun : une ligne avec `"` est refusée, avec son numéro |
| Première ligne | exactement `denomination;dci;forme;dosage` |
| Colonnes | `denomination` : nom du médicament (200 caractères au plus) ; `dci` : la molécule, ou plusieurs séparées par `+` pour une association (200 au plus chacune) ; `forme` : comprimé, sirop… (100 au plus) ; `dosage` : 500 mg, 1 g… (100 au plus) |
| Lignes | une par médicament, 20 000 au plus ; les lignes vides sont ignorées |
| Doublons | une ligne déjà présente (même nom, forme et dosage, sans tenir compte des majuscules ni des espaces en trop) est ignorée |
| Résultat | tout ou rien : à la première ligne invalide, rien n'est importé et l'outil donne le numéro de la ligne |

Exemple, `catalogue-fictif.csv` :

```
denomination;dci;forme;dosage
Produit X fictif;Molécule-A fictive;comprimé;500 mg
Produit Y fictif;Molécule-A fictive;comprimé;500 mg
Produit W fictif;Molécule-A fictive;comprimé;1 g
```

puis, depuis `backend/`, lancer `cargo run --bin importer_catalogue` : l'outil demande l'email, le mot de passe et un code d'un administrateur, le chemin du fichier, la source (écrire « Liste fictive de test »), puis confirme. **Ne jamais importer de vraie liste avant le dépôt du fichier officiel** dans `docs/Reglementation, loi et reference/`.

Ensuite, le pharmacien (ou le responsable) se connecte, active son affectation de la pharmacie, et, avec ce `jeton_acces` (exemples complets : `docs/api.md`, 2.15) :

| Étape | Demande | Corps | Réponse attendue |
|---|---|---|---|
| 1. Trouver le médicament | `GET …/api/pharmacie/medicaments?q=produit` | aucun | 200, les trois produits avec leur `id` |
| 2. Vendre sans prix | `POST …/api/pharmacie/ventes` | `{"sur_ordonnance_papier": false, "lignes": [{"medicament_id": "<id X>", "quantite": 1}]}` | 409 `prix_non_defini` |
| 3. Fixer le prix (pharmacien) | `PUT …/api/pharmacie/prix/<id X>` | `{"prix_unitaire_fcfa": 250}` | 403 `acces_refuse` : seul le responsable fixe les prix |
| 4. Fixer le prix (responsable) | même demande avec le jeton du responsable | même corps | 200 |
| 5. Entrée d'un lot | `POST …/api/pharmacie/entrees`, en-tête `Idempotency-Key: entree-1` | `{"medicament_id": "<id X>", "numero_lot": "LOT-1", "date_expiration": "<dans 2 mois>", "quantite": 10, "prix_achat_unitaire_fcfa": 100, "fournisseur": "Grossiste fictif"}` | 201, le lot (`quantite` 10) |
| 6. Renvoyer la même entrée | exactement la même demande, même clé | même corps | 201, **le même lot** (même `id`) : rien n'est créé deux fois |
| 7. Vendre | même demande qu'à l'étape 2 | même corps | 201, la vente, `total_fcfa` 250 |
| 8. Voir le stock | `GET …/api/pharmacie/stock` | aucun | 200, `quantite_disponible` 9 |
| 9. Voir le journal | `GET …/api/pharmacie/mouvements` | aucun | 200, l'entrée et la vente, avec le nom de l'auteur |
| 10. Médicaments de même molécule | `GET …/api/pharmacie/medicaments/<id X>/equivalents` | aucun | 200 : d'abord le produit Y (`"correspondance": "meme_dci_meme_dosage_meme_forme"`), puis le produit W, marqué `"meme_dci_dosage_ou_forme_different"` et `"meme_dosage": false` (1 g au lieu de 500 mg) |
| 10 bis. Boîte au mois seul | `POST …/api/pharmacie/entrees` avec `"mois_expiration": "<AAAA-MM, dans 3 mois>"` au lieu de `date_expiration` | — | 201 : `date_expiration` est le dernier jour du mois **précédent**, `mois_expiration_imprime` le mois de la boîte |
| 11. Alertes | `PUT …/api/pharmacie/seuils/<id X>` avec `{"seuil_min": 20, "seuil_max": null}`, puis `GET …/api/pharmacie/alertes` | — | 204, puis le produit X dans `sous_seuil_min` |
| 12. Dépense (pharmacien) | `POST …/api/pharmacie/depenses` | `{"date_depense": "<aujourd'hui>", "categorie": "loyer", "montant_fcfa": 50000, "libelle": "Loyer fictif"}` | 403 `acces_refuse` : réservé au responsable |
| 13. Dépense (responsable) | même demande avec le jeton du responsable | même corps | 201 |
| 14. Bénéfice net | `GET …/api/pharmacie/rapport?periode=semaine` (jeton du responsable) | aucun | 200 : ventes 250, coût 100, marge brute 150, dépenses 50 000, bénéfice net −49 850, stock restant 900 |

### Enregistrer un contrat MUNASEB (agent)

L'agent invité à l'étape 9 ci-dessus se connecte (`POST …/api/professionnels/connexion`), active son affectation `agent_assurance_munaseb`, puis, avec ce `jeton_acces` (exemples complets : `docs/api.md`, 2.14) :

| Étape | Demande | Corps | Réponse attendue |
|---|---|---|---|
| 1. Voir les valeurs de la mutuelle | `GET …/api/assurance-munaseb/parametres` | aucun | 200 : cotisation 5 000, fiche 100, délai de grâce 10 jours, carence 1 mois, plafond 100 000 |
| 2. Adhésion | `POST …/api/assurance-munaseb/contrats` | le NIP d'un compte patient **fictif** (`GET …/api/patients/moi` le donne), sa date de naissance, un numéro de carte (ex. `0001/2026`), l'UFR, l'université, le matricule, et `"paiement": {"mode": "especes"}` | 201 avec le contrat, `"etat": "en_carence"` : la couverture commence dans un mois |
| 3. Mauvaise date de naissance | même demande avec une autre date | — | 404 `patient_inconnu` (jamais « NIP inconnu » ni « date fausse » séparément) |
| 4. Retrouver par la carte | `GET …/api/assurance-munaseb/contrats?numero_carte=0001/2026` | aucun | 200, le même contrat, sans NIP ni téléphone |
| 5. Renouveler trop tôt | `POST …/api/assurance-munaseb/contrats/<id>/renouvellements` | `{"numero_carte": "0001/2027", "paiement": {"mode": "mobile_money", "reference_transaction": "OM-0001"}}` | 409 `periode_future_deja_payee` pendant la carence |
| 6. Changer une valeur (agent) | `PUT …/api/assurance-munaseb/parametres` | les cinq valeurs, l'une changée | 403 `acces_refuse` : seul le responsable modifie les valeurs |
| 7. Changer une valeur (responsable) | le responsable de l'étape 8 ci-dessus, son affectation `responsable` active : même demande | les cinq valeurs, l'une changée | 200 ; `GET …/parametres/historique` montre l'ancienne et la nouvelle version, avec le nom du responsable |
| 8. Trop d'essais | 5 adhésions avec une date de naissance fausse, puis une 6ᵉ | — | 404 `patient_inconnu` cinq fois, puis 429 `trop_de_demandes` avec l'en-tête `Retry-After` (secondes), même avec la bonne date, pendant 15 minutes au plus |

Pour tester un adhérent **déjà couvert** (demande de remboursement avec `simuler-acte`), la base de développement accepte encore une période posée en SQL, à condition de lui donner un numéro de carte (`numero_carte`) : la route d'adhésion commence toujours par un mois de carence.

### Voir ses droits et ses demandes (espace patient MUNASEB)

Avec le `jeton_acces` d'un compte patient (en-tête `Authorization: Bearer <jeton_acces>`) :

| Demande | Réponse attendue |
|---|---|
| `GET …/api/patients/munaseb/droits` | 200, `"etat": "aucun_contrat"` tant que le patient n'a pas de contrat ; `"etat": "en_carence"` juste après l'adhésion par un agent (voir ci-dessus) ; ensuite `"etat": "couvert"` avec le plafond, la somme consommée et le solde |
| `GET …/api/patients/munaseb/demandes` | 200, les demandes du patient, les plus récentes d'abord (après une demande créée par un agent avec `simuler-acte` et le NIP du patient) |
| `GET …/api/patients/munaseb/demandes/<id>` | 200 avec le détail ; 404 `demande_inconnue` pour la demande d'un autre patient |
| `GET …/api/patients/notifications` | 200, avec `non_lues` |
| `POST …/api/patients/notifications/<id>/lue` | 204 ; `non_lues` diminue de 1 |

Lien perdu ou expiré (24 h) : `POST …/api/professionnels/email/renvoyer` avec `{"email": "…", "mot_de_passe": "…"}` (mot de passe professionnel), une fois par minute au plus.

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
- `cargo run --bin desactiver_totp` : retirer le second facteur d'une personne qui a perdu son téléphone et ses codes de secours, après avoir vérifié sa CNIB. Ses sessions sont fermées ; pour un administrateur, une nouvelle fenêtre de 10 minutes est ouverte ;
- `cargo run --bin intervenir_compte` : agir sur un compte **compromis** (mot de passe volé, accès qui ne devrait plus exister). L'outil propose trois actions :
  1. **retirer une affectation** d'un professionnel : il perd aussitôt l'accès à cette structure. Le dernier responsable d'une structure peut être retiré, après un avertissement et une confirmation ;
  2. **bloquer un compte** (patient, professionnel ou administrateur) : toutes ses sessions sont fermées, son mot de passe ne marche plus, il ne reçoit plus aucun code SMS ni lien par email ;
  3. **réactiver un compte bloqué**, après avoir vérifié la CNIB de la personne devant soi : elle choisit ensuite un nouveau mot de passe avec « Mot de passe oublié ? ».

  Chaque fois, l'outil affiche la personne trouvée et demande de confirmer, puis un motif. Chaque action est tracée, sans pouvoir être modifiée ; la personne est prévenue par email si elle en a un vérifié. Un administrateur ne peut agir sur aucun de ses propres comptes.

- `cargo run --bin importer_catalogue` : ajouter des médicaments au catalogue national, depuis un fichier tiré d'un document officiel (voir « Tenir le stock d'une pharmacie »). Rien ne peut ensuite être effacé ;

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

**Tous les tests doivent passer** : depuis l'étape 3.9, aucun échec n'est attendu. Un test qui échoue est un vrai problème.

### En cas de problème

| Ce qu'on voit | Cause | Que faire |
|---|---|---|
| Les tests en base échouent tous avec `PoolTimedOut` | PostgreSQL ne répond pas : Docker Desktop n'est pas démarré | démarrer Docker Desktop, puis `docker compose up -d` et `docker compose ps` |
| Le serveur s'arrête aussitôt avec « configuration invalide » | il reste un `A_GENERER` dans `backend/.env`, une valeur mal formée (secret trop court, clé qui n'est pas en hexadécimal), ou `ENVIRONNEMENT`, `URL_APPLICATION_WEB` ou `CONTACT_SECURITE` manque | compléter `.env` (voir « Préparer la configuration ») |
| `cargo run --bin migrer` s'arrête sur « échec des migrations » | la base de développement contient déjà une structure (migration 0022) ; ou, avec « role "laaficare_serveur" does not exist », les rôles n'ont pas été créés | repartir d'une base vide, `docker compose down -v` (voir « Préparer la configuration ») ; ou créer les rôles (voir « Rôles de la base ») |
| Le serveur ou un outil répond « base pas à jour » | les dernières migrations ne sont pas appliquées | lancer `cargo run --bin migrer`, puis relancer |
| Les tests `droits_des_roles` échouent avec « role … does not exist » | les rôles n'existent pas sur le PostgreSQL de développement | les créer (voir « Rôles de la base ») |
| « envoi de SMS simulé : refusé en production » | `ENVIRONNEMENT=production` sur une machine de développement | mettre `ENVIRONNEMENT=developpement` |
| Un outil s'arrête quand il demande un mot de passe | il est lancé depuis le terminal de Git Bash | le relancer depuis PowerShell |
| Le serveur s'arrête car la base est injoignable | Docker n'est pas démarré, ou le mot de passe de `DATABASE_URL_SERVEUR` n'est pas celui donné au rôle `laaficare_serveur` (`DATABASE_URL_OUTILS` pour un outil ; `DATABASE_URL` et `POSTGRES_PASSWORD` pour `migrer`) | démarrer Docker ; remettre le bon mot de passe (`\password`, voir « Rôles de la base ») |
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
