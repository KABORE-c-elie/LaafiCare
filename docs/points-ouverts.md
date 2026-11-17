# Points ouverts

Ce document liste ce qui reste à décider ou à construire avant que LaafiCare puisse servir de vrais patients. Pour chaque point : ce qui manque, pourquoi c'est un risque, et les pistes connues. Les points sont rangés du plus urgent au moins urgent.

Les décisions déjà prises sont dans `CLAUDE.md` (sections 12 et 14), qui reste la référence. Ce document est mis à jour à chaque décision qui ferme ou ouvre un point.

État au 2026-10-20.

---

## 1. Priorité : données personnelles

### Purge des identités jamais inscrites

**Ce qui se passe aujourd'hui.** Toute demande de code par SMS crée une *identité* (une ligne qui garde le numéro de téléphone), même si la personne n'achève jamais son inscription. C'est nécessaire pour que les limites d'envoi s'appliquent à tout numéro sans révéler qui est inscrit. Mais ces numéros restent ensuite en base sans raison.

**Pourquoi c'est un risque.** Un numéro de téléphone est une donnée personnelle. La loi burkinabè demande de ne pas garder les données plus longtemps que nécessaire (loi n° 001-2021/AN, article 25, voir section 3).

**Règle déjà notée.** Effacer, au-delà de 24 heures, une identité qui n'a aucun compte.

**Pistes :**
- (a) à chaque demande de code, comme le nettoyage des envois de plus d'une heure ;
- (b) par une tâche de fond du serveur, à intervalle régulier ;
- (c) par un outil lancé à la main.

**Contrainte.** Ne jamais effacer une identité reliée à autre chose (compte, dossier patient, invitation…).

### Durée de conservation des sessions (décidée)

Une *session* correspond à une connexion sur un appareil. **Règle décidée le 2026-10-12** : à chaque nouvelle connexion, les sessions terminées (expirées ou révoquées) depuis plus de **30 jours** sont effacées. **Exception** : une session coupée parce qu'un vol de jeton est soupçonné (motif `rejeu_detecte`) est gardée **6 mois**, comme trace ; ce n'est pas à elle seule une preuve de vol (voir « Fausses alertes de rejeu », section 2). À vérifier avec la CIL, avec les autres durées de conservation (section 3).

---

## 2. Sécurité, avant la production

### Déconnexion — fermé le 2026-10-14

Réglé par l'option 3, décidée le 2026-10-12 : jetons d'accès de 15 minutes, jeton de renouvellement qui change à chaque usage et révocable, une session par appareil vérifiée à chaque demande. Déconnexion d'un appareil (`POST /api/sessions/deconnexion`) ou de tous (`POST /api/sessions/deconnexion-partout`) : voir `docs/api.md`, section 1.7. **Stockage du jeton dans un navigateur** (applications web, décision du porteur, 2026-10-19) : **option A, jeton gardé en mémoire**, pour le développement. Reste ouvert : **option C (cookie `__Host-`) à confirmer avant le pilote**.

### Fausses alertes de rejeu sur un réseau instable

**Signalé par l'équipe mobile le 2026-10-14.** Si deux demandes de renouvellement perdent leur réponse (réseau instable), une troisième tentative peut arriver plus de 30 secondes après le premier usage du jeton. Le serveur la traite alors comme un **rejeu** et ferme la session (motif `rejeu_detecte`), alors qu'il n'y a pas eu de vol. Le patient doit se reconnecter ; l'équipe mobile l'accepte.

**Conséquence** : sur un réseau instable, les sessions révoquées pour `rejeu_detecte` peuvent contenir de **fausses alertes**. Une telle session **n'est donc pas à elle seule une preuve de vol**, y compris dans la trace gardée 6 mois.

**À revoir plus tard**, si ces fausses alertes deviennent fréquentes : **d'abord mesurer leur nombre**, avant de changer la tolérance de 30 secondes (sa durée, ou le nombre de réutilisations permises).

### Connexion refusée à tort dans la même seconde qu'un second facteur réussi

**Limite connue, acceptée le 2026-10-15 (étape 3.6.3).** Le jeton intermédiaire du second facteur ne sert qu'une fois : la base retient l'heure d'émission (à la seconde près) du dernier jeton utilisé et refuse tout jeton qui n'est pas plus récent (migration 0018). Si la même personne se reconnecte **dans la même seconde** qu'un second facteur réussi, son nouveau jeton porte la même heure et il est **refusé à tort** (401 `jeton_invalide`). Il lui suffit de se reconnecter une seconde plus tard.

**À revoir** si cela gêne en pratique : un identifiant unique par jeton intermédiaire (champ `jti`, RFC 7519 §4.1.7), enregistré à son usage, lèverait la limite.

### Fenêtre de réenrôlement d'un administrateur : écriture réservée aux outils locaux

**Fermé le 2026-10-22 (P1, migration 0026).** Le rôle des outils, `laaficare_outils`, est le seul à pouvoir écrire `compte.reenrolement_autorise_jusqu_au` (droits par colonne). Le serveur la referme seulement, par la fonction `vider_fenetre_reenrolement` ; aucune route ne peut ouvrir une fenêtre, et la base le garantit.

### Prévenir l'administrateur quand son second facteur change

**Exigence notée le 2026-10-15.** Quand un second facteur est enregistré ou désactivé sur un compte administrateur, la personne doit en être prévenue **par email**, par un autre canal que l'opération elle-même. Sources : NIST SP 800-63B rév. 4, §4.1.2.1 (« When an authenticator is added, the CSP SHALL notify the subscriber via a mechanism independent of the transaction binding the new authenticator ») et §4.2.3 (« account recovery SHALL cause a notification to be sent to the subscriber ») ; OWASP ASVS 5.0, 6.3.7.
- **Fait pour la désactivation** (2026-10-18, étape 3.6.5) : `desactiver_totp` prévient la personne à son email, si elle en a un.
- **Fait pour l'enregistrement** (2026-10-21) : la confirmation du réenrôlement prévient la personne par email. Les deux alertes passent par `alerte_securite.rs` et donnent l'adresse de l'équipe (`CONTACT_SECURITE`), exigée par NIST §4.6 (« including contact information »).
- **Reste, avant le pilote** : créer une **vraie adresse email de l'équipe, lue régulièrement**, pour `CONTACT_SECURITE` (décision du porteur, 2026-10-21). En production, les adresses d'exemple de la RFC 2606 y sont refusées.
- **Reste, pour la production** : NIST SP 800-63B rév. 4 §4.6 exige d'accepter **au moins deux adresses de notification** par compte (« CSPs SHALL support at least two notification addresses per subscriber account »). Aujourd'hui, seul l'email est utilisé, le SMS restant réservé à l'OTP. À choisir : un SMS de sécurité, ou une seconde adresse email.

### Essai manuel à faire : parcours du premier administrateur (étape 3.6.5)

**Noté le 2026-10-18.** Le parcours « Créer le premier administrateur » du README (outil dans PowerShell, puis connexion, réenrôlement et second temps dans Postman) **n'a pas encore été essayé à la main** : Postman ne répondait pas, à vérifier par le porteur. La logique est couverte par les tests automatiques, mais pas la saisie masquée dans un vrai terminal ni l'enregistrement dans une application d'authentification. À faire avant de s'appuyer sur un compte administrateur réel.

### Toujours au moins deux administrateurs avec un second facteur actif

**Règle d'organisation notée le 2026-10-18.** `desactiver_totp` exige un **autre** administrateur que la personne visée (Z9, refusé par la base sinon). Si le seul administrateur perd son téléphone et ses codes de secours, plus personne ne peut lui rouvrir l'accès : l'administration est bloquée. **Aucune procédure d'urgence n'est prévue** (décision du porteur) : l'équipe doit toujours garder **au moins deux administrateurs avec un second facteur actif**, créés avec l'outil `creer_administrateur`.

### Ajouter la CNIB d'un administrateur plus tard

**Noté le 2026-10-18.** La CNIB d'un administrateur est **facultative** pour l'instant : un administrateur créé sans CNIB n'a pas de profil (`profil_administrateur`). Si elle devient obligatoire, il faudra un moyen de l'ajouter à un administrateur existant (outil de l'équipe, avec trace), en même temps que l'outil qui change un numéro de CNIB (ci-dessous).

### Purge des codes envoyés par email

**Noté le 2026-10-18 (migration 0020).** Les codes de vérification utilisés, annulés ou expirés restent dans `jeton_email` (seule leur empreinte). Leur durée de conservation et leur effacement sont à décider, avec les autres durées de conservation (section 3, CIL).

### Empreinte du code reçu par SMS

Le code est enregistré en SHA-256 simple. Avec seulement un million de codes possibles, une copie volée de la base permettrait de retrouver un code encore valable.

- **Option à étudier** : une empreinte HMAC calculée avec une clé secrète du serveur, sur le modèle de la clé du second facteur (`TOTP_CLE_CHIFFREMENT`).
- **Risque faible** : un code ne vit que 5 minutes et n'accepte que quelques essais.

### Autres points de sécurité

- **Limite du nombre de demandes par adresse IP** sur les routes de connexion et de réinitialisation (patients, professionnels, administrateurs).
- **Liste de mots de passe interdits** (exigée par la norme NIST SP 800-63B rév. 4) : source à choisir, liste intégrée ou service externe.
- **Sauvegarde de la clé `TOTP_CLE_CHIFFREMENT`**, à part et en lieu sûr, jamais avec la base. La perdre rendrait tous les seconds facteurs inutilisables.
- **Outils de l'équipe LaafiCare**, à concevoir :
  - changer l'email d'un professionnel qui a perdu l'accès à sa boîte, après vérification de sa pièce d'identité (CNIB), avec une trace. L'outil **fermera toutes les sessions du compte** (`fermer_toutes`, avec un nouveau motif à prévoir alors) ; la vérification de l'email est déjà effacée par la base à chaque changement (migration 0016) ;
  - déconnecter de force un compte compromis ;
  - changer le numéro de CNIB d'un administrateur (renouvellement de la carte, future carte de l'AES).
- **Désactivation du second facteur par les structures validées** (vérification de la CNIB à l'accueil) : à rediscuter. En V1, seule l'équipe LaafiCare le désactive.

### Exception cargo audit : `rsa` (RUSTSEC-2023-0071)

**Décidée le 2026-10-21.** L'attaque Marvin permet, par le temps de calcul, de retrouver une clé RSA **privée** ; aucune version corrigée de `rsa` n'existe. La crate arrive seulement par la fonctionnalité `rust_crypto` de `jsonwebtoken`, qui l'impose ; aucune fonctionnalité ne garde HMAC sans RSA. **Nous ne sommes pas touchés** : les jetons sont en HS256 seulement, avec un secret partagé, et aucune clé RSA n'existe dans le projet (`src/jwt.rs` ; test `hs256_seulement`). L'exception est dans `backend/.cargo/audit.toml`. **À retirer** dès qu'une version corrigée de `rsa` existe, ou si une clé RSA entre dans le projet.

### Bilan cargo audit du 2026-11-16 (étape 7)

Quatre nouveaux avis, publiés du 13 au 30 octobre 2026 :
- **corrigés** par une mise à jour de version compatible, dans `Cargo.lock` seulement (`Cargo.toml` inchangé) : `ctutils` 0.4.2 → 0.4.4 (RUSTSEC-2026-0385, comparaisons qui n'étaient pas à temps constant ; la crate sert à `aes-gcm`, qui chiffre les secrets TOTP, et aux empreintes SHA) ; `tokio` 1.53.1 → 1.53.3 (RUSTSEC-2026-0386, marqué *unsound*). Suite complète relancée ensuite ;
- **exceptions ajoutées, à confirmer par le porteur** (`backend/.cargo/audit.toml`, justification détaillée) :
  - `crypto-bigint` 0.5.5 (RUSTSEC-2026-0387) : il n'arrive que par l'ECDSA (ES256, ES384) de `jsonwebtoken`, comme `rsa` ; aucune clé à courbe elliptique dans le projet, HS256 seul ; la version corrigée n'est pas compatible avec `jsonwebtoken` 11.1.0 ;
  - `libsqlite3-sys` 0.37.0 (RUSTSEC-2026-0459) : dans `Cargo.lock` par `sqlx`, mais **jamais compilé** (seul PostgreSQL est activé) ; la version corrigée est refusée par `sqlx-sqlite` 0.9.0.
- **À refaire** à chaque nouvelle version de `jsonwebtoken` ou de `sqlx`, pour retirer ces exceptions.

### Intégration de la branche remediation-audit (2026-11-17)

Reprise morceau par morceau sur `integration-remediation-audit` (détail : CLAUDE.md §16).
- **Non repris : le cache de 15 s du contrôle d'exercice** (crate `moka`). Un accès retiré doit être refusé dès la requête suivante (règle D4) ; avec le cache, il resterait utilisable jusqu'à 15 s.
- **Non repris : la tâche de maintenance périodique** (`maintenance.rs`). Raisons :
  - la purge des sessions existe déjà, à chaque connexion (décision Q-b), et celle des envois de code SMS, à chaque envoi dans la même transaction (décision O7 b) ; la remédiation les déplaçait vers la tâche, ce qui change ces deux décisions, et les garder aux deux endroits ferait doublon ;
  - elle ajoutait une purge des jetons d'email (utilisés, annulés ou expirés depuis plus de 30 jours) : une règle de conservation nouvelle, jamais décidée.
  **Question au porteur** : faut-il une durée de conservation des jetons d'email (vérification, réinitialisation) ? Si oui, laquelle, et faut-il garder une trace de la vérification d'un email après l'effacement du jeton ?
- **Non repris : la fonction `uuidv7()` de remplacement** (PostgreSQL 18 la fournit) ni **CORS** (voir la question « CORS en production »).
- **Avis `cargo audit` du 2026-11-17** : RUSTSEC-2026-0543, `chrono` 0.4.45 déclaré « unmaintained » (plus maintenu), sans faille connue ni version corrigée. Avertissement seulement. **À suivre** : si une faille est publiée, envisager `jiff` ou `time` (choix à présenter avant tout changement de dépendance).

### Bilan de l'itération pharmacie (2026-11-16)

- `cargo audit` : aucun nouvel avis ; aucune dépendance ajoutée par la pharmacie (`sha2`, `serde_json` et `chrono` étaient déjà là).
- `cargo clippy` : aucun avertissement dans le code de la pharmacie. Les champs d'erreur ajoutés (`pieces_manquantes`, `medicament_id`, `quantite_disponible`) sont rangés dans une boîte (`Complement`, `erreur_api.rs`) : sans elle, la réponse d'erreur de toutes les routes grossissait et clippy signalait de nouveaux avertissements dans le code existant. Les 8 avertissements de style du code existant restent.
- Relecture de sécurité : la pharmacie vient toujours de l'affectation du jeton ; chaque lecture et chaque écriture filtre sur elle, et la base le garantit par des clés étrangères composites ; un lot choisi doit être du médicament de la ligne ; montants calculés en entiers de 64 bits (1 000 000 unités × 100 000 000 FCFA au plus) ; aucune valeur saisie dans les messages d'erreur ; recherche du catalogue avec les caractères `%` et `_` échappés. **Limites notées** : un pharmacien peut rejouer la clé d'idempotence d'un collègue de la même pharmacie et recevoir le même objet (même pharmacie, aucune fuite) ; aucune limite du nombre de requêtes par pharmacie (comme le reste de l'API, avant la production).

### Application web (noté le 2026-10-20)

Détail dans `web/docs/decisions.md` (branche `web`) :
- **Trusted Types** : ils bloquent l'enregistrement du service worker d'Angular (le travail hors connexion). Leur activation dans la politique de sécurité du contenu (CSP) est à résoudre.
- **Styles en ligne de la CDK d'Angular** : une CSP stricte, qui interdit les styles en ligne, les refuse. L'affichage reste correct, mais la console signale ces blocages ; leur traitement est à décider.
- **Faille de `vite`** : elle ne touche que le serveur de développement, jamais l'application déployée. En attente d'`@angular/build` 22.2.4.
- **CORS en production** (2026-11-17) : le serveur n'active aucun CORS (`docs/securite.md`, section 12). **Question** : en production, l'application web sera-t-elle servie depuis la même adresse que l'API (même domaine, ou même proxy) ? Si non, il faudra une liste fermée des origines permises, sans jamais accepter n'importe quelle origine avec les identifiants.

### À faire dans l'application web (étape 3.12)

- **Écran « Mot de passe oublié ? »** (professionnels et administrateurs) : un champ email, et **toujours le même message**, « Si un compte correspond à cette adresse, un lien vient d'être envoyé. » ; la route dépend de la sorte de compte (`docs/api.md`, section 2.11).
- **Page `/reinitialiser-mot-de-passe`** : jeton lu après `#`, jamais gardé ; deux champs mot de passe avec la liste des règles à cocher ; envoi par `POST` au clic, jamais par un GET ; en-tête `Referrer-Policy: no-referrer` ; puis retour à l'écran de connexion (aucune session ouverte). La même page sert aux administrateurs.

### À faire dans les applications web et mobile (étape 3.7)

- **Page `/verifier-email`** (application web) : lecture du jeton après `#`, bouton de confirmation, envoi par `POST`, jamais par un GET ; contrat dans `docs/api.md`, sections 2.7 et 2.8.
- **Avertissement sous le mot de passe professionnel** du formulaire de création de structure : « Choisissez un mot de passe différent de celui de votre compte patient ». C'est un conseil, pas une obligation : le serveur accepte un mot de passe identique (décision du porteur, 2026-10-21).
- **Aide sous les champs email** (email de la structure, email professionnel) : « Sans accent ni espace » (adresses en ASCII seulement, décision du porteur, 2026-10-21).

### À faire dans les applications (abandon et désactivation, 2026-10-21)

- **Confirmation avant d'abandonner une demande de structure** (AB4) : l'abandon est définitif ; contrat dans `docs/api.md`, section 2.8.
- **Confirmation avant de retirer l'accès d'un membre** : contrat dans `docs/api.md`, section 2.12.

### À faire dans l'application mobile (espace patient MUNASEB, étape 6, 2026-10-21)

Contrat dans `docs/api.md`, section 2.13.
- **Écran « Mes droits »** selon `etat`, avec les textes conseillés. En carence, « Couverture à partir du JJ/MM/AAAA », jamais « expiré ». Le solde est présenté comme indicatif.
- **Écrans « Mes demandes » et « Détail »** : le motif de rejet n'apparaît que dans le détail ; la « part MUNASEB demandée » est distincte du « montant remboursé » ; `plafond_atteint` explique l'écart.
- **Couleurs** : une demande rejetée, un contrat expiré ou suspendu ne s'affichent ni en rouge ni en orange (réservés aux alertes de santé, CLAUDE.md §6) ; couleur neutre, icône et texte.
- **Notifications** : badge avec `non_lues` ; ouvrir une notification la marque comme lue, puis ouvre la demande liée (`demande_id`). Sans push, les notifications sont rechargées à l'ouverture de l'application et quand on tire l'écran vers le bas.
- **Hors connexion (règle du porteur, 2026-10-21)** : si l'application garde les droits et les demandes pour les afficher hors connexion, **ces données sont chiffrées sur le téléphone**, comme les rappels (base locale chiffrée, clé dans `flutter_secure_storage`) ; sinon, **rien n'est gardé**. L'écran affiche alors « mis à jour le … ».
- **Jeton** : renouvelé avant l'appel quand il expire dans moins d'une minute, comme ailleurs.
- **Vérification du 2026-11-16 avec les écrans mobiles déjà validés** : l'écran « Mutuelle » (reste du plafond, période, demandes récentes) se construit avec `GET /api/patients/munaseb/droits` et `GET /api/patients/munaseb/demandes?limite=3` ; le détail avec suivi en étapes, avec l'`historique` du détail ; les notifications lues et non lues, avec `lu` et `non_lues` ; l'écran sans mutuelle, avec `"etat": "aucun_contrat"`. **Aucune route nouvelle n'est nécessaire.** Depuis l'étape 5, `numero_carte` est celui de la carte de l'année en cours (section 2.13).

### À faire dans l'application web (contrats MUNASEB, étape 5, 2026-11-16)

Contrat dans `docs/api.md`, section 2.14.
- **Adhésion** : demander le NIP (lu sur le QR code de l'application du patient) **et** la date de naissance (lue sur sa pièce d'identité). Un 404 `patient_inconnu` ne dit pas lequel des deux est faux : afficher « NIP ou date de naissance incorrect ».
- **Recherche** : par numéro de carte seulement ; il n'existe pas de recherche par NIP ni par nom.
- **Paiement** : le champ « référence de la transaction » n'apparaît que pour le mobile money. Les montants ne se saisissent pas : afficher ceux des paramètres en vigueur.
- **Paramètres** : l'écran de modification n'apparaît que pour le **responsable** de la MUNASEB (un agent reçoit 403) ; demander une confirmation avant d'enregistrer (le plafond et la carence touchent toutes les adhésions suivantes) ; montrer l'historique.
- **Trop d'essais** : sur un 429 `trop_de_demandes` à l'adhésion, afficher « Trop d'essais sans résultat. Réessayez dans N minutes. » avec `Retry-After`, sans rouge ni orange.
- **À décider (2026-11-16)** : durée de conservation des traces d'échecs d'adhésion (`echec_adhesion_munaseb`, gardées sans limite aujourd'hui, comme les autres traces) ; valeur de la limite (5 en 15 minutes, reprise du verrouillage BF-01) à revoir avec la MUNASEB après le pilote, selon le nombre réel de fautes de frappe aux guichets.
- **Couleurs** : un contrat expiré ou suspendu ne s'affiche ni en rouge ni en orange (CLAUDE.md §6).

---

## 3. Mise en production

### Cadre légal : la CIL et le lieu d'hébergement

Texte lu : **loi n° 001-2021/AN du 30 mars 2021 portant protection des personnes à l'égard du traitement des données à caractère personnel** (elle abroge la loi n° 010-2004/AN), copie publiée par le ministère de la Fonction publique : <https://www.fonction-publique.gov.bf/fileadmin/user_upload/storage/22-08-2021-C2-LOI-N__001-2021_AN_portant_protection_des_personnes.pdf>. L'autorité de contrôle est la **CIL** (Commission de l'informatique et des libertés).

Ce que dit le texte, article par article :

| Article | Contenu |
|---|---|
| 2 | Les données de santé sont des **données sensibles**. |
| 13 | Le consentement préalable de la personne est la règle. Exception : le traitement « aux fins de médecine préventive, de diagnostics médicaux, d'administration de soins ou de traitements, de gestion des services de santé », s'il est mis en œuvre par un membre d'une profession de santé ou une personne tenue au secret professionnel. |
| 24 | Le responsable du traitement prend toute mesure utile pour empêcher que les données soient « déformées, endommagées, perdues, volées », ou que des tiers non autorisés y accèdent. |
| 25 | Les données ne sont pas conservées au-delà de la durée nécessaire. |
| 26, 27 | Avant tout traitement, une formalité auprès de la CIL : demande d'avis, autorisation, déclaration normale ou déclaration simplifiée. Par défaut, une **déclaration normale**. |
| 31 | Exigent une **autorisation** de la CIL, entre autres : les traitements portant sur « un numéro national d'identification ou tout autre identifiant de la même nature » ; la recherche dans le domaine de la santé ; « les transferts de données vers un pays étranger ». |
| 32 | Le contenu et le format de la déclaration ou de la demande sont fixés par la CIL ; les délais de réponse, par décret. |
| 37 | « Les données de santé permettant l'identification directe ou indirecte des personnes physiques doivent être hébergées sur le territoire national sauf dérogation prévue à l'article 56 ». |
| 56 | Énumère les pouvoirs de la CIL (décisions, recommandations, mesures de sécurité). Le passage lu ne détaille pas les conditions de la dérogation citée à l'article 37. |

**Ce qu'il reste à faire, sans rien supposer de plus que le texte :**
- **Demander à la CIL quelle formalité s'applique à LaafiCare** : déclaration normale, ou autorisation, notamment à cause du NIP, qui pourrait être un « identifiant de la même nature » qu'un numéro national d'identification (article 31). Le texte ne permet pas de trancher seul.
- **Héberger la base dans le pays** : l'article 37 l'exige pour les données de santé identifiantes. Le choix de l'hébergeur doit en tenir compte, sauf dérogation obtenue de la CIL.
- **Ne transférer aucune donnée vers un pays étranger** sans autorisation (article 31). Cela vaut aussi pour les services externes : envoi de SMS ou d'emails, sauvegardes, surveillance.
- **Obtenir le formulaire de la CIL** (article 32) et le décret qui fixe les délais.
- Le décret d'application, s'il existe, n'a pas été lu : à rechercher.

### Sauvegardes

- Sauvegardes régulières de la base, conservées dans le pays (article 37) et chiffrées.
- **Un essai de restauration** à intervalle régulier : une sauvegarde qu'on n'a jamais restaurée n'est pas une sauvegarde sûre.
- La clé `TOTP_CLE_CHIFFREMENT` est sauvegardée à part (section 2).

### HTTPS obligatoire

En production, le serveur n'accepte que des connexions chiffrées (HTTPS) : jetons, mots de passe et données de santé ne circulent jamais en clair. En développement, il reste sur `127.0.0.1`, sans HTTPS.

### Infrastructure

- Rôle PostgreSQL sans droits d'administration pour le serveur : **fait le 2026-10-22** (P1, migration 0026). Reste pour la production : créer chez l'hébergeur un rôle propriétaire sans droits de superuser pour appliquer les migrations, lancer `sql/roles.sql`, et ne garder sur le serveur que `DATABASE_URL_SERVEUR` (et `DATABASE_URL_OUTILS` si les outils y tournent). **Le rôle propriétaire de production ne doit pas être superuser**, et c'est lui qui doit appliquer les migrations : la fonction `vider_fenetre_reenrolement` (migration 0026) est `SECURITY DEFINER`, elle s'exécute avec les droits de son propriétaire (doc PostgreSQL 18, CREATE FUNCTION). Créée par un superuser, elle tournerait avec tous les droits ; créée par le rôle propriétaire sans droits d'administration, elle ne peut que ce que ce rôle peut (en développement, l'utilisateur `laaficare` de Docker est superuser : écart accepté par le porteur le 2026-10-22).
- Chiffrement du disque de la base.
- Fournisseurs réels : SMS (codes, invitations) et email (réinitialisation, vérification de l'email). Crate et fournisseur à présenter.
- Fichiers déposés par les structures : réécriture des images, antivirus, désactivation du contenu actif des PDF (OWASP).

### Avant le pilote : décisions extérieures à prendre tôt

**Noté le 2026-10-21.** Ces décisions viennent d'ailleurs que du code, et une étape du MVP les attend :
- **durée de la licence d'une structure** : illimitée, ou à renouveler (étape 3.8) ;
- **règles de suspension d'un contrat MUNASEB** : à demander à la MUNASEB (étape 5) ;
- **valeurs des types d'actes manquants** : dentaire et prothèses, accouchement et consultations prénatales, chirurgie, frais funéraires, radiologie (étape 7, mémoire MUNASEB).

**Outil local de l'équipe pour un compte compromis** (DS10, J11) : **fait le 2026-10-22** (P2, outil `intervenir_compte`, migration 0027) : retrait d'une affectation, blocage et réactivation d'un compte, avec trace.

**Outil « désigner un nouveau responsable »** (noté le 2026-10-22, P2, à faire avant le pilote) : l'équipe peut retirer le dernier responsable d'une structure (compte compromis). La structure reste validée, mais plus personne ne peut y inviter. Il faut un outil de l'équipe pour désigner un nouveau responsable (identité vérifiée, trace), qui se substitue alors à l'invitation.

**Limite connue : un patient sans email n'est pas prévenu** (noté le 2026-10-22, P2, IC5) quand l'équipe bloque ou réactive son compte : les alertes partent par email seulement, et le SMS reste réservé au code de connexion (OTP).

### Éléments à découper en étapes

**Noté le 2026-10-15.** La mise en production n'est pas encore découpée en étapes, comme le sont celles du backoffice (CLAUDE.md, section 12). Voici les éléments déjà connus, repris de ce document, à découper plus tard en étapes ordonnées, chacune présentée et validée avant d'être faite :

- **Cadre légal** : formalité auprès de la CIL (déclaration ou autorisation), hébergement des données de santé dans le pays, aucun transfert vers l'étranger sans autorisation (section 3, « Cadre légal »).
- **Comptes PostgreSQL** : faits le 2026-10-22 (P1) ; reste leur mise en place chez l'hébergeur (section 3, « Infrastructure »).
- **HTTPS** obligatoire.
- **Sauvegardes** chiffrées, dans le pays, avec essais de restauration ; sauvegarde à part de la clé `TOTP_CLE_CHIFFREMENT`.
- **Chiffrement du disque** de la base.
- **Fournisseurs réels** d'envoi de SMS et d'email (crate et fournisseur à présenter).
- **Limite du nombre de demandes par adresse IP** sur les routes de connexion et de réinitialisation.
- **Liste de mots de passe interdits** (NIST SP 800-63B rév. 4).
- **Stockage du jeton dans un navigateur** (applications web) : en mémoire pour le développement (option A) ; **option C (cookie `__Host-`) à confirmer avant le pilote**.
- **Accord de la MUNASEB pour l'usage de ses couleurs et de son logo** dans l'application web (palette rose `#8C4C65`), à obtenir **avant le pilote** (noté le 2026-10-19).
- **Fichiers déposés par les structures** : réécriture des images, antivirus, contenu actif des PDF.

---

## 4. Décisions métier à prendre

### Seuils de vigilance (orange) et d'alerte (rouge)

Les seuils sont fixés par les **professionnels concernés de chaque structure**, selon les appareils qu'ils utilisent (valeurs de référence du fabricant). À concevoir avec les modules médicaux :
- **qui peut les modifier** (quels rôles) ;
- une **trace non modifiable** de chaque changement : qui, quand, ancienne et nouvelle valeur ;
- chaque résultat **garde le seuil utilisé au moment de la mesure** : un changement de seuil ne réécrit jamais un résultat passé ;
- un seuil fixé par une **source officielle** (par exemple 140 mmHg pour la tension, dans le carnet de santé mère-enfant) peut être rendu **plus strict** par une structure, **jamais plus souple**.

### Carte des structures de santé (proposition, 2026-11-16, aucune route écrite)

Demande du porteur pour l'application patient. **Proposition, à valider avant d'écrire quoi que ce soit** :
- **Quand** : après l'ouverture des autres types de structure (hôpital, clinique, pharmacie, laboratoire, « reste de la 3.7 », après le pilote). Aujourd'hui, seule la MUNASEB peut être validée : la carte serait vide.
- **Route** : `GET /api/structures/carte?sud=&ouest=&nord=&est=&type=`, la zone affichée à l'écran (un rectangle de latitudes et longitudes), au plus 200 structures par réponse. Jeton patient exigé, pour limiter la collecte automatique.
- **Ce qui est renvoyé** : seulement les structures **validées et sous licence** (jamais un brouillon, une demande en attente, une structure refusée ou suspendue) ; nom, type, téléphone, **numéro d'urgence**, commune, province, latitude et longitude. **Jamais** : le responsable, l'email, les membres, les pièces, la licence.
- **Calcul de la distance** : sur le téléphone, à partir de sa position ; **la position du patient n'est jamais envoyée au serveur** (minimisation, loi n° 001-2021/AN).
- **Hors connexion** : la dernière liste peut être gardée sur le téléphone (ce ne sont pas des données personnelles), avec « mis à jour le … ».
- **Urgences** : le bouton d'appel du numéro d'urgence peut être en rouge (exception du porteur du 2026-10-26, CLAUDE.md §6).
- **Questions** : un numéro de structure peut être le téléphone personnel d'un praticien seul (faut-il son accord pour l'afficher ?) ; faut-il les horaires et les services (garde, maternité) ; lien avec BF-06 (médicaments disponibles), qui viendra avec le module Pharmacie.

### Obtenir le NIP d'un patient qui ne peut pas ouvrir son application

Un soignant a besoin du NIP pour ouvrir le dossier. Le patient peut être trop malade, avoir un téléphone verrouillé, déchargé ou perdu, ou ne pas avoir de smartphone.

**Pistes :**
- le QR code du NIP affichable **sans déverrouiller l'application** ;
- une **carte NIP** physique ;
- le NIP **noté dans le carnet de santé** ;
- une **recherche par identité** à l'accueil (nom, prénom, date et lieu de naissance).

**Questions liées :** qui peut ouvrir un dossier avec un NIP ; faut-il un **accès d'urgence tracé** (« bris de glace ») quand le patient ne peut pas consentir. À décider avec le module du dossier médical.

### Structures et rôles

- Durée d'une licence de structure : illimitée ou à renouveler.
- Pièces justificatives exigées pour la MUNASEB, les hôpitaux et les laboratoires.
- **Pièces exigées pour une pharmacie (officine)** : depuis le 2026-11-16 (D4), seule l'**autorisation d'ouverture et d'exploitation de l'officine** est demandée : c'est le seul document nommé par un texte officiel trouvé (arrêté n° 2006-040/MS/CAB, article 4 b, sur le transfert d'une officine). **Question à poser à la DGPML ou à l'Ordre national des pharmaciens** : quelles pièces prouvent qu'une officine a le droit d'exercer (autorisation d'ouverture et d'exploitation, inscription du titulaire au tableau de l'Ordre, autre) ? Aucune liste officielle de pièces pour l'ouverture n'a été trouvée.
- **Date d'expiration au mois seul** (« EXP 06/2027 ») : **aucune source officielle burkinabè ni de l'UEMOA trouvée** sur la convention (2026-11-16). La seule source officielle trouvée est étrangère : la pharmacopée des États-Unis (USP, *General Notices*) dit qu'une date donnée en mois et année désigne **le dernier jour de ce mois** ; elle ne s'applique pas au Burkina. **Règle retenue, la plus prudente pour le patient** : le lot est périmé **dès le premier jour du mois imprimé** ; LaafiCare enregistre le dernier jour du mois précédent et garde la trace du mois imprimé (migration 0033, `docs/api.md` §2.15). Prix de cette prudence : au plus un mois de vente perdu par lot. **Question à la DGPML ou à l'ANRP** : pour un médicament dont l'emballage indique seulement le mois et l'année d'expiration, jusqu'à quel jour peut-il être délivré au Burkina (dernier jour du mois indiqué, ou dernier jour du mois précédent) ? Quel texte le dit ?
- **Prix de vente des médicaments en officine** (2026-11-16, ne bloque rien) : aujourd'hui, chaque responsable fixe librement son prix de vente (D7). **Question à la DGPML** : les prix de vente des médicaments en officine sont-ils encadrés au Burkina (prix public fixé, coefficient de marge maximal, liste de prix publiée) ? Par quel texte ? Si oui, LaafiCare pourrait contrôler qu'un prix saisi respecte la règle, ou l'afficher à côté.
- **FEFO : source OMS à vérifier** (D8, demande du porteur du 2026-11-16). La sortie du lot qui périme le plus tôt (*first expired, first out*) est appliquée par le système, mais la référence OMS qui la recommande n'a pas pu être lue : le site `www.afro.who.int` n'est pas joignable depuis l'environnement de travail (2026-11-16). **À faire** : retrouver le texte de l'OMS (bonnes pratiques de stockage des produits pharmaceutiques) et le citer dans `stock_pharmacie.rs`.
- **Liste nationale des médicaments** (D6) : le catalogue reste **vide** tant que la liste nationale des médicaments essentiels 2023 n'est pas déposée dans `docs/Reglementation, loi et reference/`. Il faudra ensuite la convertir au format de l'outil (`denomination;dci;forme;dosage`, `backend/src/catalogue.rs`). **Question** : la liste nationale donne-t-elle les noms commerciaux, ou seulement les DCI ? Si elle ne donne que les DCI, une ligne du catalogue sera « DCI, forme, dosage » et les noms commerciaux viendront d'une autre source officielle (autorisations de mise sur le marché de l'ANRP).
- **Corriger une ligne du catalogue** : un médicament importé ne se modifie ni ne s'efface (une vente passée le désigne). **Question** : faut-il pouvoir marquer un médicament comme retiré du marché (sans l'effacer), et sur quelle source ?
- **Clés d'idempotence gardées sans limite de durée** : choix fait seul, le draft IETF laisse la durée au serveur. À revoir si la table grossit trop (une ligne par création envoyée avec une clé).
- **Bénéfice net : règles de calcul** (2026-11-16). (1) **Décidé par le porteur** : toute annulation (vente, perte, écart d'inventaire, entrée, dépense) corrige la **période du mouvement d'origine** ; le journal n'est jamais modifié, le rapport rattache l'annulation à la date du mouvement annulé. **Choisies seul, à confirmer** : (2) les **achats** ne sont pas des dépenses : leur coût compte à la vente ou à la perte, le reste est dans la valeur du stock ; (3) un **excédent d'inventaire** diminue les pertes ; (4) une dépense n'a **jamais une date future** ; (5) les mouvements anciens ne sont jamais recalculés à un autre prix. **Question** : ces règles conviennent-elles au titulaire ? Faut-il afficher les achats de la période à part (trésorerie) ?
- **Pharmacie : produits sur ordonnance** (D10) : la vente enregistre seulement la case « sur ordonnance papier ». Aucune liste officielle accessible ne dit quels produits l'exigent. **Question à la DGPML ou à l'Ordre** : existe-t-il une liste officielle des médicaments délivrés seulement sur ordonnance ?
- **Préparateur en pharmacie** (D5) : **remplacé le 2026-11-17** par le rôle `auxiliaire_pharmacie` (décision du porteur) : entrées, ventes, pertes et inventaires ; jamais les prix d'achat, les dépenses, le bénéfice ni un dossier. **Choisi seul, à confirmer** : il saisit le prix d'achat d'une entrée (lu sur la facture), mais aucune réponse ne le lui renvoie ; il n'annule rien et ne règle pas les seuils (403). **Question** : un auxiliaire peut-il annuler sa propre saisie erronée, ou faut-il toujours le pharmacien ?
- **Découpage administratif (région, province, commune)** : texte libre pour l'instant (migration 0022). Recherche du 2026-10-20 : le compte rendu officiel du Conseil des ministres du 2 juillet 2025 (site de la Présidence du Faso) annonce le passage de 13 à 17 régions et de 45 à 47 provinces, et donne les noms des 17 régions avec leurs chefs-lieux ; mais ni le texte des décrets, ni leur numéro, ni la liste complète des provinces et des communes n'ont été trouvés (une période transitoire de six mois y est aussi annoncée). Fermer les listes demande le décret publié au Journal officiel.
- **Position GPS d'une structure** : contrôlée dans le rectangle donné par l'Institut géographique du Burkina (rapport d'août 2014 aux Nations unies, UN-GGIM : 8° à 16° de latitude nord, 6° ouest à 3° est). Ce rectangle déborde sur les pays voisins ; un contrôle par le contour du territoire demanderait une source officielle de ce contour.
- **Adresses email en ASCII seulement** (décision du porteur, 2026-10-21) : les domaines internationaux (`exemplé.bf`, `例え.jp`) et les parties locales accentuées sont refusés, par le code (`adresse_email.rs`) et par la base (migration 0022). À revoir si un utilisateur en a besoin : il faudra alors une migration, et une règle de comparaison des majuscules qui ne dépende pas des réglages de langue de la base (OWASP : convertir les domaines internationaux en punycode pour les comparer).
- **Validation d'une seconde MUNASEB** : fait à l'étape 3.8 réduite (2026-10-21), 409 `munaseb_deja_validee`, contrôlé avant l'écriture, puis l'erreur de l'index `structure_une_seule_munaseb_validee` traduite en ce même code.
- **Impasse : mot de passe oublié sans email vérifié** (noté le 2026-10-21, étape 3.12, R12). Un lien de réinitialisation ne part que vers un email vérifié (NIST SP 800-63B rév. 4, §4.2 : une adresse de récupération « SHALL be verified »). Un professionnel qui a oublié son mot de passe **et** n'a jamais vérifié son email ne peut plus rien faire : le renvoi du lien de vérification demande lui aussi le mot de passe. **Outil de l'équipe à prévoir** : vérification de l'identité (CNIB), puis, au choix, nouveau lien de vérification envoyé sans mot de passe, ou changement d'email (l'outil « changer l'email » de la section 2), avec une trace.
- **Corriger et soumettre à nouveau une demande refusée** (retour en brouillon, CLAUDE.md §14) : hors de la 3.8 réduite (V10). En attendant, le responsable abandonne sa demande et en crée une nouvelle.
- **Un agent qui quitte lui-même une structure** (DS4, 2026-10-21) : hors du MVP. Aujourd'hui, seul un responsable retire l'accès d'un agent ; un responsable peut retirer le sien s'il en reste un autre.
- **Prévenir l'invitant** quand une invitation est acceptée ou refusée (I17, 2026-10-21) : rien pour l'instant, il le voit dans sa liste.
- **Longueur du SMS d'invitation** : réglé le 2026-10-21 (options a et c du porteur). Le texte fixe est entièrement dans l'alphabet GSM 7 bits (3GPP TS 23.038), et le nom d'une structure est limité à 100 caractères (migration 0023). Avec un nom en GSM 7 bits, le SMS tient en 2 parties au plus (153 caractères par partie, TS 23.040 §9.2.3.24.1). **Reste, accepté** : un nom qui contient un caractère hors de cet alphabet (`ê`, `ç`, « », etc.) fait partir tout le SMS en UCS-2 (70 caractères seul, 67 par partie), donc en plus de parties.
- **Avertissement des numéros d'autorisation en double** dans le détail administrateur (§14) : **fait le 2026-11-16** avec les pièces de la pharmacie (`meme_numero_dans`, `docs/api.md` §2.9).
- **Logo de la structure** (demande du porteur, 2026-10-21) : reporté à la suite de la 3.7. Facultatif, route séparée, PNG et JPEG seulement (jamais SVG), contenu vérifié, image réencodée par le serveur, nom de fichier choisi par le serveur, stockage au Burkina, seul le responsable peut le changer. Proposition à présenter avant d'écrire (OWASP File Upload Cheat Sheet).
- **Second facteur à la création d'une structure** : un professionnel qui a un TOTP actif reçoit 409 `second_facteur_requis` (aucun professionnel ne peut en avoir avant l'étape 3.11). À concevoir avec l'activation du TOTP par les professionnels.
- Où ranger le numéro d'ordre et la spécialité d'un professionnel : **fait le 2026-11-17** (profil professionnel, migration 0035, `docs/api.md` §2.17). Restent les questions de la section « Module médecin » ci-dessous.
- Création d'un compte pour une autre personne : patient sans téléphone, nouveau-né, urgence (patient inconscient). Qui en a le droit, et comment la personne en reprend ensuite le contrôle.
- Major de laboratoire : rôle séparé, ou autorité sur le stock pour un biologiste.

### Module médecin (2026-11-17)

Décisions du porteur MD1 à MD20 et « vues par rôle » du 2026-11-17 (CLAUDE.md §17). Questions encore ouvertes, chacune avec la valeur appliquée en attendant :

- **Sexe du patient** : deux valeurs, `feminin` et `masculin`, facultatif. Aucun texte officiel lu ne fixe la liste. **Question** : quelles valeurs l'état civil burkinabè et le carnet de santé mère-enfant utilisent-ils ?
- **Spécialités** (MD10) : liste courte proposée, `medecine_generale`, `gynecologie_obstetrique`, `pediatrie`, `chirurgie`, `sage_femme`. **À valider par le porteur** ; aucune liste officielle lue.
- **Numéro d'ordre vérifié exigé ?** (MD9) : l'équipe vérifie le numéro, mais rien ne l'exige encore pour ouvrir un dossier. **Question** : faut-il un numéro vérifié avant tout accès à un dossier ? Pour les infirmiers et les sages-femmes, quel ordre professionnel délivre le numéro (aucune source lue) ?
- **Hôpital et CSPS** (MD8) : non ouverts. **Questions** : quelles pièces prouvent qu'un hôpital a le droit d'exercer ? Un CSPS public doit-il demander une licence LaafiCare, ou être enregistré par l'équipe à partir d'une liste du ministère ? Faut-il un type `csps` séparé ?
- **CDC, BF-02, à corriger par le porteur** : le texte dit que le dossier s'ouvre « via son NIP … après scan du QR Code ». Règle décidée le 2026-11-17 : **le NIP seul, écrit ou scanné, n'ouvre jamais un dossier** ; il donne une fiche d'identité selon le rôle (`docs/api.md` §2.18), et le dossier ne s'ouvre que par une prise en charge (code de soin ou CNIB vérifiée) ou un bris de glace tracé.
- **Conservation du journal des accès** (MD6) : **aucune purge** en attendant l'avis de la CIL. **Question à la CIL** : combien de temps garder la trace des accès aux données de santé (loi n° 001-2021/AN, article 25 : « pas au-delà de la durée nécessaire ») ?
- **Limite des scans** : **60 scans réussis par affectation en 15 minutes** (un patient toutes les 15 secondes), en plus de 5 essais sans résultat en 15 minutes (valeur du verrouillage BF-01). La valeur 60 est **proposée sans source chiffrée** (OWASP API4:2023 demande une limite sans en donner). **À valider par le porteur**, et à revoir avec un accueil réel.
- **Adhésion MUNASEB plus faible depuis que l'agent voit le téléphone** (décision du porteur du 2026-11-17) : l'adhésion demande le NIP **et** la date de naissance, pour qu'une faute de frappe sur le NIP ne crée pas un contrat au nom d'une autre personne et que la route ne serve pas à savoir si un NIP existe. Depuis le 2026-11-17, l'agent peut **scanner** un NIP et voir la date de naissance et le téléphone : la vérification « NIP + date de naissance » ne protège plus contre un agent malveillant, seulement contre l'erreur de saisie. **Compensation** : chaque scan et chaque affichage du téléphone est tracé et visible par le patient, et le nombre de scans est limité.
- **Pièces de la clinique** : `autorisation_creation` et `autorisation_ouverture_exploitation` (§14). Le décret n° 2009-676 (structures privées de santé de la reproduction), dans `docs/Reglementation, loi et reference/`, est un PDF scanné : il n'a pas pu être lu (aucun outil de lecture d'image dans l'environnement). À relire pour confirmer la liste.

### MUNASEB

- **Étape 5 faite le 2026-11-16** (migration 0028, `docs/api.md` §2.14), d'après le « Modèle retenu pour l'étape 5 » ci-dessous : paramètres en base et historisés, périodes avec numéro de carte, antenne, niveau d'études et plafond figé, délai de grâce (option A), paiement de la cotisation (espèces ou mobile money, sans API d'opérateur), routes de l'agent (adhésion, renouvellement, détail, recherche par carte, paramètres). **Pas codé**, faute de règle : la suspension, les ayants droit, les tarifs par circuit (voir les questions ci-dessous).
- **Suspension d'un contrat : non codée.** Aucune règle n'est écrite dans nos sources. En attendant : pas de route ; un contrat `suspendu` (posé en SQL seulement) n'est pas couvert (`non_couvert` pour un acte, `suspendu` pour le patient) et **ne se renouvelle pas** (409 `contrat_suspendu`) : on bloque plutôt que de deviner si un paiement lève la suspension.
- **Plafond en base plutôt qu'en constante** : fait le 2026-11-16 (`parametre_munaseb.plafond_periode_fcfa`). **Décision prise à l'étape 5, à confirmer** : le plafond est **figé sur chaque période à sa création** ; un changement ne touche que les périodes suivantes, comme un changement de tarif ne touche jamais une demande existante (CLAUDE.md §12). Même règle pour la cotisation et la fiche (figées sur chaque paiement) et pour la carence et le délai de grâce (lus au moment du paiement).
- **Tarifs par type d'acte et par circuit : conçus (modèle retenu ci-dessous), pas codés à l'étape 5.** Raisons : la valeur de chaque couple (type d'acte, circuit) n'est confirmée par aucune source (consultation gratuite au service de la MUNASEB selon son offre, 80 % avec un bon d'après le bon de 2024, 100 % chez nous) ; les partenaires n'ont pas de circuit en base ; et le paiement direct est hors périmètre. Coder la structure avec des valeurs devinées irait contre la règle « rien ne s'invente ». Les tarifs actuels (`tarif_acte_munaseb`, une ligne par type d'acte) restent en service, et aucune route ne les modifie encore.
- **Questions exactes à poser à la MUNASEB (étape 5, 2026-11-16)** :
  1. **Suspension** : dans quels cas un contrat est-il suspendu (cotisation impayée, fraude, fin d'inscription à l'université, autre) ? Qui la décide ? Les actes faits avant la suspension restent-ils remboursés ? Comment est-elle levée ? Un adhérent suspendu peut-il renouveler ?
  2. **Fin de couverture** : la couverture finit-elle bien le jour anniversaire de l'adhésion, ce jour compris (carte de 2024 : validité = adhésion + 1 an, effet = adhésion + 1 mois) ?
  3. **Délai de grâce** : existe-t-il ? Combien de jours ? Les jours entre la fin et le paiement sont-ils couverts (aujourd'hui : non, option A) ?
  4. **Renouvellement anticipé** : peut-on payer l'année suivante plusieurs mois avant la fin (aujourd'hui : oui, une seule année d'avance) ?
  5. **Plafond** : 100 000 FCFA par an est-il toujours en vigueur ? Par adhérent, ou partagé avec les ayants droit ? Quand il change, s'applique-t-il à l'année en cours ou seulement aux adhésions suivantes (aujourd'hui : aux suivantes) ?
  6. **Ayants droit** : qui est couvert en plus de l'étudiant (conjoint, enfants) ? Comment sont-ils inscrits ? Ont-ils leur propre numéro de carte ?
  7. **Numéro de carte** : quel est son format exact (`0552/2024` sur la carte de 2024) ? Un numéro est-il jamais réattribué une autre année ?
  8. **Cotisation et fiche** : montants actuels (5 000 et 100 FCFA en 2024) ? Changent-ils entre une première adhésion et un renouvellement ?
  9. **Tarifs par circuit** : pour chaque type d'acte (consultation, examen, hospitalisation, pharmacie, laboratoire, lunetterie, et les types manquants), le taux et le plafond par acte au dépôt et au service de santé de la MUNASEB, chez un partenaire avec un bon, et en paiement direct.
  10. **Mobile money** : quels opérateurs ? Quelle référence de transaction l'agent note-t-il ?
- **Durée de conservation des notifications** (noté le 2026-10-21) : aujourd'hui, elles ne s'effacent jamais. À décider avec la purge de l'étape 7.
- Types d'actes sans valeur dans les sources : dentaire et prothèses, accouchement et consultations prénatales, chirurgie, frais funéraires, radiologie.
- Partenaires de la MUNASEB et structures : à rapprocher (un partenaire deviendra un lien entre la MUNASEB et une structure).
- **Règles relevées sur des documents MUNASEB de 2024 (étape 5)** — source : « documents MUNASEB 2024 transmis par le porteur, 2026-10-22 » (une carte de membre et un bon de prise en charge vierge ; aucune donnée personnelle reprise ici). Plus fiables que les sources en ligne (mémoire de 2008, presse), mais à confirmer par la MUNASEB avant de changer le modèle :
  - **cotisation** payée par l'étudiant en 2024 : 5 000 FCFA, plus 100 FCFA de fiche ;
  - **numéro de carte** au format numéro/année (ex. `0552/2024`), repris comme « N° adhésion » sur le bon ; la carte indique aussi l'**antenne** (ex. Bobo), l'université, l'UFR **avec le niveau**, le matricule et le téléphone ;
  - **date d'effet** = date d'adhésion + 1 mois ; **validité** = date d'adhésion + 1 an, et non date d'effet + 1 an : **couverture réelle de 11 mois**. Notre modèle couvre 12 mois à partir de la date d'effet (`calculer_periode`) : **écart** ;
  - **trois circuits** : (1) dépôt pharmaceutique de la MUNASEB, sur ordonnance, l'étudiant paie 20 % ; (2) bon de prise en charge dans les structures conventionnées (SCHIPHRA, HOSCO, Ex-CMA du 30, CHU-YO, OST), l'étudiant paie 20 % ; (3) paiement direct, puis remboursement (hors périmètre aujourd'hui) ;
  - **bon de prise en charge numéroté** : bénéficiaire (adhérent : nom, prénoms, N° adhésion, UFR, année) et **malade** (nom, prénoms, **lien de parenté**) ; date de la maladie ou de l'accident ; actes remplis par le praticien (date, nature « Cs » consultation ou « EX » examen, coût, ordonnance oui ou non, signature et cachet) ; récapitulatif : consultation et examen **80 % MUNASEB / 20 %** bénéficiaire, hospitalisation **100 % / 0 %**, autres, déductions, total net. Le lien de parenté laisse penser que des **ayants droit** sont couverts : notre modèle n'en a pas ;
  - **forfaits de remboursement** : appareillage 10 000, prothèses 10 000, verres correcteurs et montures 15 000, consultation prénatale 15 000, accouchement normal 10 000, césarienne 25 000, chirurgie 50 000, hospitalisation 50 000, rééducation 15 000, scanner 15 000 FCFA ;
  - **plafond** de 100 000 FCFA par période : **absent des deux documents** (il ne vient que du mémoire MUNASEB).
  - **Décision provisoire du porteur (2026-10-22), à confirmer par la MUNASEB** : hospitalisation à **100 %** comme sur le bon, **dans la limite de 50 000 FCFA par hospitalisation** (le forfait du verso de la carte lu comme un plafond).
  - **Observation (2026-10-22)** : le taux semble dépendre **du circuit** autant que du type d'acte. Consultation gratuite au service de santé de la MUNASEB (selon son offre de service), 80 % avec un bon chez un partenaire ; hospitalisation à 100 % avec un bon, forfait en paiement direct. Aujourd'hui, `tarif_acte_munaseb` n'a qu'une ligne par type d'acte : un taux par type d'acte **et par circuit** est à concevoir à l'étape 5.
  - **Réponses du porteur d'après son expérience d'adhérent (2026-10-22), toutes à confirmer par la MUNASEB, à appliquer à l'étape 5** :
    1. **Fin de couverture** : le jour anniversaire de l'adhésion, **compris** (adhésion le 09/04/2024 : effet le 09/05/2024, fin le 09/04/2025). Couverture réelle de 11 mois.
    2. **Renouvellement** : payé avant l'échéance, continuité sans nouvelle carence ; payé après l'échéance mais dans un **délai de grâce de 10 jours** (valeur modifiable, en base), pas de nouvelle carence (ce que devient la couverture pendant l'interruption reste à trancher, voir la proposition de l'étape 5) ; au-delà, nouvelle carence d'un mois. **Le numéro de carte change chaque année** : il doit être porté par la période, pas par le contrat.
    3. **Suspension** : inconnue, à demander à la MUNASEB.
    4. **Plafond** : par adhérent seul, ou partagé avec les ayants droit : à trancher (voir la proposition de l'étape 5).
    5. **Ayants droit** : inconnus, à demander à la MUNASEB.
    6. **Antenne** (lieu d'inscription) : information seulement, **aucun effet** sur la prise en charge ; champ facultatif.
    7. **Paiement de la cotisation** : en espèces aujourd'hui ; LaafiCare doit aussi accepter le mobile money. **Pour le MVP**, l'agent enregistre le mode de paiement (`especes` ou `mobile_money`), la date, le montant et, pour le mobile money, la référence de la transaction. **Aucune intégration d'API d'opérateur avant le pilote.**
    - **Montants de la cotisation et de la fiche** (5 000 et 100 FCFA en 2024) : **en base, modifiables**, jamais en constante.
  - **Après le pilote** : intégration des API Orange Money et Moov Money pour le paiement de la cotisation.
  - **Modèle retenu pour l'étape 5 (accord du porteur, 2026-10-22 ; rien à coder avant les réponses de la MUNASEB)** — **codé le 2026-11-16 à la demande du porteur, sauf les tarifs par circuit** (voir plus haut) :
    - **Tarifs par type d'acte et par circuit** : une ligne de `tarif_acte_munaseb` par couple (type d'acte, circuit), les circuits étant `structure_munaseb` (dépôt et service de santé de la MUNASEB), `partenaire_bon` (structure conventionnée, avec un bon) et `paiement_direct`. Chaque ligne porte un **taux** (0 à 100) et un **plafond par acte** facultatif ; remboursement = min(coût × taux, arrondi ; plafond). Un forfait devient « 100 % dans la limite de N » (lunetterie : 15 000 ; hospitalisation avec un bon : 50 000). Aucune ligne pour un couple : acte non couvert dans ce circuit. La demande garde le circuit, le taux et le plafond appliqués, figés à sa création ; chaque modification de tarif est tracée (auteur, date, ancienne et nouvelle valeur). Le circuit d'un acte se déduit du partenaire. Une réponse de la MUNASEB ne demande alors qu'une mise à jour de valeurs.
    - **Paramètres MUNASEB en base**, modifiables et historisés : cotisation (5 000), fiche (100), délai de grâce (10 jours), plafond par période (100 000), durée de la carence (1 mois). La constante `PLAFOND_ANNUEL_FCFA` disparaît.
    - **Périodes** : date d'adhésion (paiement), date d'effet, date de fin (jour anniversaire de l'adhésion compris) ; **numéro de carte porté par la période** (unique), plus par le contrat ; antenne facultative ; niveau d'études (affiché avec l'UFR sur la carte, il change chaque année).
    - **Renouvellement dans le délai de grâce : option (A)** : pas de nouvelle carence, la couverture reprend **le jour du paiement**, les jours d'interruption **ne sont pas couverts**, et la fin reste à la date anniversaire suivante. Même raison que la règle sans effet rétroactif de CLAUDE.md §12 : sinon, un adhérent paierait le jour où il tombe malade.
    - **Plafond : option (a)** : 100 000 FCFA par période et par adhérent, jusqu'à la réponse de la MUNASEB sur les ayants droit.
    - **Paiement de la cotisation** : table `paiement_cotisation_munaseb` (période, mode `especes` ou `mobile_money`, montants de la cotisation et de la fiche, référence de transaction obligatoire pour le mobile money, affectation de l'agent qui enregistre). **Date du paiement** : la date qui sert au calcul (carence, délai de grâce) reste **la date du jour fixée par le système** (§12) ; la date déclarée par l'agent (par exemple celle de la transaction mobile money) est gardée **pour information seulement**, et ne peut jamais être postérieure à aujourd'hui.
    - **En attente de la MUNASEB** : suspension, ayants droit (malade et lien de parenté sur la demande), forfaits du paiement direct.
  Écarts avec notre modèle et questions à poser : voir la comparaison du 2026-10-22 (consultation à 100 % chez nous, 80 % sur le bon ; hospitalisation à 100 % sur le bon mais forfait de 50 000 dans la liste ; types d'actes absents ; un bon regroupe plusieurs actes, une demande n'en porte qu'un).
- **Rattacher les contrats et les demandes à une structure, avant d'admettre un second assureur** (noté le 2026-10-18, étape 3.9). Aujourd'hui, un contrat et une demande de remboursement ne disent pas à quelle mutuelle ils appartiennent : MUNASEB est câblée en dur. Un agent ne touche que les demandes de sa mutuelle parce qu'il doit exercer dans une structure MUNASEB validée et que la base n'en admet qu'une (index `structure_une_seule_munaseb_validee`, migration 0012). Avec le futur module « Assurance » générique, plusieurs assureurs existeront : il faudra alors rattacher chaque contrat (et donc ses demandes) à la structure de son assureur, et ne permettre une transition qu'à un agent de cette structure.

---

## 5. Technique, plus tard

- Requêtes SQLx vérifiées à la compilation : après la stabilisation du module MUNASEB.
- Synchronisation de l'application mobile hors ligne avec le serveur : protocole à concevoir.
- Calcul du solde d'une période MUNASEB : regroupé le 2026-10-21 (étape 6), une seule fonction `solde_periode` pour la validation, l'agent et le patient.

### Après le pilote (décision du porteur, 2026-10-21)

Hors du MVP MUNASEB (CLAUDE.md §12, « Périmètre du MVP MUNASEB ») :
- **structures** :
  - reste de la 3.7 pour les autres types de structure (pièces justificatives, hôpital, pharmacie, laboratoire) ;
  - logo de la structure (section 4, « Structures et rôles ») ;
  - suspension et réactivation d'une structure ;
  - liste fermée des régions et provinces (section 4, « Structures et rôles ») ;
- **comptes** :
  - bascule professionnel ↔ patient ;
  - invitations d'administrateurs (l'outil `creer_administrateur` suffit) ;
  - TOTP pour les patients et les professionnels (J10) ;
  - domaines d'email internationaux (section 4, « Structures et rôles ») ;
- **technique** : requêtes SQLx vérifiées à la compilation (ci-dessus) ;
- **MUNASEB** :
  - rapprochement des partenaires avec les structures (section 4, « MUNASEB ») ;
  - paiement direct par le mutualiste ;
  - intégration des API Orange Money et Moov Money pour la cotisation (décision du porteur, 2026-10-22).

---

## 6. Documents à mettre à jour

- **Cahier des charges** :
  - architecture (Rust/Axum, Angular, Flutter) ;
  - rôles d'affectation ;
  - correction de BF-08 (périmètre de la sage-femme) ;
  - BF-01 : comptes séparés ;
  - espace patient sur le web ;
  - structures sous licence et invitations ;
  - le besoin « rappels créés par le patient », s'il est validé.
- **Diagrammes de séquence** : ils ne sont pas dans le dépôt. À y ajouter, puis à vérifier avec les nouveaux rôles et les comptes séparés.
