# Points ouverts

Ce document liste ce qui reste à décider ou à construire avant que LaafiCare puisse servir de vrais patients. Pour chaque point : ce qui manque, pourquoi c'est un risque, et les pistes connues. Les points sont rangés du plus urgent au moins urgent.

Les décisions déjà prises sont dans `CLAUDE.md` (sections 12 et 14), qui reste la référence. Ce document est mis à jour à chaque décision qui ferme ou ouvre un point.

État au 2026-10-11.

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

Une *session* correspond à une connexion sur un appareil. **Règle décidée le 2026-10-12** : à chaque nouvelle connexion, les sessions terminées (expirées ou révoquées) depuis plus de **30 jours** sont effacées. **Exception** : une session coupée parce qu'un vol de jeton a été détecté (motif `rejeu_detecte`) est gardée **6 mois**, comme trace. À vérifier avec la CIL, avec les autres durées de conservation (section 3).

---

## 2. Sécurité, avant la production

### Déconnexion — fermé le 2026-10-14

Réglé par l'option 3, décidée le 2026-10-12 : jetons d'accès de 15 minutes, jeton de renouvellement qui change à chaque usage et révocable, une session par appareil vérifiée à chaque demande. Déconnexion d'un appareil (`POST /api/sessions/deconnexion`) ou de tous (`POST /api/sessions/deconnexion-partout`) : voir `docs/api.md`, section 1.7. Reste ouvert : le **stockage du jeton de renouvellement dans un navigateur** (applications web), à décider à part.

### Empreinte du code reçu par SMS

Le code est enregistré en SHA-256 simple. Avec seulement un million de codes possibles, une copie volée de la base permettrait de retrouver un code encore valable.

- **Option à étudier** : une empreinte HMAC calculée avec une clé secrète du serveur, sur le modèle de la clé du second facteur (`TOTP_CLE_CHIFFREMENT`).
- **Risque faible** : un code ne vit que 5 minutes et n'accepte que quelques essais.

### Autres points de sécurité

- **Limite du nombre de demandes par adresse IP** sur les routes de connexion et de réinitialisation (patients, professionnels, administrateurs).
- **Liste de mots de passe interdits** (exigée par la norme NIST SP 800-63B rév. 4) : source à choisir, liste intégrée ou service externe.
- **Sauvegarde de la clé `TOTP_CLE_CHIFFREMENT`**, à part et en lieu sûr, jamais avec la base. La perdre rendrait tous les seconds facteurs inutilisables.
- **Outils de l'équipe LaafiCare**, à concevoir :
  - changer l'email d'un professionnel qui a perdu l'accès à sa boîte, après vérification de sa pièce d'identité (CNIB), avec une trace ;
  - déconnecter de force un compte compromis ;
  - changer le numéro de CNIB d'un administrateur (renouvellement de la carte, future carte de l'AES).
- **Désactivation du second facteur par les structures validées** (vérification de la CNIB à l'accueil) : à rediscuter. En V1, seule l'équipe LaafiCare le désactive.

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
- Liste officielle des provinces ; bornes géographiques du Burkina Faso, pour contrôler la position GPS d'une structure.
- Où ranger le numéro d'ordre et la spécialité d'un professionnel : ils appartiennent à la personne, pas à l'établissement (futur profil professionnel).
- Création d'un compte pour une autre personne : patient sans téléphone, nouveau-né, urgence (patient inconscient). Qui en a le droit, et comment la personne en reprend ensuite le contrôle.
- Major de laboratoire : rôle séparé, ou autorité sur le stock pour un biologiste.

### MUNASEB

- Suspension d'un contrat : à concevoir.
- Types d'actes sans valeur dans les sources : dentaire et prothèses, accouchement et consultations prénatales, chirurgie, frais funéraires, radiologie.
- Partenaires de la MUNASEB et structures : à rapprocher (un partenaire deviendra un lien entre la MUNASEB et une structure).

---

## 5. Technique, plus tard

- Requêtes SQLx vérifiées à la compilation : après la stabilisation du module MUNASEB.
- Synchronisation de l'application mobile hors ligne avec le serveur : protocole à concevoir.
- Calcul du solde d'une période MUNASEB, présent en double (demandes et contrat) : à regrouper.

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
