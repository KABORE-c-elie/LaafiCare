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

**Noté le 2026-10-15 (étape 3.6.4, migration 0019).** Seul un outil local de l'équipe doit pouvoir ouvrir la fenêtre de 10 minutes pendant laquelle un administrateur sans second facteur en enregistre un nouveau (`compte.reenrolement_autorise_jusqu_au`). Aujourd'hui, c'est garanti **par le code seulement** : le serveur et les outils utilisent le même compte PostgreSQL. **En production**, un compte PostgreSQL séparé pour les outils locaux, **seul autorisé à écrire cette colonne** (droits par colonne de PostgreSQL), garantirait par la base qu'aucune route ne peut ouvrir une fenêtre. À traiter avec le compte PostgreSQL sans droits d'administration (section 3).

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

### Application web (noté le 2026-10-20)

Détail dans `web/docs/decisions.md` (branche `web`) :
- **Trusted Types** : ils bloquent l'enregistrement du service worker d'Angular (le travail hors connexion). Leur activation dans la politique de sécurité du contenu (CSP) est à résoudre.
- **Styles en ligne de la CDK d'Angular** : une CSP stricte, qui interdit les styles en ligne, les refuse. L'affichage reste correct, mais la console signale ces blocages ; leur traitement est à décider.
- **Faille de `vite`** : elle ne touche que le serveur de développement, jamais l'application déployée. En attente d'`@angular/build` 22.2.4.

### À faire dans les applications web et mobile (étape 3.7)

- **Page `/verifier-email`** (application web) : lecture du jeton après `#`, bouton de confirmation, envoi par `POST`, jamais par un GET ; contrat dans `docs/api.md`, sections 2.7 et 2.8.
- **Avertissement sous le mot de passe professionnel** du formulaire de création de structure : « Choisissez un mot de passe différent de celui de votre compte patient ». C'est un conseil, pas une obligation : le serveur accepte un mot de passe identique (décision du porteur, 2026-10-21).
- **Aide sous les champs email** (email de la structure, email professionnel) : « Sans accent ni espace » (adresses en ASCII seulement, décision du porteur, 2026-10-21).

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

- Rôle PostgreSQL sans droits d'administration pour le serveur : sinon, les protections des historiques peuvent être désactivées.
- Chiffrement du disque de la base.
- Fournisseurs réels : SMS (codes, invitations) et email (réinitialisation, vérification de l'email). Crate et fournisseur à présenter.
- Fichiers déposés par les structures : réécriture des images, antivirus, désactivation du contenu actif des PDF (OWASP).

### Avant le pilote : décisions extérieures à prendre tôt

**Noté le 2026-10-21.** Ces décisions viennent d'ailleurs que du code, et une étape du MVP les attend :
- **durée de la licence d'une structure** : illimitée, ou à renouveler (étape 3.8) ;
- **règles de suspension d'un contrat MUNASEB** : à demander à la MUNASEB (étape 5) ;
- **valeurs des types d'actes manquants** : dentaire et prothèses, accouchement et consultations prénatales, chirurgie, frais funéraires, radiologie (étape 7, mémoire MUNASEB).

### Éléments à découper en étapes

**Noté le 2026-10-15.** La mise en production n'est pas encore découpée en étapes, comme le sont celles du backoffice (CLAUDE.md, section 12). Voici les éléments déjà connus, repris de ce document, à découper plus tard en étapes ordonnées, chacune présentée et validée avant d'être faite :

- **Cadre légal** : formalité auprès de la CIL (déclaration ou autorisation), hébergement des données de santé dans le pays, aucun transfert vers l'étranger sans autorisation (section 3, « Cadre légal »).
- **Comptes PostgreSQL** : un compte sans droits d'administration pour le serveur ; un compte séparé pour les outils locaux, seul autorisé à écrire la fenêtre de réenrôlement (section 2).
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
- Pièces justificatives exigées pour la MUNASEB, les hôpitaux, les pharmacies et les laboratoires.
- **Découpage administratif (région, province, commune)** : texte libre pour l'instant (migration 0022). Recherche du 2026-10-20 : le compte rendu officiel du Conseil des ministres du 2 juillet 2025 (site de la Présidence du Faso) annonce le passage de 13 à 17 régions et de 45 à 47 provinces, et donne les noms des 17 régions avec leurs chefs-lieux ; mais ni le texte des décrets, ni leur numéro, ni la liste complète des provinces et des communes n'ont été trouvés (une période transitoire de six mois y est aussi annoncée). Fermer les listes demande le décret publié au Journal officiel.
- **Position GPS d'une structure** : contrôlée dans le rectangle donné par l'Institut géographique du Burkina (rapport d'août 2014 aux Nations unies, UN-GGIM : 8° à 16° de latitude nord, 6° ouest à 3° est). Ce rectangle déborde sur les pays voisins ; un contrôle par le contour du territoire demanderait une source officielle de ce contour.
- **Adresses email en ASCII seulement** (décision du porteur, 2026-10-21) : les domaines internationaux (`exemplé.bf`, `例え.jp`) et les parties locales accentuées sont refusés, par le code (`adresse_email.rs`) et par la base (migration 0022). À revoir si un utilisateur en a besoin : il faudra alors une migration, et une règle de comparaison des majuscules qui ne dépende pas des réglages de langue de la base (OWASP : convertir les domaines internationaux en punycode pour les comparer).
- **Validation d'une seconde MUNASEB** : fait à l'étape 3.8 réduite (2026-10-21), 409 `munaseb_deja_validee`, contrôlé avant l'écriture, puis l'erreur de l'index `structure_une_seule_munaseb_validee` traduite en ce même code.
- **Corriger et soumettre à nouveau une demande refusée** (retour en brouillon, CLAUDE.md §14) : hors de la 3.8 réduite (V10). En attendant, le responsable abandonne sa demande et en crée une nouvelle.
- **Prévenir l'invitant** quand une invitation est acceptée ou refusée (I17, 2026-10-21) : rien pour l'instant, il le voit dans sa liste.
- **Longueur du SMS d'invitation** (2026-10-21) : le texte fixe fait 229 caractères, plus le nom de la structure. Il contient `ê`, `«` et `»`, absents de l'alphabet GSM 7 bits : il part donc en UCS-2, soit 4 SMS au moins pour un nom court (3GPP TS 23.038 : 70 caractères UCS-2 dans un SMS seul ; TS 23.040 §9.2.3.24.1 : 67 par partie d'un SMS en plusieurs parties). **Le nom d'une structure n'a pas de longueur maximale** : le SMS non plus. À décider avec le fournisseur réel (coût).
- **Avertissement des numéros d'autorisation en double** dans le détail administrateur (§14) : à faire avec les pièces justificatives (après le pilote, sauf pour la MUNASEB, qui n'en a pas).
- **Logo de la structure** (demande du porteur, 2026-10-21) : reporté à la suite de la 3.7. Facultatif, route séparée, PNG et JPEG seulement (jamais SVG), contenu vérifié, image réencodée par le serveur, nom de fichier choisi par le serveur, stockage au Burkina, seul le responsable peut le changer. Proposition à présenter avant d'écrire (OWASP File Upload Cheat Sheet).
- **Second facteur à la création d'une structure** : un professionnel qui a un TOTP actif reçoit 409 `second_facteur_requis` (aucun professionnel ne peut en avoir avant l'étape 3.11). À concevoir avec l'activation du TOTP par les professionnels.
- Où ranger le numéro d'ordre et la spécialité d'un professionnel : ils appartiennent à la personne, pas à l'établissement (futur profil professionnel).
- Création d'un compte pour une autre personne : patient sans téléphone, nouveau-né, urgence (patient inconscient). Qui en a le droit, et comment la personne en reprend ensuite le contrôle.
- Major de laboratoire : rôle séparé, ou autorité sur le stock pour un biologiste.

### MUNASEB

- Suspension d'un contrat : à concevoir.
- Types d'actes sans valeur dans les sources : dentaire et prothèses, accouchement et consultations prénatales, chirurgie, frais funéraires, radiologie.
- Partenaires de la MUNASEB et structures : à rapprocher (un partenaire deviendra un lien entre la MUNASEB et une structure).
- **Rattacher les contrats et les demandes à une structure, avant d'admettre un second assureur** (noté le 2026-10-18, étape 3.9). Aujourd'hui, un contrat et une demande de remboursement ne disent pas à quelle mutuelle ils appartiennent : MUNASEB est câblée en dur. Un agent ne touche que les demandes de sa mutuelle parce qu'il doit exercer dans une structure MUNASEB validée et que la base n'en admet qu'une (index `structure_une_seule_munaseb_validee`, migration 0012). Avec le futur module « Assurance » générique, plusieurs assureurs existeront : il faudra alors rattacher chaque contrat (et donc ses demandes) à la structure de son assureur, et ne permettre une transition qu'à un agent de cette structure.

---

## 5. Technique, plus tard

- Requêtes SQLx vérifiées à la compilation : après la stabilisation du module MUNASEB.
- Synchronisation de l'application mobile hors ligne avec le serveur : protocole à concevoir.
- Calcul du solde d'une période MUNASEB, présent en double (demandes et contrat) : à regrouper.

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
  - paiement direct par le mutualiste.

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
