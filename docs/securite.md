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

Le code est enregistré haché (SHA-256), et non avec Argon2id : avec seulement un million de valeurs possibles, un calcul lent n'apporterait rien. La vraie protection est la courte durée de vie et l'usage unique.

Fichier : [`otp.rs`](../backend/src/otp.rs). Source : NIST SP 800-63B.

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
| Le secret est-il protégé ? | Il est **chiffré** en base (AES-256-GCM), avec une clé qui n'est pas dans la base. Le chiffrement est lié au compte : un secret recopié sur un autre compte ne se déchiffre pas. |
| Que voit l'application d'authentification ? | « LaafiCare » et « Patient », « Professionnel » ou « Administrateur ». Aucun nom ni numéro : ces données pourraient finir dans une sauvegarde en ligne du téléphone. |
| Et sans le téléphone ? | **10 codes de secours**, donnés une seule fois à l'activation, chacun utilisable une seule fois. Ils sont hachés avec Argon2id, comme un mot de passe. |
| Activation | Elle demande le mot de passe, et ne devient effective qu'après un premier code correct. |
| Changement de téléphone | Le nouveau second facteur remplace l'ancien seulement une fois confirmé ; d'ici là, l'ancien fonctionne toujours. L'ancien secret et ses codes de secours sont alors détruits. |
| Téléphone perdu, plus de codes de secours | Seule l'équipe LaafiCare peut désactiver le second facteur, après vérification de la pièce d'identité. La base est prête ; l'outil arrive à l'étape suivante. Le membre de l'équipe s'identifie avec son propre compte, la désactivation est tracée sans pouvoir être effacée, et personne ne peut désactiver le second facteur de ses propres comptes. |
| Et le mot de passe oublié ? | Le réinitialiser par SMS ne retire **jamais** le second facteur : sinon, un vol de carte SIM suffirait à le contourner. |

## 6. Les jetons de connexion (JWT)

Après une connexion réussie, l'application reçoit un **jeton** (*JWT*) signé par le serveur. Elle le présente à chaque demande, pour ne pas renvoyer le mot de passe à chaque fois. La signature empêche de fabriquer ou de modifier un jeton sans connaître le secret du serveur.

Le secret de signature fait au moins 32 caractères (256 bits) : c'est le minimum de la RFC 7518 §3.2 pour l'algorithme HS256. Le serveur refuse de démarrer avec un secret plus court.
Fichiers : [`jwt.rs`](../backend/src/jwt.rs), [`config.rs`](../backend/src/config.rs).

Prévu à l'étape suivante : chaque compte aura un numéro de version de jeton, augmenté à chaque changement sensible (mot de passe réinitialisé, bascule entre comptes). Les anciens jetons seront alors refusés immédiatement.

## 7. Comptes séparés

Une personne peut avoir un compte patient, un compte professionnel et, pour l'équipe LaafiCare, un compte administrateur. **Chaque compte a son propre mot de passe et son propre verrouillage** : bloquer le compte patient ne bloque pas le compte professionnel, et inversement.

Les professionnels se connectent par **email**, les patients par **téléphone**. Le SMS ne sert qu'aux patients.

Un établissement n'a **accès à aucune donnée de patient tant que l'équipe LaafiCare ne l'a pas validé**.

## 8. Ce que la base de données garantit elle-même

Beaucoup de règles sont vérifiées deux fois : par le code, et par la base PostgreSQL elle-même. Même une erreur de programmation ne peut alors pas les contourner. Quelques exemples :

- **Historiques non modifiables** : les changements de statut d'une demande de remboursement, les décisions sur les établissements et les désactivations de second facteur ne peuvent être ni modifiés ni supprimés (règles appelées *triggers*, migrations 0010, 0012 et 0013).
- **Un seul second facteur actif par compte**, un secret chiffré de la bonne taille, un code de secours rattaché à son second facteur (migration 0013).
- **Un seul établissement validé par numéro d'autorisation**, et une seule MUNASEB validée (migration 0012).

**Limite connue** : ces protections tiennent contre les erreurs, pas contre un administrateur de la base. En production, le serveur devra donc se connecter avec un compte PostgreSQL sans droits d'administration.

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

Le modèle [`backend/.env.example`](../backend/.env.example) a des valeurs volontairement inutilisables (`A_GENERER`) : le serveur refuse de démarrer tant qu'elles ne sont pas remplacées.

La clé de chiffrement du second facteur doit être **sauvegardée à part** en production : la perdre rendrait tous les seconds facteurs inutilisables.

## 11. Ce qui reste à faire avant la mise en service

- une **liste de mots de passe interdits** (les plus courants ou déjà divulgués), exigée par NIST ;
- une **limite d'envois de SMS** par numéro : aujourd'hui, rien n'empêche de faire envoyer de nombreux codes à un même numéro ;
- la vérification du **numéro de version des jetons** ;
- un compte PostgreSQL sans droits d'administration, le chiffrement du disque de la base, un antivirus et un nettoyage des fichiers déposés ;
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
