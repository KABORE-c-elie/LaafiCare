# La sécurité de LaafiCare, expliquée simplement

LaafiCare manipule des **données de santé** : savoir qu'une personne est suivie, qu'elle attend un résultat d'analyse ou qu'elle est enceinte est déjà une information sensible. Chaque choix ci-dessous répond à une question simple, *que se passe-t-il si quelqu'un de mal intentionné essaie ?*, et renvoie à sa source et au fichier de code qui l'applique.

Les fichiers de code cités sont dans [`backend/src/`](../backend/src/) et [`backend/migrations/`](../backend/migrations/). Les termes techniques sont repris dans le [glossaire](glossaire.md).

---

## Sommaire

1. [Les mots de passe](#1-les-mots-de-passe)
2. [Le verrouillage après 5 essais](#2-le-verrouillage-après-5-essais)
3. [Ne rien révéler sur l'existence d'un compte](#3-ne-rien-révéler-sur-lexistence-dun-compte)
4. [Le code reçu par SMS (OTP)](#4-le-code-reçu-par-sms-otp)
5. [Le second facteur (TOTP)](#5-le-second-facteur-totp)
6. [Les jetons de connexion (JWT)](#6-les-jetons-de-connexion-jwt)
7. [Comptes séparés](#7-comptes-séparés)
8. [Ce que la base de données garantit elle-même](#8-ce-que-la-base-de-données-garantit-elle-même)
9. [Les pièces justificatives](#9-les-pièces-justificatives)
10. [Les secrets du serveur](#10-les-secrets-du-serveur)
11. [Ce qui reste à faire avant la mise en service](#11-ce-qui-reste-à-faire-avant-la-mise-en-service)

---

## 1. Les mots de passe

**Le mot de passe n'est jamais enregistré.** Le serveur n'en garde qu'une *empreinte* (un *hachage*) : une transformation à sens unique. On peut vérifier qu'un mot de passe saisi donne la même empreinte, mais on ne peut pas retrouver le mot de passe à partir de l'empreinte. Si la base était volée, les mots de passe ne seraient pas lisibles.

**L'algorithme : Argon2id.** C'est celui que recommande l'OWASP (*Password Storage Cheat Sheet*). Il est volontairement lent et gourmand en mémoire : un attaquant qui voudrait essayer des milliards de mots de passe sur une empreinte volée devrait y consacrer énormément de calcul. Paramètres : 19 Mio de mémoire, 2 passes, 1 fil d'exécution (le minimum OWASP). Chaque empreinte contient un *sel*, une valeur aléatoire propre à chaque compte, pour que deux personnes ayant le même mot de passe n'aient pas la même empreinte.
Fichier : [`mot_de_passe.rs`](../backend/src/mot_de_passe.rs). Source : OWASP, documentation de la bibliothèque `argon2` 0.6.

**Les règles** (source : NIST SP 800-63B, révision 4, §3.1.1.2) :

- de **8 à 128 caractères**, comptés en caractères et non en octets (une lettre accentuée compte pour un) ;
- au moins **une lettre** (accentuée ou non), **un chiffre** et **un caractère spécial** ;
- les espaces sont permises, mais ne comptent pas comme caractère spécial ; les caractères de contrôle (tabulation, retour à la ligne) sont refusés ;
- si une règle n'est pas respectée, le serveur renvoie **la liste de toutes les règles manquantes**, pour que l'application affiche une liste à cocher.

**La normalisation NFC.** Une même lettre accentuée, « é » par exemple, peut être codée de deux façons selon le clavier du téléphone. Sans précaution, un patient ne pourrait plus se connecter depuis un autre appareil. Le mot de passe est donc ramené à une forme unique (*NFC*) avant d'être haché, à la création comme à la connexion.
Source : NIST SP 800-63B rév. 4 ; bibliothèque `unicode-normalization`.

**Deux écarts à la norme NIST, assumés par le porteur du projet :**

1. NIST déconseille d'imposer un mélange de types de caractères. LaafiCare l'impose quand même (lettre, chiffre, caractère spécial).
2. NIST demande 15 caractères pour un mot de passe utilisé seul, et n'en accepte 8 qu'avec un second facteur. Or le second facteur reste facultatif pour les patients et les professionnels.

Ces écarts sont documentés dans le code.

## 2. Le verrouillage après 5 essais

Après **5 essais faux, le compte est bloqué 15 minutes** (exigence BF-01 du cahier des charges). Cela s'applique à tous les comptes : patient, professionnel, administrateur.

**Compté avant d'être vérifié.** Chaque essai est enregistré *avant* que le mot de passe soit contrôlé, en une seule opération de la base. Sinon, dix essais envoyés exactement en même temps pourraient tous passer avant que le compteur ne bloque. Ici, au plus 5 essais sont vérifiés, même s'ils arrivent simultanément.

**Un seul compteur pour le mot de passe et le second facteur.** Quand le second facteur est activé, le bon mot de passe ne remet *pas* le compteur à zéro : seul le bon code le fait. Sinon, quelqu'un qui connaît le mot de passe pourrait alterner sans fin « bon mot de passe, 4 codes au hasard ». La personne dispose donc de 5 essais en tout, mot de passe compris.

Fichier : [`verrouillage.rs`](../backend/src/verrouillage.rs). Source : NIST SP 800-63B rév. 4, §3.2.2 (limitation des essais).

## 3. Ne rien révéler sur l'existence d'un compte

Taper un numéro de téléphone ou un email ne doit jamais apprendre si cette personne utilise LaafiCare : ce serait déjà une information de santé.

- **Une seule réponse** pour un numéro inconnu, une inscription inachevée, un mauvais mot de passe, un compte bloqué ou désactivé : « Numéro ou mot de passe incorrect. » (patients), « Email ou mot de passe incorrect. » (professionnels).
- **Le même temps de réponse.** Quand aucun compte n'existe, le serveur vérifie quand même une fausse empreinte Argon2id. Sinon, une réponse plus rapide trahirait l'absence de compte.
- **Aucun nom affiché avant la connexion**, même pour aider à repérer une faute de frappe.
- **Une erreur de format ne dit rien d'un compte** : un numéro mal écrit est refusé comme tel (« téléphone invalide »), puisqu'aucun compte ne peut avoir ce numéro.

Fichiers : [`verrouillage.rs`](../backend/src/verrouillage.rs), [`auth_patient.rs`](../backend/src/auth_patient.rs).

**Les numéros de téléphone** sont ramenés à une forme unique (`+226` suivi de 8 chiffres pour le Burkina Faso), quelle que soit la façon dont ils sont saisis : espaces, points, tirets, préfixe `00226`. Le SMS part toujours vers le numéro ainsi normalisé.
Fichier : [`telephone.rs`](../backend/src/telephone.rs). Source : plan national de numérotage du Burkina Faso publié par l'UIT.

## 4. Le code reçu par SMS (OTP)

Un *OTP* (*one-time password*, mot de passe à usage unique) est un code à **6 chiffres**, **valable 5 minutes**, envoyé par SMS. Il ne sert qu'aux **patients**, à deux moments :

- **l'inscription** : vérifier que le numéro appartient bien à la personne avant de créer le compte ;
- **le mot de passe oublié** : prouver qu'on détient le téléphone avant d'en choisir un nouveau.

Il n'est **accepté qu'une fois**, puis effacé. Une nouvelle demande remplace l'ancien code.

**Les essais sont limités.** Avec un million de codes possibles, des essais illimités permettraient de trouver le bon code en quelques minutes. Un code n'accepte donc que **5 essais** ; ensuite, tant qu'aucun code juste n'a été saisi, chaque nouveau code n'en accepte qu'**un seul**. Les échecs s'additionnent d'un code à l'autre (NIST interdit de les remettre à zéro avec un nouveau code) et ne reviennent à zéro qu'après un code juste. Chaque essai est compté *avant* d'être vérifié, pour que des essais lancés en même temps ne dépassent pas la limite.

**Les envois sont limités** : **60 secondes** au moins entre deux codes pour un même numéro, et **5 codes par heure**. Un SMS coûte de l'argent, et un envoi sans limite permettrait de harceler quelqu'un. Ces limites s'appliquent à tout numéro, inscrit ou non : elles ne révèlent pas si une personne a un compte. La base ne garde la trace des envois qu'une heure (sans le numéro de téléphone) : les plus anciens sont effacés à chaque envoi.

Le code est enregistré haché (SHA-256), et non avec Argon2id : avec seulement un million de valeurs possibles, un calcul lent n'apporterait rien. La vraie protection est la courte durée de vie, l'usage unique et la limite d'essais.

Fichier : [`otp.rs`](../backend/src/otp.rs). Sources : NIST SP 800-63B rév. 4, §3.1.3 (limitation des essais) ; OWASP *Multifactor Authentication Cheat Sheet* (limites par compte, coût des SMS). Les valeurs (60 s, 5 par heure, 5 essais) sont celles du porteur : aucune des deux sources n'en donne.

## 5. Le second facteur (TOTP)

**Le principe.** En plus du mot de passe, la personne saisit un code à **6 chiffres qui change toutes les 30 secondes**, affiché par une application comme Google Authenticator. Ce code est calculé à partir d'un **secret** partagé une seule fois entre le serveur et le téléphone, au moment de l'activation (par un QR code). Voler le mot de passe ne suffit plus : il faut aussi le téléphone.

**Qui l'utilise** : facultatif pour les patients et les professionnels, **obligatoire pour l'équipe LaafiCare**, qui valide les établissements et accorde les licences.

**Les choix** (fichier : [`totp.rs`](../backend/src/totp.rs) ; sources : RFC 6238, RFC 4226, NIST SP 800-63B rév. 4) :

| Question | Réponse |
|---|---|
| Quel calcul ? | Celui de la norme RFC 6238 (SHA-1, 6 chiffres, 30 secondes), compatible avec toutes les applications courantes. Il est fait par la bibliothèque `totp-rs`, jamais recodé à la main. |
| Quelle taille de secret ? | 160 bits, tirés au hasard par un générateur cryptographique (taille recommandée par la RFC 4226). |
| Et si l'horloge du téléphone dérive ? | Le code précédent et le suivant sont aussi acceptés, soit une marge d'environ 90 secondes. |
| Un code peut-il resservir ? | Non. Le serveur retient la période du dernier code accepté et refuse tout code de cette période ou d'une précédente, même encore valable. |
| Le secret est-il protégé ? | Il est **chiffré** en base (AES-256-GCM), avec une clé qui n'est pas dans la base. Le chiffrement est lié au compte : un secret recopié sur un autre compte ne se déchiffre pas. Une fois déchiffré pour vérifier un code, le secret est **effacé de la mémoire** du serveur dès qu'il ne sert plus (bibliothèque `zeroize`). |
| Que voit l'application d'authentification ? | « LaafiCare » et « Patient », « Professionnel » ou « Administrateur ». Aucun nom ni numéro : ces données pourraient finir dans une sauvegarde en ligne du téléphone. |
| Et sans le téléphone ? | **10 codes de secours**, donnés une seule fois à l'activation, chacun utilisable une seule fois. Ils sont hachés avec Argon2id, comme un mot de passe. |
| Activation | Elle demande le mot de passe, et ne devient effective qu'après un premier code correct. |
| Changement de téléphone | Le nouveau second facteur remplace l'ancien seulement une fois confirmé ; d'ici là, l'ancien fonctionne toujours. L'ancien secret et ses codes de secours sont alors détruits. |
| Téléphone perdu, plus de codes de secours | Seule l'équipe LaafiCare peut désactiver le second facteur, après vérification de la pièce d'identité. La base est prête ; l'outil arrive à l'étape suivante. Le membre de l'équipe s'identifie avec son propre compte, la désactivation est tracée sans pouvoir être effacée, et personne ne peut désactiver le second facteur de ses propres comptes. |
| Et le mot de passe oublié ? | Le réinitialiser par SMS ne retire **jamais** le second facteur : sinon, un vol de carte SIM suffirait à le contourner. |

## 6. Les jetons de connexion (JWT)

Après une connexion réussie, l'application reçoit un **jeton d'accès** (*JWT*) signé par le serveur. Elle le présente à chaque demande, pour ne pas renvoyer le mot de passe à chaque fois. La signature empêche de fabriquer ou de modifier un jeton sans connaître le secret du serveur.

Le secret de signature fait au moins 32 caractères (256 bits) : c'est le minimum de la RFC 7518 §3.2 pour l'algorithme HS256. Le serveur refuse de démarrer avec un secret plus court.

**Huit sortes de jetons**, une par usage (patient, professionnel, administrateur, étape intermédiaire du second facteur…) : chaque route n'en accepte qu'une, pour qu'un jeton ne serve jamais à la place d'un autre (RFC 8725 §3.12).

**Révocation** : chaque compte a un numéro de version de jeton, augmenté à chaque réinitialisation du mot de passe ; les jetons émis avant sont refusés aussitôt.
Fichiers : [`jwt.rs`](../backend/src/jwt.rs), [`extracteur_jwt.rs`](../backend/src/extracteur_jwt.rs), [`config.rs`](../backend/src/config.rs).

**Sessions et jeton de renouvellement** (sources : RFC 9700, *OAuth 2.0 Security Best Current Practice*, §4.14 ; NIST SP 800-63B rév. 4, §2.1.3 et §2.2.3) :

| Question | Réponse |
|---|---|
| Combien de temps vit le jeton d'accès ? | **15 minutes**. Un jeton volé ne sert donc pas longtemps. |
| Comment rester connecté plus longtemps ? | Avec un **jeton de renouvellement**, qui donne une nouvelle paire de jetons. Patient : jusqu'à 30 jours sans retaper le mot de passe (NIST : 30 jours au plus avec un mot de passe seul) ; professionnel : 12 h ; administrateur : 24 h (NIST : 24 h au plus). |
| Et si le jeton de renouvellement est volé ? | Il **change à chaque usage**. Si un ancien jeton revient alors que son successeur a déjà servi, c'est qu'il a été copié : la session est fermée (RFC 9700). Une seule tolérance : 30 secondes pour une réponse perdue sur un réseau instable, une fois. |
| Le serveur garde-t-il les jetons ? | Non : seulement l'**empreinte** SHA-256 du jeton de renouvellement (256 bits aléatoires). Le jeton en clair n'apparaît jamais dans les journaux, et il est effacé de la mémoire après usage. |
| Se déconnecter ? | Chaque connexion ouvre une **session**, une par appareil, vérifiée à chaque demande. On peut fermer celle d'un appareil, ou toutes à la fois ; une réinitialisation du mot de passe les ferme toutes. |

Fichier : [`session.rs`](../backend/src/session.rs).

## 7. Comptes séparés

Une personne peut avoir un compte patient, un compte professionnel et, pour l'équipe LaafiCare, un compte administrateur. **Chaque compte a son propre mot de passe et son propre verrouillage** : bloquer le compte patient ne bloque pas le compte professionnel, et inversement.

Les professionnels se connectent par **email**, les patients par **téléphone**. Le SMS ne sert qu'aux patients.

**L'email d'un professionnel doit être vérifié** : la personne prouve qu'elle reçoit les messages de cette adresse, en ouvrant un lien à usage unique valable 24 heures (source : NIST SP 800-63A rév. 4, §3.8 ; le lien arrive à l'étape 3.7). Tant que ce n'est pas fait, le compte ne peut pas se connecter. La base garantit qu'une identité ne naît jamais vérifiée, et que **tout changement d'email efface la vérification**. Une session professionnelle ouverte s'arrête alors à son prochain renouvellement, au plus tard 15 minutes après. Fichiers : [`auth_professionnel.rs`](../backend/src/auth_professionnel.rs), [`session.rs`](../backend/src/session.rs). Migrations 0016 et 0017.

**À la connexion professionnelle**, les mêmes règles que pour le patient : une seule réponse pour un email inconnu, un mauvais mot de passe, un compte bloqué ou désactivé, avec le même temps de réponse ; verrouillage après 5 essais. Le message « email pas encore vérifié » n'apparaît qu'après un mot de passe correct : seul le détenteur du mot de passe l'apprend. En cas de panne, la réponse ne contient jamais l'email ni le mot de passe saisis, et les journaux non plus.

**Second temps de la connexion professionnelle** (quand le second facteur est actif) : après le mot de passe, la personne reçoit un jeton intermédiaire valable 5 minutes, qui ne sert qu'à envoyer son code (de l'application d'authentification, ou de secours). Règles :

- une seule réponse, « Code incorrect. », pour un code faux, un code de secours faux ou un compte bloqué ;
- chaque code faux compte dans le **même verrouillage que le mot de passe** (5 essais, puis 15 minutes) ;
- le jeton intermédiaire **ne sert qu'une fois** : la base retient l'heure d'émission du dernier jeton utilisé, et refuse tout jeton qui n'est pas plus récent (migration 0018). Deux envois simultanés du même jeton n'ouvrent qu'une session. Un code faux, lui, ne consomme pas le jeton ;
- les heures comparées viennent toutes de l'horloge du serveur d'application, jamais de celle de la base : deux horloges peuvent dériver l'une par rapport à l'autre.

**Connexion d'un administrateur LaafiCare.** Le second facteur y est **obligatoire** : le mot de passe seul ne donne jamais de session. Un administrateur qui n'a plus de second facteur (premier administrateur, ou téléphone perdu) ne peut en enregistrer un nouveau que pendant une **fenêtre de 10 minutes**, ouverte par l'équipe LaafiCare avec un outil local, après avoir vérifié son identité. Sans cette règle, quelqu'un qui aurait volé le mot de passe pourrait enregistrer son propre second facteur et prendre le compte. Hors de la fenêtre, le bon mot de passe reçoit la même réponse qu'un mauvais.

- Durée : NIST SP 800-63B rév. 4, §4.1.3 (un code d'association d'un nouvel authentificateur est valable « a maximum of 10 minutes »). La fenêtre est fixée par le code, jamais par l'outil.
- La fenêtre est vérifiée à chaque étape (connexion, début, confirmation) et **refermée** par la confirmation, dans la même opération que l'activation.
- Le mot de passe est redemandé au début de l'enregistrement, et chaque essai faux compte dans le verrouillage.
- Après la confirmation, aucune session n'est ouverte : la personne se reconnecte avec son nouveau second facteur, par la seule voie qui ouvre une session administrateur.
- Un jeton d'administrateur est refusé dès que son second facteur n'est plus actif, même s'il est encore valable.

Fichiers : [`auth_administrateur.rs`](../backend/src/auth_administrateur.rs), [`extracteur_jwt.rs`](../backend/src/extracteur_jwt.rs). Migration 0019. **Limite** : la base ne peut pas encore interdire à une route d'ouvrir une fenêtre (voir `points-ouverts.md`).

**Travailler dans un établissement (affectation).** Après la connexion, un professionnel choisit l'établissement et le rôle dans lesquels il travaille. Le jeton qu'il reçoit porte ce choix, et **à chaque demande** le serveur vérifie :
- que la session est ouverte et porte bien cette affectation, que le compte est actif et son email toujours vérifié ; sinon, le jeton est refusé ;
- que l'affectation est toujours active, l'établissement validé et sa licence en cours (à la date du jour au Burkina Faso) ; sinon, l'accès est refusé aussitôt, même si le jeton est encore valable.

Changer d'établissement ferme la session en cours et en ouvre une nouvelle, **qui finit à la même heure** : on ne prolonge jamais une session sans redonner son mot de passe. Chaque décision d'un agent MUNASEB est enregistrée avec son affectation (qui, et dans quel établissement). **Un agent ne touche que les demandes de sa mutuelle** : il doit exercer dans une structure MUNASEB validée, et la base n'en admet qu'une seule. Fichiers : [`affectation.rs`](../backend/src/affectation.rs), [`extracteur_jwt.rs`](../backend/src/extracteur_jwt.rs). Migration 0021.

**Les outils de l'équipe LaafiCare** (`creer_premier_administrateur`, `creer_administrateur`, `desactiver_totp`) se lancent sur l'ordinateur du serveur, **jamais par le réseau** :
- `creer_administrateur` et `desactiver_totp` exigent que l'administrateur qui les lance s'authentifie avec son email, son mot de passe et un code de son application ; chaque essai faux compte dans son verrouillage ;
- les mots de passe, les codes et les jetons se tapent **sans s'afficher** (crate `rpassword`), puis sont effacés de la mémoire ;
- un nouvel administrateur **choisit lui-même son mot de passe**, différent de celui de son compte patient ; il enregistre lui-même son second facteur pendant la fenêtre de 10 minutes : **aucun outil n'affiche de secret de second facteur** ;
- `desactiver_totp` exige l'attestation que la CNIB de la personne a été vérifiée ; il supprime le second facteur et ses codes de secours, ferme toutes les sessions du compte, prévient la personne par email, et la base refuse qu'un administrateur l'utilise sur ses propres comptes ;
- **séparation des tâches** : un administrateur ne valide ni ne refuse jamais la demande d'une structure dont il est responsable, même avec un autre compte de la même personne ; un autre membre de l'équipe doit décider. Source : NIST SP 800-53 rév. 5, contrôle AC-5 « Separation of Duties » ; application : décision du porteur, 2026-10-21 ;
- **alertes de sécurité par email** : quand un second facteur est enregistré (réenrôlement d'un administrateur) ou désactivé par l'équipe, la personne reçoit un email, après l'enregistrement en base. Il donne la date et l'heure (heure du Burkina Faso) et l'adresse de l'équipe à contacter (`CONTACT_SECURITE`), sans aucun code ni lien. Un échec d'envoi n'annule rien. Sources : NIST SP 800-63B rév. 4, §4.1.2.1 et §4.6 ; OWASP ASVS 5.0, 6.3.7 ;
- un seul « premier administrateur » peut jamais exister, garanti par la base, même si deux outils sont lancés en même temps ;
- **chaque création, reprise et désactivation est tracée**, sans pouvoir être modifiée ni supprimée.

Fichiers : [`outils_administrateur.rs`](../backend/src/outils_administrateur.rs), [`console.rs`](../backend/src/console.rs), [`src/bin/`](../backend/src/bin/). Migrations 0013 et 0020.

**Vérification d'un email par un code** : un jeton de 256 bits, valable 24 heures et une seule fois (NIST SP 800-63A rév. 4, §3.8), dont seule l'empreinte est gardée. Il ne vérifie que l'adresse à laquelle il a été envoyé : si l'email change entre-temps, il ne vaut plus rien. Un nouveau code annule le précédent. Fichier : [`jeton_email.rs`](../backend/src/jeton_email.rs).

**Envois simulés interdits en production** : en développement, les SMS et les emails ne partent pas, ils s'affichent dans le terminal (les emails ne passent jamais par les journaux, qui ne doivent contenir aucun jeton, OWASP). Le serveur et les outils lisent la variable obligatoire `ENVIRONNEMENT` : en `production`, ils refusent de démarrer tant qu'un envoi est simulé. Fichiers : [`config.rs`](../backend/src/config.rs), [`email.rs`](../backend/src/email.rs), [`sms.rs`](../backend/src/sms.rs).

**La clé de chiffrement des secrets TOTP** est gardée en mémoire dans un type qui l'efface à l'arrêt et qui affiche `***` si on tente de l'écrire dans un journal ; l'état partagé du serveur, qui la contient avec le secret JWT, ne peut pas être affiché du tout. Fichiers : [`config.rs`](../backend/src/config.rs), [`main.rs`](../backend/src/main.rs).

Un établissement n'a **accès à aucune donnée de patient tant que l'équipe LaafiCare ne l'a pas validé**.

## 8. Ce que la base de données garantit elle-même

Beaucoup de règles sont vérifiées deux fois : par le code, et par la base PostgreSQL elle-même. Même une erreur de programmation ne peut alors pas les contourner. Quelques exemples :

- **Historiques non modifiables** : les changements de statut d'une demande de remboursement, les décisions sur les établissements et les désactivations de second facteur ne peuvent être ni modifiés ni supprimés (règles appelées *triggers*, migrations 0010, 0012 et 0013).
- **Un seul second facteur actif par compte**, un secret chiffré de la bonne taille, un code de secours rattaché à son second facteur (migration 0013).
- **Un seul établissement validé par numéro d'autorisation**, et une seule MUNASEB validée (migration 0012).

**Le serveur n'a pas les droits pour défaire ces protections** (migration 0026, 2026-10-22). Il se connecte avec un rôle limité, `laaficare_serveur` : ni administrateur de la base, ni propriétaire des tables. Il ne peut donc ni désactiver un trigger, ni modifier ou effacer une trace, ni créer ou changer une table, ni écrire le journal des migrations. Les outils de l'équipe ont leur propre rôle, `laaficare_outils`, **seul autorisé à ouvrir la fenêtre de réenrôlement** d'un administrateur ; le serveur peut seulement la refermer, par une fonction prévue pour cela. Les migrations sont appliquées à part (`cargo run --bin migrer`), avec le rôle propriétaire. Un test vérifie ces refus et fait tourner un parcours complet du serveur avec son seul rôle ([`tests/droits_des_roles.rs`](../backend/tests/droits_des_roles.rs)).

**Limite connue** : le rôle propriétaire, lui, peut tout faire. Il ne doit servir qu'aux migrations ; en production, son mot de passe reste sur le poste qui les applique, jamais sur le serveur.

**Les migrations elles-mêmes sont protégées** : un test refuse tout caractère invisible ou trompeur (espace insécable, tiret qui ressemble au tiret simple) dans les fichiers de migration, et Git y impose des fins de ligne identiques sur toutes les machines. Sinon, une migration pourrait changer à l'insu de tous, et le serveur refuserait de démarrer en production.
Fichiers : [`tests/caracteres_migrations.rs`](../backend/tests/caracteres_migrations.rs), [`.gitattributes`](../.gitattributes).

## 9. Les pièces justificatives

Les établissements déposent leurs autorisations officielles (PDF, JPEG ou PNG, 5 Mo au plus). Règles prévues, inspirées de l'OWASP (*File Upload Cheat Sheet*) :

- le type du fichier est reconnu par ses **premiers octets**, jamais par son nom ou par ce que déclare l'expéditeur ;
- le nom d'origine est remplacé par un identifiant, et une **empreinte** (SHA-256) est enregistrée ;
- les fichiers sont stockés dans la base, **jamais à une adresse publique**, et seule l'équipe LaafiCare peut les télécharger ;
- le contenu n'est lu que par la route de téléchargement, jamais par les listes.

Structure en place : migration 0012. Les routes viendront à une étape ultérieure.

## 10. Les secrets du serveur

Les secrets (accès à la base, secret des jetons, clé de chiffrement du second facteur) ne sont **jamais dans le code ni dans Git**. Ils sont lus au démarrage dans des variables d'environnement, en développement depuis un fichier `backend/.env` qui n'est jamais envoyé dans Git.

Le modèle [`backend/.env.example`](../backend/.env.example) a des valeurs volontairement inutilisables (`A_GENERER`) : le serveur refuse de démarrer tant que celles de `DATABASE_URL`, `JWT_SECRET` et `TOTP_CLE_CHIFFREMENT` ne sont pas remplacées. `POSTGRES_PASSWORD`, lu par Docker et non par le serveur, n'est pas contrôlé : il faut le choisir soi-même.

La clé de chiffrement du second facteur doit être **sauvegardée à part** en production : la perdre rendrait tous les seconds facteurs inutilisables.

## 11. Ce qui reste à faire avant la mise en service

- une **liste de mots de passe interdits** (les plus courants ou déjà divulgués), exigée par NIST ;
- une **limite du nombre de demandes par adresse IP** sur les connexions et les réinitialisations ;
- le choix du **stockage du jeton de renouvellement dans un navigateur** (applications web) ;
- le chiffrement du disque de la base, un antivirus et un nettoyage des fichiers déposés ;
- un fournisseur de SMS réel ;
- la vérification des requêtes SQL dès la compilation.

La liste complète est dans [`points-ouverts.md`](points-ouverts.md).

## Sources

- NIST SP 800-63B, révision 4 (26 août 2025), *Digital Identity Guidelines: Authentication and Authenticator Management*.
- OWASP, *Password Storage Cheat Sheet* et *File Upload Cheat Sheet*.
- IETF : RFC 4226 (HOTP), RFC 6238 (TOTP), RFC 7518 (algorithmes des JWT).
- UIT, plan national de numérotage du Burkina Faso (communication de l'ARCEP, 4 mai 2023).
- Documentation PostgreSQL 18 (triggers, contraintes, expressions régulières) ; documentation Git (gitattributes).
- Documentation et code source des bibliothèques Rust : `argon2` 0.6, `totp-rs` 6.0.0, `aes-gcm` 0.11.1, `jsonwebtoken`, `unicode-normalization`, `rand`.
