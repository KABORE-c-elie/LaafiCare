# API LaafiCare : contrat entre le backend et les applications

Ce document décrit les routes de l'API Rust/Axum telles qu'elles sont **codées aujourd'hui**, puis celles qui sont **seulement prévues**. Les applications Angular (web) et Flutter (mobile) s'appuient sur lui. Il est mis à jour avec chaque ajout ou modification d'une route.

État au 2026-11-16, branche `backend-pharmacie`.

Toutes les valeurs des exemples sont fictives.

---

## 1. Règles communes

### 1.1 Adresse et format

- Adresse de développement : `http://127.0.0.1:8080` (port fixé par `SERVER_PORT`). Utiliser `127.0.0.1`, pas `localhost` (voir le README).
- Les corps de requête et de réponse sont en **JSON**, avec l'en-tête `Content-Type: application/json` sur toute requête qui envoie un corps.
- Les noms de champs sont en `snake_case`.
- Dates : `AAAA-MM-JJ` (ex. `2026-10-08`). Dates avec heure : format RFC 3339 en UTC (ex. `2026-10-08T09:30:00Z`).
- Identifiants : UUID sous forme de texte (ex. `0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55`).
- Montants : entiers en FCFA, sans décimale.
- **Identifiant de requête** : chaque réponse porte l'en-tête **`X-Request-ID`**, un UUID tiré par le serveur. L'application l'affiche (ou le garde) quand elle montre une erreur `erreur_interne` : avec ce numéro, l'équipe retrouve la requête dans les journaux. Un `X-Request-ID` envoyé par l'application est **ignoré** (jamais recopié dans les journaux ni dans la réponse).
- **Taille d'un corps de requête** : **256 Kio au plus**, sauf le dépôt d'une pièce justificative (5 Mio, section 2.8). Au-delà : **413**, code `corps_illisible` (ou `fichier_trop_grand` pour une pièce). La plus grosse requête JSON de l'API (vente ou inventaire de 100 lignes) fait environ 10 Ko.

### 1.2 Jeton d'authentification

Les routes protégées attendent l'en-tête :

```
Authorization: Bearer <jeton>
```

Le jeton est un JWT signé en HS256. Il contient :

| Champ | Contenu |
|---|---|
| `sub` | identifiant du **compte** (patient, professionnel ou administrateur) |
| `aud` | sorte de jeton (tableau ci-dessous) |
| `ver` | version du jeton du compte : un jeton émis avant une réinitialisation du mot de passe ou une bascule sera refusé |
| `aff` | affectation active (structure et rôle), seulement pour `laaficare:professionnel_en_exercice` |
| `sid` | session de l'appareil (section 1.7), seulement pour les quatre sortes de 15 min ; une session fermée refuse aussitôt ses jetons |
| `iat`, `exp` | date d'émission et d'expiration (secondes depuis 1970) |

Il existe **huit sortes de jetons**. Chaque route n'en accepte qu'une : un jeton d'une autre sorte est refusé en 401 `jeton_invalide`, même s'il est encore valable.

| Sorte (`aud`) | Sert à | Durée | État |
|---|---|---|---|
| `laaficare:patient` | routes du patient (`GET /api/patients/moi`, `deconnexion-partout`) | 15 min | émis par l'inscription, la connexion et le renouvellement ; refusé après une réinitialisation du mot de passe ou la fermeture de sa session |
| `laaficare:professionnel_sans_affectation` | lister ses affectations, en activer une | 15 min | émis par la connexion professionnelle (section 2.4) ; routes des affectations à l'étape 3.9 |
| `laaficare:professionnel_en_exercice` | routes métier (remboursements MUNASEB…) | 15 min | émis par l'activation d'une affectation (section 2.6) ; refusé (403) dès que l'affectation est désactivée, l'établissement suspendu ou la licence expirée |
| `laaficare:administrateur` | routes de l'équipe LaafiCare | 15 min | émis par le second temps de la connexion administrateur (section 2.5) ; routes de l'équipe à l'étape 3.8 ; refusé dès que le TOTP du compte n'est plus actif |
| `laaficare:second_facteur_patient`, `…_professionnel`, `…_administrateur` | saisir le code du second facteur, après un mot de passe correct | 5 min | professionnel et administrateur : émis (sections 2.4 et 2.5), **une seule utilisation réussie** ; patient : après l'étape 3.11 |
| `laaficare:reenrolement_administrateur` | enregistrer un nouveau second facteur, rien d'autre | 10 min | émis par la connexion administrateur (section 2.5), seulement pendant une fenêtre ouverte par l'équipe ; refusé dès qu'un TOTP est actif ou que la fenêtre a expiré |

Aucune tolérance après l'expiration : un jeton expiré d'une seconde est refusé.

Les applications ne doivent **pas** lire le contenu du jeton pour décider de ce qu'elles affichent : elles le gardent et le renvoient tel quel.

Refus d'une route protégée (RFC 6750) :

| Code HTTP | `code` | Cas | En-tête renvoyé |
|---|---|---|---|
| 401 | `authentification_requise` | en-tête `Authorization` absent ou mal formé | `WWW-Authenticate: Bearer` |
| 401 | `jeton_invalide` | signature fausse, jeton expiré ou illisible, ou **jeton d'une autre sorte** que celle attendue par la route | `WWW-Authenticate: Bearer error="invalid_token"` |
| 403 | `acces_refuse` | jeton de la bonne sorte, mais droit retiré depuis l'émission (ex. affectation désactivée) | — |
| 500 | `erreur_interne` | panne pendant la vérification | — |

Les messages restent volontairement vagues : ils n'aident pas à deviner pourquoi un jeton est refusé.

### 1.3 Format des erreurs

Toute réponse d'erreur, sans exception, a cette forme :

```json
{ "erreur": "message lisible", "code": "code_machine" }
```

- **Les applications testent `code`, jamais `erreur`.** Le message peut changer ; le code, non.
- Certains codes ajoutent un champ :

| `code` | Champ ajouté | Contenu |
|---|---|---|
| `mot_de_passe_non_conforme` | `regles_non_respectees` | liste des règles non respectées (voir 1.5) |
| `en_carence` | `debut_couverture` | date de début de couverture (`AAAA-MM-JJ`) |
| `transition_invalide` | `statut_actuel` | statut actuel de la demande |

- `erreur_interne` (500) ne donne jamais de détail : il reste dans les journaux du serveur.
- `trop_de_demandes` (429) peut s'accompagner de l'**en-tête** `Retry-After` (nombre de secondes à attendre), jamais d'un champ du corps (voir 2.2.5).

### 1.4 Erreurs de lecture de la requête

Ces erreurs viennent de la lecture de la requête, avant tout traitement. Elles peuvent survenir sur toute route qui lit un corps, un paramètre d'URL ou des paramètres de requête. Le code HTTP est celui choisi par axum.

| `code` | Code HTTP habituel | Cas |
|---|---|---|
| `json_invalide` | 400 | JSON mal formé |
| `donnees_invalides` | 422 | champ manquant, type faux, valeur inconnue (ex. type d'acte) |
| `content_type_invalide` | 415 | `Content-Type: application/json` absent |
| `corps_illisible` | 400, 413 | corps illisible pour une autre raison ; 413 : corps de plus de 256 Kio (section 1.1) |
| `chemin_invalide` | 400 | paramètre d'URL invalide (ex. identifiant qui n'est pas un UUID) |
| `parametre_invalide` | 400 | paramètre de requête invalide (après le `?`) |

### 1.5 Règles du mot de passe

Contrôlées à la création et au changement d'un mot de passe, jamais à la connexion. Valeurs possibles de `regles_non_respectees` :

| Valeur | Règle |
|---|---|
| `longueur_min` | au moins 8 caractères |
| `longueur_max` | au plus 128 caractères |
| `caractere_de_controle` | aucun caractère de contrôle (tabulation, retour à la ligne…) |
| `lettre` | au moins une lettre (accentuée ou non) |
| `chiffre` | au moins un chiffre de 0 à 9 |
| `caractere_special` | au moins un caractère qui n'est ni lettre, ni chiffre, ni espace |

Toutes les règles non respectées sont renvoyées ensemble, pour que l'application affiche une liste à cocher. Les espaces sont permises mais ne comptent pas comme caractère spécial. Conseiller `!`, `@` ou `#` plutôt que des emojis.

### 1.6 Numéro de téléphone

Le serveur normalise le numéro saisi : il retire les espaces, les points et les tirets, et ajoute `+226` devant un numéro burkinabè à 8 chiffres. Formes acceptées (exemples fictifs) : `70 00 00 00`, `70.00.00.00`, `+226 70000000`, `0022670000000`, `22670000000`. Un numéro étranger commence par `+` et compte 7 à 15 chiffres. Le 0 initial d'un numéro national fait partie du numéro et n'est jamais retiré. Tout autre format → 422 `telephone_invalide`.

L'application peut donc envoyer le numéro tel que saisi.

### 1.7 Sessions et jeton de renouvellement

> **En service depuis le 2026-10-14** pour les comptes patients, et depuis le 2026-10-15 pour les professionnels et les administrateurs (sections 2.4 et 2.5). L'ancienne réponse `{"jeton": "<JWT>"}` n'existe plus.

**Le principe.** Après la connexion, l'application reçoit deux jetons :
- un **jeton d'accès** (`jeton_acces`), de courte durée, à présenter à chaque demande (`Authorization: Bearer …`) ;
- un **jeton de renouvellement** (`jeton_renouvellement`), qui sert uniquement à obtenir une nouvelle paire quand le jeton d'accès expire. Il **change à chaque usage** : l'ancien ne sert plus. Le serveur n'en garde qu'une empreinte.

Chaque connexion ouvre une **session** (une par appareil). Le jeton d'accès porte l'identifiant de sa session : une session déconnectée refuse aussitôt ses jetons.

| Compte | Jeton d'accès | Sans renouvellement pendant… | Au plus tard, mot de passe redemandé après… |
|---|---|---|---|
| Patient | 15 min | 30 jours | 30 jours |
| Professionnel | 15 min | 1 h | 12 h |
| Administrateur | 15 min | 1 h | 24 h |

**Réponse de la connexion et de l'inscription** :

```json
{
  "etape": "complete",
  "jeton_acces": "<JWT>",
  "expire_dans": 900,
  "jeton_renouvellement": "<64 caractères>"
}
```

- `expire_dans` : durée de vie du jeton d'accès, en secondes.
- `jeton_renouvellement` : 64 caractères hexadécimaux.
- Plus tard, si le second facteur est actif : `{"etape": "second_facteur", "jeton_intermediaire": "<JWT>"}`, puis la paire après la saisie du code. Les patients ne peuvent pas encore activer le second facteur (après l'étape 3.11) : aujourd'hui, `etape` vaut toujours `complete`. L'application doit quand même tester `etape`.
- L'inscription (201) renvoie la même forme, avec `"etape": "complete"`.

**Les trois routes :**

| Route | Appelant | Corps | Réponses |
|---|---|---|---|
| `POST /api/sessions/renouveler` | tout le monde, **sans** en-tête `Authorization` (le jeton d'accès peut être expiré) | `{"jeton_renouvellement": "…"}` | 200 : nouvelle paire, même forme que ci-dessus sans `etape` ; 401 `jeton_invalide` : jeton inconnu, expiré, session révoquée ou expirée, vol détecté, ou (professionnel, administrateur) email qui n'est plus vérifié, auquel cas la session est fermée |
| `POST /api/sessions/deconnexion` | tout le monde | `{"jeton_renouvellement": "…"}` | **toujours 204**, même pour un jeton inconnu : se déconnecter n'échoue jamais |
| `POST /api/sessions/deconnexion-partout` | un jeton d'accès **valide** (si expiré : renouveler d'abord) : patient, professionnel avec ou sans affectation, ou administrateur | aucun | 204 : toutes les sessions du compte sont fermées, sur tous les appareils ; 401 comme pour toute route protégée (section 1.2) |

`deconnexion-partout` ne vérifie que la session et le compte : un professionnel dont l'affectation vient d'être désactivée peut quand même se déconnecter partout.

Erreurs de lecture de la requête (corps JSON mal formé, champ manquant) : section 1.4.

**Ce que fait l'application :**
- garder le jeton de renouvellement dans le **stockage sécurisé** du téléphone (Keystore sous Android, Keychain sous iOS) ; dans l'application web, les jetons restent **en mémoire seulement**, sans aucun stockage dans le navigateur (localStorage, sessionStorage et cookies exclus) : un rechargement de la page demande une nouvelle connexion ;
- sur un 401 `jeton_invalide` d'une route ordinaire : renouveler une fois, puis rejouer la demande ; si le renouvellement répond 401, renvoyer à l'écran de connexion ;
- toujours remplacer **les deux** jetons par ceux du renouvellement ;
- ne jamais lancer deux renouvellements en même temps avec le même jeton.

**Réponse perdue pendant un renouvellement (règle des 30 secondes).** Sur un réseau instable, la réponse d'un renouvellement peut se perdre alors que le serveur a déjà changé le jeton. Le serveur accepte alors **une seule fois** l'ancien jeton de renouvellement, **dans les 30 secondes** qui suivent son premier usage, et seulement si le nouveau jeton n'a jamais servi. Il renvoie une nouvelle paire.
- **L'application doit relancer sa demande de renouvellement en moins de 30 secondes**, avec le même jeton, si elle n'a pas reçu de réponse. Son délai d'attente réseau doit être réglé en conséquence.
- Au-delà de 30 secondes, ou après une seconde réutilisation, le serveur considère que le jeton a été volé : **la session est fermée** (401), et il faut se reconnecter.

**Déconnexions automatiques** : la réinitialisation du mot de passe ferme toutes les sessions du compte ; une session fermée parce qu'un vol a été détecté aussi.

---

## 2. Routes codées

Résumé :

| Méthode | Chemin | Appelant | État |
|---|---|---|---|
| GET | `/health` | tout le monde | fonctionne |
| POST | `/api/patients/otp` | tout le monde | fonctionne |
| POST | `/api/patients/inscription` | tout le monde | fonctionne |
| POST | `/api/patients/connexion` | tout le monde | fonctionne |
| POST | `/api/patients/mot-de-passe/reinitialiser` | tout le monde | fonctionne |
| GET | `/api/patients/moi` | jeton `laaficare:patient` | fonctionne |
| POST | `/api/sessions/renouveler` | tout le monde, avec un jeton de renouvellement | fonctionne (section 1.7) |
| POST | `/api/sessions/deconnexion` | tout le monde, avec un jeton de renouvellement | fonctionne (section 1.7) |
| POST | `/api/sessions/deconnexion-partout` | tout jeton d'accès (patient, professionnel, administrateur) | fonctionne (section 1.7) |
| POST | `/api/professionnels/connexion` | tout le monde | fonctionne (section 2.4) ; un compte professionnel naît à la création d'une structure (section 2.8) |
| POST | `/api/professionnels/connexion/second-facteur` | jeton intermédiaire `laaficare:second_facteur_professionnel` | fonctionne (section 2.4) ; même remarque |
| GET | `/api/professionnels/affectations` | jeton professionnel, sans affectation ou en exercice | fonctionne (section 2.6) ; même remarque |
| POST | `/api/professionnels/affectations/{id}/activer` | jeton professionnel, sans affectation ou en exercice | fonctionne (section 2.6) ; même remarque |
| POST | `/api/professionnels/email/verifier` | tout le monde, avec le jeton du lien | fonctionne (section 2.7) |
| POST | `/api/professionnels/email/renvoyer` | tout le monde, avec email et mot de passe professionnels | fonctionne (section 2.7) |
| POST | `/api/structures` | jeton `laaficare:patient` | fonctionne (section 2.8) ; type `munaseb` seulement |
| POST | `/api/structures/{id}/soumettre` | jeton `laaficare:patient` du créateur | fonctionne (section 2.8) |
| POST | `/api/structures/{id}/abandonner` | jeton `laaficare:patient` du créateur | fonctionne (section 2.8) |
| POST | `/api/structures/{id}/pieces/{type_piece}` | jeton `laaficare:patient` du créateur | fonctionne (section 2.8) |
| GET | `/api/structures/{id}/pieces` | jeton `laaficare:patient` du créateur | fonctionne (section 2.8) |
| GET | `/api/administrateurs/pieces/{id}/fichier` | jeton `laaficare:administrateur` | fonctionne (section 2.9) |
| GET | `/api/structures/mes-demandes` | jeton `laaficare:patient` | fonctionne (section 2.8) |
| GET | `/api/administrateurs/structures` | jeton `laaficare:administrateur` | fonctionne (section 2.9) |
| GET | `/api/administrateurs/structures/{id}` | jeton `laaficare:administrateur` | fonctionne (section 2.9) |
| POST | `/api/administrateurs/structures/{id}/valider` | jeton `laaficare:administrateur` | fonctionne (section 2.9) |
| POST | `/api/administrateurs/structures/{id}/refuser` | jeton `laaficare:administrateur` | fonctionne (section 2.9) |
| POST, GET | `/api/professionnels/invitations` | jeton en exercice, responsable | fonctionne (section 2.10) |
| POST | `/api/professionnels/invitations/{id}/annuler` | jeton en exercice, responsable | fonctionne (section 2.10) |
| GET | `/api/professionnels/membres` | jeton en exercice, responsable | fonctionne (section 2.12) |
| POST | `/api/professionnels/membres/{id}/desactiver` | jeton en exercice, responsable | fonctionne (section 2.12) |
| GET | `/api/patients/invitations` | jeton `laaficare:patient` | fonctionne (section 2.10) |
| GET | `/api/patients/munaseb/droits` | jeton `laaficare:patient` | fonctionne (section 2.13) |
| GET | `/api/patients/munaseb/demandes` | jeton `laaficare:patient` | fonctionne (section 2.13) |
| GET | `/api/patients/munaseb/demandes/{id}` | jeton `laaficare:patient` | fonctionne (section 2.13) |
| GET | `/api/patients/notifications` | jeton `laaficare:patient` | fonctionne (section 2.13) |
| POST | `/api/patients/notifications/{id}/lue` | jeton `laaficare:patient` | fonctionne (section 2.13) |
| POST | `/api/patients/notifications/lues` | jeton `laaficare:patient` | fonctionne (section 2.13) |
| POST | `/api/patients/invitations/{id}/accepter` | jeton `laaficare:patient` | fonctionne (section 2.10) |
| POST | `/api/patients/invitations/{id}/refuser` | jeton `laaficare:patient` | fonctionne (section 2.10) |
| POST | `/api/professionnels/mot-de-passe/oubli` | tout le monde | fonctionne (section 2.11) |
| POST | `/api/administrateurs/mot-de-passe/oubli` | tout le monde | fonctionne (section 2.11) |
| POST | `/api/mot-de-passe/reinitialiser` | tout le monde, avec le jeton du lien | fonctionne (section 2.11) |
| POST | `/api/administrateurs/connexion` | tout le monde | fonctionne (section 2.5) ; testable après l'outil du premier administrateur |
| POST | `/api/administrateurs/connexion/second-facteur` | jeton intermédiaire `laaficare:second_facteur_administrateur` | fonctionne (section 2.5) |
| POST | `/api/administrateurs/reenrolement/commencer` | jeton `laaficare:reenrolement_administrateur` | fonctionne (section 2.5) |
| POST | `/api/administrateurs/reenrolement/confirmer` | jeton `laaficare:reenrolement_administrateur` | fonctionne (section 2.5) |
| POST | `/api/assurance-munaseb/remboursements/simuler-acte` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| GET | `/api/assurance-munaseb/remboursements` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| GET | `/api/assurance-munaseb/remboursements/{id}` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| POST | `/api/assurance-munaseb/remboursements/{id}/prendre-en-charge` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| POST | `/api/assurance-munaseb/remboursements/{id}/valider` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| POST | `/api/assurance-munaseb/remboursements/{id}/rejeter` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| POST | `/api/assurance-munaseb/remboursements/{id}/payer` | jeton en exercice, agent MUNASEB | fonctionne (section 2.3) |
| GET | `/api/assurance-munaseb/parametres` | jeton en exercice, agent MUNASEB ou responsable de la MUNASEB | fonctionne (section 2.14) |
| PUT | `/api/assurance-munaseb/parametres` | jeton en exercice, **responsable** de la MUNASEB seulement | fonctionne (section 2.14) |
| GET | `/api/assurance-munaseb/parametres/historique` | jeton en exercice, agent MUNASEB ou responsable de la MUNASEB | fonctionne (section 2.14) |
| POST, GET | `/api/assurance-munaseb/contrats` | jeton en exercice, agent MUNASEB | fonctionne (section 2.14) ; GET avec `?numero_carte=` |
| GET | `/api/assurance-munaseb/contrats/{id}` | jeton en exercice, agent MUNASEB | fonctionne (section 2.14) |
| POST | `/api/assurance-munaseb/contrats/{id}/renouvellements` | jeton en exercice, agent MUNASEB | fonctionne (section 2.14) |
| GET | `/api/pharmacie/medicaments`, `/api/pharmacie/medicaments/{id}/equivalents` | jeton en exercice, pharmacien ou responsable d'une pharmacie | fonctionne (section 2.15) |
| GET | `/api/pharmacie/stock`, `/api/pharmacie/stock/{medicament_id}`, `/api/pharmacie/mouvements`, `/api/pharmacie/ventes/{id}`, `/api/pharmacie/alertes` | même appelant | fonctionne (section 2.15) |
| POST | `/api/pharmacie/entrees`, `/api/pharmacie/ventes`, `/api/pharmacie/ventes/{id}/annuler`, `/api/pharmacie/pertes`, `/api/pharmacie/inventaires`, `/api/pharmacie/mouvements/{id}/annuler` | même appelant ; en-tête `Idempotency-Key` facultatif | fonctionne (section 2.15) |
| PUT | `/api/pharmacie/seuils/{medicament_id}` | même appelant | fonctionne (section 2.15) |
| PUT | `/api/pharmacie/prix/{medicament_id}`, `/api/pharmacie/reglages` | jeton en exercice, **responsable** d'une pharmacie seulement | fonctionne (section 2.15) |
| POST, GET | `/api/pharmacie/depenses` | jeton en exercice, **responsable** d'une pharmacie seulement ; POST avec `Idempotency-Key` facultatif | fonctionne (section 2.16) |
| POST | `/api/pharmacie/depenses/{id}/annuler` | même appelant | fonctionne (section 2.16) |
| GET | `/api/pharmacie/rapport` | même appelant | fonctionne (section 2.16) |
| PUT | `/api/patients/moi/sexe` | jeton `laaficare:patient` | fonctionne (section 2.2.7) |
| GET, PUT | `/api/professionnels/profil` | jeton professionnel, avec ou sans affectation | fonctionne (section 2.17) |
| GET | `/api/administrateurs/profils-professionnels` | jeton administrateur | fonctionne (section 2.17) |
| POST | `/api/administrateurs/profils-professionnels/{compte_id}/verifier` | jeton administrateur | fonctionne (section 2.17) |
| POST | `/api/professionnels/patients/scan` | jeton en exercice : accueil, médecin, infirmier, agent MUNASEB, pharmacien, auxiliaire | fonctionne (section 2.18) |
| GET | `/api/patients/journal-acces` | jeton `laaficare:patient` | fonctionne (section 2.18) |
| POST | `/api/patients/code-soin` | jeton `laaficare:patient` | fonctionne (section 2.19) |
| POST | `/api/professionnels/patients/recherche` | jeton en exercice : accueil, médecin, infirmier d'une clinique | fonctionne (section 2.19) |
| POST | `/api/prises-en-charge` | jeton en exercice : accueil, médecin, infirmier d'une clinique ; pharmacien | fonctionne (section 2.19) |
| POST | `/api/prises-en-charge/bris-de-glace`, `/api/prises-en-charge/{id}/prolonger` | jeton en exercice : médecin ou infirmier d'une clinique | fonctionne (section 2.19) |
| GET | `/api/structures/bris-de-glace` | jeton en exercice : responsable d'une clinique | fonctionne (section 2.19) |
| POST | `/api/structures/bris-de-glace/{id}/controler` | même appelant | fonctionne (section 2.19) |
| GET | `/api/prises-en-charge/{id}/dossier` | jeton en exercice : médecin, infirmier (dossier complet) ; pharmacien (vue pharmacie) | fonctionne (section 2.20) |
| POST | `/api/prises-en-charge/{id}/allergies`, `/antecedents` | médecin ou infirmier | fonctionne (section 2.20) |
| POST | `/api/prises-en-charge/{id}/allergies/{allergie_id}/retirer`, `/consultations`, `/consultations/{consultation_id}/ajouts`, `/ordonnances` | médecin | fonctionne (section 2.20) |

> **Routes MUNASEB rebranchées (étape 3.9).** Un agent se connecte par la connexion professionnelle (section 2.4), puis active son affectation `agent_assurance_munaseb` (section 2.6). Les routes de remboursement n'acceptent qu'un jeton `laaficare:professionnel_en_exercice` : tout autre jeton est refusé en 401. Avec un jeton de la bonne sorte, elles répondent **403 `acces_refuse`** si l'affectation n'est pas celle d'un agent MUNASEB, est désactivée, ou si la MUNASEB n'est plus validée ou sous licence. **Testable dans Postman de bout en bout** : la structure MUNASEB se crée (section 2.8), l'équipe la valide (section 2.9), le responsable invite un agent (section 2.10), et l'agent enregistre les contrats (section 2.14).

### 2.1 Infrastructure

#### `GET /health`

État du serveur et de la base. Pour la supervision, pas pour les applications.

- **Appelant** : tout le monde, sans jeton.
- **Corps** : aucun.
- **Réponses** :

| Code | Corps |
|---|---|
| 200 | `{"status": "ok", "database": "up"}` |
| 503 | `{"status": "error", "database": "down"}` |

Exception au format d'erreur de 1.3 : cette route ne renvoie jamais `erreur`/`code`.

### 2.2 Patient : inscription, connexion, mot de passe oublié

Parcours prévu côté application :

- **Inscription** : le formulaire reste dans l'application → `POST /api/patients/otp` → la personne saisit le code reçu par SMS → `POST /api/patients/inscription` (un seul appel, avec le code et le mot de passe) → jeton reçu, session ouverte.
- **Connexion** : `POST /api/patients/connexion`.
- **Mot de passe oublié** (couvre aussi le compte verrouillé) : `POST /api/patients/otp` → `POST /api/patients/mot-de-passe/reinitialiser` → retour à l'écran de connexion.

Aucune de ces routes ne dit si un compte existe pour un numéro, sauf la réinitialisation après un code OTP correct (voir 2.2.4).

#### 2.2.1 `POST /api/patients/otp`

Envoie un code à 6 chiffres par SMS. Sert à l'inscription et au mot de passe oublié.

- **Appelant** : tout le monde, sans jeton.
- **Corps** :

```json
{ "telephone": "70 00 00 00" }
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | code envoyé (aucun corps) |
| 422 | `telephone_invalide` | format non reconnu (voir 1.6) |
| 429 | `trop_de_demandes` | moins de 60 s depuis le dernier code, ou déjà 5 codes dans l'heure ; en-tête `Retry-After` (voir 2.2.5) |
| 500 | `erreur_interne` | panne |

- Le code est valable **5 minutes**. Une nouvelle demande remplace le code précédent.
- La réponse est la même que le numéro ait un compte ou non, limites comprises.
- **Compte patient bloqué par l'équipe LaafiCare** (outil `intervenir_compte`) : aucun code n'est créé et **aucun SMS ne part**, mais la réponse est exactement la même (204, limites comprises) : rien ne révèle le blocage.
- En développement, aucun SMS ne part : le code s'affiche dans les journaux du serveur.

#### 2.2.2 `POST /api/patients/inscription`

Crée le compte patient, attribue le NIP et ouvre la session.

- **Appelant** : tout le monde, sans jeton, avec un code OTP valide.
- **Corps** :

```json
{
  "telephone": "70 00 00 00",
  "code_otp": "123456",
  "nom": "OUEDRAOGO",
  "prenom": "Awa",
  "date_naissance": "1995-04-12",
  "lieu_naissance": "Koudougou",
  "mot_de_passe": "Exemple#2026"
}
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 201 | — | compte créé et session ouverte ; corps `{"etape": "complete", "jeton_acces", "expire_dans", "jeton_renouvellement"}` (section 1.7), jeton d'accès `laaficare:patient` |
| 422 | `telephone_invalide` | format du numéro non reconnu |
| 422 | `mot_de_passe_non_conforme` | + `regles_non_respectees` (voir 1.5) |
| 401 | `code_invalide` | code OTP faux, expiré ou déjà utilisé |
| 429 | `trop_de_demandes` | code faux, et c'était le dernier essai permis : le code est effacé, pas d'en-tête `Retry-After` (voir 2.2.5) |
| 409 | `telephone_deja_utilise` | un compte existe déjà pour ce numéro |
| 502, 503 ou 504 | — | réponse d'un intermédiaire réseau (passerelle, proxy, répartiteur), pas du serveur LaafiCare : la demande a **peut-être** été traitée et le compte créé. Ne pas renvoyer l'inscription : voir « Si la réponse se perd ». |
| 500 | `erreur_interne` | panne |

- Ordre des contrôles : numéro, puis mot de passe, puis code OTP. Un mot de passe refusé **ne consomme pas** le code : la personne corrige et renvoie la même requête.
- Le 409 n'arrive qu'après un code OTP correct : seul le titulaire du numéro l'apprend.

**Si la réponse se perd** (coupure réseau, délai dépassé, ou réponse **502, 503 ou 504** d'un intermédiaire réseau) : le compte a peut-être été créé quand même, et le code OTP est peut-être déjà consommé. L'application ne renvoie pas la même inscription. Elle fait une **connexion de contrôle** (`POST /api/patients/connexion`, section 2.2.3) avec le même téléphone et le même mot de passe :

| Réponse de la connexion de contrôle | Ce que fait l'application |
|---|---|
| 200 | Le compte existe : la session est ouverte avec cette paire de jetons, l'inscription est terminée. |
| 401 `identifiants_invalides` | Le compte n'a pas été créé. L'application relance l'inscription : nouvelle demande de code (`POST /api/patients/otp`, qui peut répondre 429 : attendre le délai de `Retry-After`), puis nouvel envoi du formulaire, qu'elle a gardé. |
| 502, 503, 504 ou erreur réseau | L'application réessaie la connexion de contrôle, **au plus 3 fois**, espacées (par exemple après 2 s, 5 s, puis 15 s) ; elle ne renvoie pas l'inscription. Après le troisième échec, elle demande au patient de réessayer quand le réseau sera meilleur. **Le formulaire reste gardé.** |

**Si l'inscription relancée répond 409 `telephone_deja_utilise`**, le compte a été créé entre-temps : la première inscription avait abouti. L'application refait une connexion de contrôle, **sans afficher d'erreur au patient**.

**Sortie de boucle.** Si cette connexion de contrôle répond **encore** 401 `identifiants_invalides`, le numéro appartient à un autre compte (ou le mot de passe de ce compte est différent). L'application **arrête le parcours** et affiche le message normal « ce numéro a déjà un compte », avec accès à la connexion et à « Mot de passe oublié ? ».

**Session inutilisée.** Une inscription dont la réponse s'est perdue a ouvert une session (section 1.7) que l'application n'a jamais reçue. Cette session n'est jamais utilisée et expire d'elle-même, au bout de 30 jours sans renouvellement, sans aucune action du patient ni du serveur.

Aucun changement côté serveur : ce parcours n'utilise que les routes existantes.

#### 2.2.3 `POST /api/patients/connexion`

- **Appelant** : tout le monde, sans jeton.
- **Corps** :

```json
{ "telephone": "70 00 00 00", "mot_de_passe": "Exemple#2026" }
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | session ouverte ; corps `{"etape": "complete", "jeton_acces", "expire_dans", "jeton_renouvellement"}` (section 1.7), jeton d'accès `laaficare:patient` |
| 401 | `identifiants_invalides` | tous les refus (voir ci-dessous) |
| 422 | `telephone_invalide` | format du numéro non reconnu |
| 500 | `erreur_interne` | panne |

- `identifiants_invalides`, message « Numéro ou mot de passe incorrect. », couvre **tous** les refus : numéro inconnu, inscription inachevée, mauvais mot de passe, compte verrouillé, compte désactivé, et compte dont le second facteur est actif (la connexion en deux temps n'existe pas encore, voir section 4). Le temps de réponse est le même dans tous les cas.
- **Verrouillage** : après 5 échecs, le compte est verrouillé 15 minutes. La réponse ne le dit pas. L'application propose les boutons **« Mot de passe oublié ? »** (qui lève aussi le verrouillage) et **« Créer un compte »**.

#### 2.2.4 `POST /api/patients/mot-de-passe/reinitialiser`

- **Appelant** : tout le monde, sans jeton, avec un code OTP valide.
- **Corps** :

```json
{ "telephone": "70 00 00 00", "code_otp": "123456", "nouveau_mot_de_passe": "Nouveau#2026" }
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | mot de passe changé (aucun corps) |
| 422 | `telephone_invalide` | format du numéro non reconnu |
| 422 | `mot_de_passe_non_conforme` | + `regles_non_respectees` |
| 401 | `code_invalide` | code OTP faux, expiré ou déjà utilisé |
| 429 | `trop_de_demandes` | code faux, et c'était le dernier essai permis : le code est effacé, pas d'en-tête `Retry-After` (voir 2.2.5) |
| 404 | `telephone_inconnu` | aucun compte pour ce numéro (seulement après un code correct) |
| 500 | `erreur_interne` | panne |

- Effets : le verrouillage est levé, le compteur d'échecs remis à zéro.
- Aucun jeton n'est renvoyé : la personne se connecte ensuite avec son nouveau mot de passe.
- **Toutes les sessions du compte sont fermées**, sur tous les appareils : les jetons d'accès et de renouvellement émis avant la réinitialisation sont refusés.
- La réinitialisation **ne touche jamais au second facteur** (TOTP).

#### 2.2.5 Limites du code OTP et réponse 429

**Les limites**, identiques pour tout numéro, inscrit ou non (la réponse ne révèle donc rien d'un compte) :

| Limite | Valeur | Réponse quand elle est atteinte |
|---|---|---|
| Délai entre deux demandes de code pour un même numéro | 60 s | 429 sur `POST /api/patients/otp`, avec `Retry-After` |
| Demandes de code par numéro | 5 par heure glissante | 429 sur `POST /api/patients/otp`, avec `Retry-After` (temps avant que la plus ancienne des 5 ait plus d'une heure) |
| Essais sur un code | 5 ; puis, tant qu'aucun code juste n'a été saisi, **1 seul essai** sur chaque nouveau code | 429 sur l'inscription ou la réinitialisation, **sans** `Retry-After` : le code est effacé |

Les échecs s'additionnent d'un code à l'autre et ne reviennent à zéro qu'après un code juste. Le numéro est compté sous sa forme normalisée : « 70 00 00 00 » et « +226 70000000 » comptent ensemble.

**Ce que l'application doit afficher sur un 429 `trop_de_demandes` :**

| Où | En-tête `Retry-After` | Affichage |
|---|---|---|
| Demande de code (`POST /api/patients/otp`) | présent : nombre de secondes | Le délai à attendre, par exemple « Vous pourrez demander un nouveau code dans 45 s », avec un compte à rebours. Le bouton « Renvoyer le code » reste désactivé jusqu'à la fin du délai. Au-delà d'une minute, afficher en minutes. |
| Inscription ou réinitialisation | absent | « Trop d'essais avec ce code. Demandez un nouveau code. » Le champ du code est vidé et l'application propose le bouton « Renvoyer le code ». |

L'application lit la valeur de l'en-tête `Retry-After`, jamais le message `erreur`. Après un 401 `code_invalide`, elle laisse simplement la personne ressaisir le code, ou en redemander un s'il a expiré.

#### 2.2.6 `GET /api/patients/moi`

Profil du patient connecté.

- **Appelant** : jeton `laaficare:patient` (en-tête `Authorization: Bearer <jeton>`).
- **Corps** : aucun.
- **Réponse 200** :

```json
{
  "compte_id": "0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55",
  "nip": "1000000000000008",
  "nom": "OUEDRAOGO",
  "prenom": "Awa",
  "telephone": "+22670000000",
  "date_naissance": "1995-04-12",
  "lieu_naissance": "Koudougou",
  "sexe": null
}
```

| Champ | Contenu |
|---|---|
| `compte_id` | identifiant du compte patient ; **ne change jamais**. L'application s'en sert pour rattacher ce qu'elle garde sur le téléphone (rappels…). |
| `nip` | NIP à 16 chiffres |
| `nom`, `prenom` | toujours remplis |
| `telephone` | forme normalisée (`+226…`) |
| `date_naissance` | `AAAA-MM-JJ` |
| `lieu_naissance` | texte saisi à l'inscription |
| `sexe` | `feminin`, `masculin`, ou `null` tant que la personne ne l'a pas donné (ajouté le 2026-11-17, facultatif) |

- **Erreurs** :

| Code | `code` | Cas |
|---|---|---|
| 401 | `authentification_requise` | en-tête `Authorization` absent |
| 401 | `jeton_invalide` | jeton expiré, d'une autre sorte, **émis avant une réinitialisation du mot de passe**, ou compte désactivé |
| 500 | `erreur_interne` | panne, ou incohérence en base |

- **Révocation** : après une réinitialisation du mot de passe, ou quand la session du jeton a été fermée (déconnexion, déconnexion partout, vol détecté), le jeton est refusé (401 `jeton_invalide`) même s'il n'a pas encore expiré. L'application tente un renouvellement (section 1.7) ; s'il répond 401, elle renvoie la personne à l'écran de connexion.
- **Compte verrouillé** (trop d'essais de mot de passe) : un jeton déjà reçu reste valable ; le verrouillage ne bloque que les nouvelles connexions.

#### 2.2.7 `PUT /api/patients/moi/sexe` (2026-11-17)

Le patient donne, change ou retire son sexe, **facultatif**.

- **Appelant** : jeton `laaficare:patient`.
- **Corps** : `{"sexe": "feminin"}`, `{"sexe": "masculin"}` ou `{"sexe": null}` (retire la valeur ; un champ absent vaut `null`).
- **Réponse** : **204**.
- **Erreurs** : 422 `donnees_invalides` pour une autre valeur ; 401 comme `GET /api/patients/moi`.

### 2.3 Agent MUNASEB

#### 2.3.1 Connexion de l'agent

L'ancienne route `POST /api/assurance-munaseb/connexion` est **supprimée** (2026-10-08), avec son code `423 compte_verrouille`. L'agent passe par la connexion professionnelle (section 2.4), puis active son affectation (section 2.6). Chaque transition d'une demande est enregistrée avec cette affectation : qui l'a faite, et dans quelle structure.

#### 2.3.2 Valeurs utilisées par les routes de remboursement

`type_acte` :

| Valeur | |
|---|---|
| `consultation` | |
| `hospitalisation` | |
| `pharmacie` | |
| `laboratoire` | |
| `lunetterie` | |

`statut` d'une demande, et transitions permises :

| Valeur | Sens | Transitions possibles |
|---|---|---|
| `en_attente` | créée, pas encore prise en charge | → `en_cours`, → `rejete` |
| `en_cours` | prise en charge par un agent | → `valide`, → `rejete` |
| `valide` | montant remboursé fixé | → `paye` |
| `rejete` | refusée, avec motif | aucune |
| `paye` | dossier clos | aucune |

Toute autre transition → 409 `transition_invalide` avec `statut_actuel`.

#### 2.3.3 `POST /api/assurance-munaseb/remboursements/simuler-acte`

Route de test : crée une demande comme le feront plus tard les modules Pharmacie, Laboratoire et Hôpital (tiers payant).

- **Appelant** : jeton agent MUNASEB.
- **Corps** :

```json
{
  "nip": "1000000000000008",
  "partenaire_id": "0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55",
  "type_acte": "pharmacie",
  "reference_acte": "FACT-0001",
  "date_acte": "2026-10-08",
  "montant_acte_fcfa": 12500
}
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | corps `{"demande_id": "<UUID>"}`, demande en `en_attente` |
| 422 | `montant_invalide` | montant nul ou négatif |
| 422 | `nip_invalide` | NIP mal formé (16 chiffres, clé de Luhn) |
| 404 | `patient_inconnu` | aucun patient pour ce NIP |
| 404 | `partenaire_inconnu` | partenaire inexistant |
| 422 | `partenaire_suspendu` | partenaire suspendu (le patient reste assuré) |
| 409 | `reference_acte_deja_utilisee` | même référence déjà utilisée par ce partenaire pour un autre acte |
| 422 | `non_couvert` | pas de contrat actif couvrant la date de l'acte |
| 422 | `en_carence` | + `debut_couverture` : contrat payé, couverture pas encore commencée |
| 500 | `erreur_interne` | panne |

- Renvoyer exactement le même acte (même référence, mêmes données) renvoie la demande déjà créée, sans doublon.
- Le montant demandé est calculé avec le tarif du type d'acte, puis figé.

#### 2.3.4 `GET /api/assurance-munaseb/remboursements`

Liste des demandes, la plus ancienne d'abord, par pages.

- **Appelant** : jeton agent MUNASEB.
- **Paramètres de requête** (tous facultatifs) :

| Paramètre | Contenu |
|---|---|
| `statut` | une valeur de 2.3.2 |
| `limite` | 1 à 100, 50 par défaut |
| `apres` | valeur `suivant` de la page précédente |

Exemple : `GET /api/assurance-munaseb/remboursements?statut=en_attente&limite=20`

- **Réponse 200** :

```json
{
  "demandes": [
    {
      "id": "0199b8e2-7c41-7a10-9f3e-2b6d1c0e4a55",
      "statut": "en_attente",
      "type_acte": "pharmacie",
      "date_acte": "2026-10-08",
      "montant_acte_fcfa": 12500,
      "montant_demande_fcfa": 10000,
      "montant_rembourse_fcfa": null,
      "date_demande": "2026-10-08T09:30:00Z",
      "partenaire": "Pharmacie exemple",
      "patient_nom": "OUEDRAOGO",
      "patient_prenom": "Awa",
      "numero_carte": "MUN-0001"
    }
  ],
  "suivant": null
}
```

- `suivant` vaut `null` quand la page n'est pas pleine ; sinon, le passer dans `apres` pour la page suivante.
- `montant_rembourse_fcfa` vaut `null` tant que la demande n'est pas validée. `patient_nom` et `patient_prenom` peuvent être `null`.
- `numero_carte` (précisé le 2026-11-16, étape 5) : la carte de la **période de la demande**, puisque le numéro change chaque année ; pour une période enregistrée avant l'étape 5, la carte d'origine du contrat, comme avant.
- L'agent ne voit **jamais** le NIP ni le téléphone du patient dans les demandes (le téléphone n'est que dans le détail d'un contrat, section 2.14).
- Une demande créée pendant le parcours des pages peut n'apparaître qu'au rechargement.
- **Erreurs** : 400 `parametre_invalide` (limite hors bornes, statut inconnu, curseur qui n'est pas un UUID) ; 500 `erreur_interne`.

#### 2.3.5 `GET /api/assurance-munaseb/remboursements/{id}`

Détail d'une demande avec son historique.

- **Appelant** : jeton agent MUNASEB.
- **Réponse 200** : les champs de la liste, plus :

```json
{
  "motif_rejet": null,
  "periode_debut": "2026-02-01",
  "periode_fin": "2027-01-31",
  "solde_restant_periode_fcfa": 87500,
  "historique": [
    {
      "statut_precedent": null,
      "statut_nouveau": "en_attente",
      "date_changement": "2026-10-08T09:30:00Z",
      "agent_nom": null,
      "agent_prenom": null
    }
  ]
}
```

- `solde_restant_periode_fcfa` est indicatif : il est recalculé au moment de la validation.
- Dans l'historique, `agent_nom` et `agent_prenom` sont `null` pour la création (faite par le partenaire).
- **Erreurs** : 400 `chemin_invalide` ; 404 `demande_inconnue` ; 500 `erreur_interne`.

#### 2.3.6 Transitions

Toutes : jeton agent MUNASEB ; l'agent qui agit est enregistré dans l'historique ; chaque changement crée une notification pour le patient.

| Méthode et chemin | Corps | Succès |
|---|---|---|
| `POST …/remboursements/{id}/prendre-en-charge` | aucun | 204 |
| `POST …/remboursements/{id}/valider` | aucun | 200 `{"montant_rembourse_fcfa": 10000}` |
| `POST …/remboursements/{id}/rejeter` | `{"motif": "Pièce manquante"}` | 204 |
| `POST …/remboursements/{id}/payer` | aucun | 204 |

- **Valider** : le montant remboursé est le montant demandé, plafonné au solde de la période. Il peut valoir **0** si le plafond est atteint ; la demande n'est jamais rejetée pour ce seul motif, et doit quand même passer à `paye` pour être close.
- **Rejeter** : motif obligatoire, 1 000 caractères au plus. Il n'apparaît jamais dans la notification du patient.
- **Erreurs** :

| Code | `code` | Cas |
|---|---|---|
| 400 | `chemin_invalide` | identifiant qui n'est pas un UUID |
| 404 | `demande_inconnue` | demande inexistante |
| 409 | `transition_invalide` | + `statut_actuel` : transition non permise depuis ce statut |
| 422 | `motif_vide` | rejet sans motif (ou motif fait d'espaces) |
| 422 | `motif_trop_long` | motif de plus de 1 000 caractères |
| 500 | `erreur_interne` | panne |

### 2.4 Professionnels : connexion

#### `POST /api/professionnels/connexion`

- **Appelant** : tout le monde, sans jeton.
- **Corps** :

```json
{ "email": "pro.exemple@example.org", "mot_de_passe": "Exemple#2026" }
```

L'email est comparé sans les espaces autour et sans tenir compte des majuscules.

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | `{"etape": "complete", "jeton_acces", "expire_dans", "jeton_renouvellement"}` (section 1.7) : session ouverte de 12 h au plus, 1 h sans renouvellement ; jeton d'accès `laaficare:professionnel_sans_affectation`, qui servira à lister ses établissements et à en choisir un (étape 3.9) |
| 200 | — | `{"etape": "second_facteur", "jeton_intermediaire": "<JWT>"}` : second facteur actif ; jeton `laaficare:second_facteur_professionnel` de 5 min, à échanger contre la paire avec le code (route suivante). Aucune session n'est encore ouverte. |
| 401 | `identifiants_invalides` | tous les refus : email inconnu, mauvais mot de passe, compte verrouillé ou désactivé ; message « Email ou mot de passe incorrect. », même temps de réponse |
| 403 | `email_non_verifie` | mot de passe correct, mais email jamais vérifié (ou plus vérifié après un changement) ; message « Votre email n'est pas encore vérifié. Ouvrez le lien que nous vous avons envoyé, ou demandez-en un nouveau. » |
| 500 | `erreur_interne` | panne |

- **L'application teste toujours `etape`**, même si aucun professionnel ne peut encore activer de second facteur (après l'étape 3.11).
- Le **403** n'arrive qu'après un mot de passe correct : seul le détenteur du mot de passe l'apprend. Sur ce 403, l'application propose « Renvoyer le lien » (section 2.7).
- **Verrouillage** : après 5 échecs, 15 minutes ; la réponse reste 401 `identifiants_invalides`. La récupération du mot de passe se fera par email (étape 3.12).
- Un compte professionnel naît quand un patient crée sa structure (section 2.8).

#### `POST /api/professionnels/connexion/second-facteur`

Second temps de la connexion, quand la première étape a répondu `"etape": "second_facteur"`.

- **Appelant** : en-tête `Authorization: Bearer <jeton_intermediaire>` (le jeton reçu à la première étape, valable 5 minutes).
- **Corps** : **un seul** des deux champs :

```json
{ "code": "123456" }
```

ou

```json
{ "code_secours": "ABCDE-FGHIJ" }
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | `{"etape": "complete", "jeton_acces", "expire_dans", "jeton_renouvellement"}` (section 1.7) : session ouverte, jeton d'accès `laaficare:professionnel_sans_affectation` |
| 401 | `code_invalide` | code faux, code de secours faux ou déjà utilisé, ou compte verrouillé ; message « Code incorrect. », une seule réponse dans tous ces cas |
| 401 | `jeton_invalide` | jeton intermédiaire expiré, d'une autre sorte, **déjà utilisé**, émis avant une réinitialisation du mot de passe, compte désactivé, ou email plus vérifié |
| 422 | `donnees_invalides` | les deux champs, ou aucun |
| 500 | `erreur_interne` | panne |

- **Le jeton intermédiaire ne sert qu'une fois** : après un succès, il est refusé, même avec un autre code valable. Un **code faux ne le consomme pas** : la personne peut retaper son code tant que le jeton est valable (5 minutes) et que le compte n'est pas verrouillé.
- **Verrouillage** : chaque code ou code de secours faux compte dans le **même compteur que le mot de passe** (5 essais, puis 15 minutes). Obtenir le jeton intermédiaire compte aussi : le mot de passe correct ne remet pas le compteur à zéro tant que le code n'est pas donné.
- Sur 401 `code_invalide` : l'application vide le champ du code et laisse la personne réessayer. Sur 401 `jeton_invalide` : elle revient à l'écran de connexion.
- **Limite connue** : une nouvelle connexion du même compte dans **la même seconde** qu'un second facteur réussi donne un jeton intermédiaire refusé à tort (401 `jeton_invalide`) ; la personne se reconnecte une seconde plus tard.

### 2.5 Administrateurs LaafiCare : connexion et réenrôlement

Le second facteur (TOTP) est **obligatoire** pour un administrateur : la connexion ne donne jamais de session sans code. Un administrateur sans TOTP actif (premier administrateur, ou téléphone perdu) ne peut en enregistrer un nouveau que pendant une **fenêtre de 10 minutes** ouverte par l'équipe LaafiCare avec un outil local, jamais par une route. Hors de cette fenêtre, son mot de passe seul ne donne rien.

**Testable dans Postman** après l'outil local `creer_premier_administrateur` : il crée le compte, fait vérifier l'email, puis ouvre la fenêtre de 10 minutes (README, « Créer le premier administrateur »). Les comptes administrateur ne se créent jamais par une route : les outils `creer_administrateur` et `desactiver_totp` (qui rouvre une fenêtre après la perte d'un téléphone) sont lancés sur l'ordinateur du serveur, par un administrateur authentifié.

#### `POST /api/administrateurs/connexion`

- **Appelant** : tout le monde. **Corps** : `{"email": "…", "mot_de_passe": "…"}` (même forme que la connexion professionnelle).
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | TOTP actif : `{"etape": "second_facteur", "jeton_intermediaire": "…"}` (5 min) |
| 200 | — | pas de TOTP actif, fenêtre ouverte : `{"etape": "reenrolement", "jeton_reenrolement": "…"}` (10 min) |
| 401 | `identifiants_invalides` | email inconnu, mauvais mot de passe, compte verrouillé ou désactivé, **ou pas de TOTP et pas de fenêtre** ; message « Email ou mot de passe incorrect. » dans tous les cas |
| 403 | `email_non_verifie` | mot de passe correct, email pas vérifié |
| 422 | `donnees_invalides` | champ manquant |
| 500 | `erreur_interne` | panne |

- Jamais d'`etape: "complete"` pour un administrateur.

#### `POST /api/administrateurs/connexion/second-facteur`

Identique à la route professionnelle (section 2.4) : en-tête `Authorization: Bearer <jeton_intermediaire>`, corps `{"code"}` ou `{"code_secours"}`, mêmes réponses, jeton à usage unique. La réponse 200 ouvre une **session administrateur** : jeton d'accès `laaficare:administrateur` de 15 min, session de 24 h au plus, fermée après 1 h d'inactivité.

#### `POST /api/administrateurs/reenrolement/commencer`

- **Appelant** : en-tête `Authorization: Bearer <jeton_reenrolement>`.
- **Corps** : `{"mot_de_passe": "…"}`. Le mot de passe est redemandé, comme pour toute activation d'un second facteur.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | `{"url_otpauth": "otpauth://totp/…", "cle_base32": "…"}` : l'application dessine le QR code à partir de `url_otpauth`, et affiche `cle_base32` pour une saisie à la main |
| 401 | `identifiants_invalides` | mot de passe faux ou compte verrouillé ; message « Mot de passe incorrect. » |
| 401 | `jeton_invalide` | jeton expiré ou d'une autre sorte, fenêtre expirée, TOTP déjà actif, compte désactivé, email plus vérifié |
| 422 | `donnees_invalides` | champ manquant |
| 500 | `erreur_interne` | panne |

- Peut être relancée tant que la fenêtre est ouverte (QR code perdu) : la demande précédente est remplacée.

#### `POST /api/administrateurs/reenrolement/confirmer`

- **Appelant** : en-tête `Authorization: Bearer <jeton_reenrolement>`.
- **Corps** : `{"code": "123456"}`, le premier code affiché par l'application d'authentification.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | `{"codes_secours": ["XXXXX-XXXXX", …]}` : 10 codes, **montrés une seule fois** ; l'application demande de les noter avant de continuer |
| 401 | `code_invalide` | code faux, aucune demande en cours, fenêtre expirée entre-temps, ou compte verrouillé ; message « Code incorrect. » |
| 401 | `jeton_invalide` | comme pour `commencer` |
| 422 | `donnees_invalides` | champ manquant |
| 500 | `erreur_interne` | panne |

- **Aucune session n'est ouverte** : après avoir montré les codes de secours, l'application revient à l'écran de connexion. La personne se connecte avec son mot de passe, puis un code de son nouveau TOTP.
- Après la confirmation, le jeton de réenrôlement est refusé (401 `jeton_invalide`), et la fenêtre est refermée.
- Chaque essai faux (mot de passe ou code) compte dans le verrouillage du compte (5 essais, puis 15 minutes).
- Après une confirmation réussie, **un email d'alerte** part à l'adresse de l'administrateur : date et heure de l'enregistrement (heure du Burkina Faso), et adresse de l'équipe à contacter si ce n'était pas lui. Il ne contient aucun code ni aucun lien. Un échec d'envoi ne change pas la réponse : le second facteur est déjà enregistré.

### 2.6 Professionnels : choisir son affectation

Après la connexion professionnelle, le jeton d'accès (`laaficare:professionnel_sans_affectation`) ne sert qu'à lister ses affectations et à en activer une. L'activation donne un jeton `laaficare:professionnel_en_exercice`, qui porte l'affectation (`aff`) et ouvre les routes métier de cette affectation. **Changer d'établissement ou de rôle** : rappeler l'activation avec le jeton en exercice, sans redonner le mot de passe.

Les affectations naissent de la création d'une structure (responsable, section 2.8) et des invitations acceptées (section 2.10).

#### `GET /api/professionnels/affectations`

- **Appelant** : en-tête `Authorization: Bearer <jeton_acces>`, jeton sans affectation **ou** en exercice.
- **Réponse 200** :

```json
{
  "affectations": [
    {
      "id": "…",
      "role": "agent_assurance_munaseb",
      "structure_id": "…",
      "structure_nom": "MUNASEB",
      "structure_type": "munaseb",
      "structure_statut": "validee",
      "licence_expire_le": "2027-10-18",
      "exercable": true
    }
  ]
}
```

- Seules les affectations **actives** sont listées. `exercable` vaut `false` si l'établissement n'est pas validé (par exemple `suspendue`) ou si sa licence est expirée : l'application montre la ligne, mais grisée, avec la raison (`structure_statut`, `licence_expire_le`). `licence_expire_le` vaut `null` pour une licence sans fin.
- La licence est valable jusqu'à sa date de fin **comprise**, à la date du jour au Burkina Faso.

#### `POST /api/professionnels/affectations/{id}/activer`

- **Appelant** : même en-tête, jeton sans affectation ou en exercice. Pas de corps.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | `{"jeton_acces", "expire_dans", "jeton_renouvellement"}` (forme de la section 1.7, sans `etape`) : nouvelle session, de cette affectation |
| 401 | `jeton_invalide` | jeton expiré, d'une autre sorte, session fermée, compte désactivé, email plus vérifié |
| 403 | `affectation_non_exercable` | affectation désactivée, établissement non validé, ou licence expirée |
| 404 | `affectation_inconnue` | affectation inexistante, ou qui n'est pas à vous (jamais distingués) |
| 500 | `erreur_interne` | panne |

- **La session courante est fermée** et remplacée par une session de l'affectation choisie. Les anciens jetons, d'accès et de renouvellement, ne servent plus : l'application garde la nouvelle paire.
- La nouvelle session **finit à la même heure** que l'ancienne (12 h après la connexion au plus) : changer d'affectation ne prolonge jamais une session.
- Sur un jeton en exercice, une réponse **403 `acces_refuse`** d'une route métier veut dire que le droit a été retiré (affectation désactivée, établissement suspendu, licence expirée) : l'application relit la liste des affectations.

### 2.7 Professionnels : vérification de l'email

L'email d'un compte professionnel doit être vérifié avant toute connexion et avant la soumission d'une structure. Le serveur envoie un **lien** à l'adresse :

```
<URL_APPLICATION_WEB>/verifier-email#jeton=<64 caractères hexadécimaux>
```

- Le lien est **valable 24 heures** et **ne sert qu'une fois**. Un nouveau lien annule le précédent.
- **Page `/verifier-email` de l'application web** : elle lit le jeton **après `#`** (il n'est jamais envoyé au serveur par le navigateur, ni dans l'en-tête Referer), affiche un bouton « Confirmer mon adresse », et n'envoie le jeton qu'au clic. **Ouvrir le lien ne vérifie rien** : les filtres de messagerie ouvrent les liens tout seuls.
- La page ne garde le jeton nulle part (ni stockage du navigateur, ni journal).
- En développement, l'email n'est pas envoyé : il s'affiche dans le terminal du serveur, avec le lien.

#### `POST /api/professionnels/email/verifier`

- **Appelant** : la page `/verifier-email`, sans jeton d'accès. **Corps** : `{"jeton": "…"}`.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | email vérifié ; la page propose de se connecter |
| 400 | `lien_invalide` | lien inconnu, déjà utilisé, remplacé par un plus récent, expiré, ou email changé depuis l'envoi (jamais distingués) ; message « Ce lien n'est plus valable. Demandez-en un nouveau. » |
| 422 | `donnees_invalides` | champ manquant |
| 500 | `erreur_interne` | panne |

#### `POST /api/professionnels/email/renvoyer`

- **Appelant** : tout le monde, depuis l'écran qui a reçu `email_non_verifie`. **Corps** : `{"email": "…", "mot_de_passe": "…"}` (mot de passe **professionnel**, même forme que la connexion).
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | un nouveau lien est parti ; ou l'email était déjà vérifié (rien n'est envoyé, même réponse) |
| 401 | `identifiants_invalides` | mêmes cas et même message que la connexion ; l'essai compte dans le verrouillage |
| 429 | `trop_de_demandes` | un lien a été envoyé il y a moins d'**une minute** ; en-tête `Retry-After` en secondes |
| 422 | `donnees_invalides` | champ manquant |
| 500 | `erreur_interne` | panne |

### 2.8 Demande de structure (MUNASEB, pharmacie, clinique)

N'importe quel patient peut créer sa structure ; il en devient **responsable**. Tout se fait avec le **jeton patient** (`laaficare:patient`). L'écran qui portera ce parcours (application mobile, ou espace patient du web) reste à décider. Types ouverts : `munaseb`, `pharmacie` (2026-11-16) et `clinique` (2026-11-17, module médecin ; deux pièces exigées : `autorisation_creation` et `autorisation_ouverture_exploitation`). Les autres (`hopital`, `laboratoire`) attendent la liste de leurs pièces justificatives : 422 `type_non_disponible`.

Cycle : `brouillon` (création) → `en_attente` (soumission) → `validee` ou `refusee` (décision de l'équipe LaafiCare, étape 3.8). Une seule demande non validée à la fois par personne.

**Une seule MUNASEB peut être validée** (ou suspendue). Plusieurs demandes MUNASEB peuvent exister en brouillon ou en attente : sinon, n'importe qui bloquerait le vrai responsable de la mutuelle avec un brouillon. **La création et la soumission d'une seconde MUNASEB ne sont donc jamais refusées pour ce motif** ; le refus arrive à la **validation** par l'équipe LaafiCare (étape 3.8, code à documenter alors).

**Pour les applications web et mobile** (liste complète dans `docs/points-ouverts.md`, « À faire dans les applications web et mobile ») :
- le lien de vérification a la forme `<URL_APPLICATION_WEB>/verifier-email#jeton=<64 caractères hexadécimaux>` ;
- la page `/verifier-email` lit le jeton **après `#`** et l'envoie par **`POST /api/professionnels/email/verifier`** (section 2.7), **jamais par un GET** : RFC 9110 §9.2.1 (un GET est une méthode « sûre », qui ne doit rien changer ; les filtres de messagerie ouvrent les liens tout seuls) et RFC 3986 §3.5 (le fragment après `#` n'est jamais envoyé au serveur par le navigateur) ;
- réponses : **204** (email vérifié) ou **400 `lien_invalide`** (lien inconnu, déjà utilisé, remplacé ou expiré).

#### `POST /api/structures`

- **Appelant** : en-tête `Authorization: Bearer <jeton_acces patient>`.
- **Corps** :

```json
{
  "type": "munaseb",
  "nom": "MUNASEB",
  "telephone": "70 00 00 01",
  "telephone_urgence": "70 00 00 02",
  "region": "Région fictive",
  "province": "Province fictive",
  "commune": "Commune fictive",
  "parcelle": "12",
  "lot": "3",
  "section": "B",
  "latitude": 12.3714,
  "longitude": -1.5197,
  "email": "contact.munaseb@example.org",
  "email_professionnel": "responsable@example.org",
  "mot_de_passe_professionnel": "Exemple#2026"
}
```

- **Obligatoires** : `type`, `nom`, `telephone` (numéro de service), `telephone_urgence` (peut être le même), `email` (email **de la structure**), `region`, `province`, `commune`, `latitude`, `longitude`, `mot_de_passe_professionnel`. **Facultatifs** : `parcelle`, `lot`, `section` (zones non loties).
- **`nom`** : **100 caractères au plus**, sans retour à la ligne ni caractère invisible. Il est repris dans le SMS d'invitation : en lettres sans accent, le SMS tient alors en 2 parties.
- **`email`** : la coordonnée de l'établissement. Elle n'est pas vérifiée par lien et ne sert pas à se connecter.
- **`email_professionnel`** : l'email de connexion du responsable. Obligatoire **seulement si la personne n'a pas encore de compte professionnel** ; le compte est alors créé avec cet email et ce mot de passe, qui doit suivre les règles de la section 1.5 (il peut être le même que celui du compte patient). Si l'identité a déjà un email, il doit être le même (majuscules près). C'est cet email qui reçoit le lien de vérification (section 2.7). Avec un compte professionnel existant, `email_professionnel` est ignoré et `mot_de_passe_professionnel` est son mot de passe actuel.
- **Région, province, commune** : texte libre (aucune liste officielle fermée pour l'instant).
- **Position** : envoyée par l'application (position de la structure, confirmée ou corrigée à l'écran), en degrés décimaux ; elle doit tomber dans le rectangle du Burkina Faso (latitude 8 à 16 nord, longitude 6 ouest à 3 est, soit `-6` à `3`). Enregistrée avec 6 décimales.
- Les deux numéros suivent la section 1.6.
- **Forme des deux emails** (`email` et `email_professionnel`), un seul contrôle :
  - lettres, chiffres et signes **ASCII seulement**, domaine compris (pas d'accent, pas de domaine international pour l'instant) ;
  - ni espace, ni retour à la ligne, ni caractère de contrôle ;
  - 64 caractères au plus avant le `@`, 254 au total ;
  - ni `Nom <adresse>`, ni adresse IP entre crochets à la place du domaine ;
  - sinon, règle de l'adresse valide du HTML5.
  - Les majuscules sont gardées telles que saisies ; deux adresses qui ne diffèrent que par les majuscules sont la même adresse.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 201 | — | `{"structure_id": "…", "statut": "brouillon", "email_a_verifier": true}` ; si le compte professionnel vient d'être créé, le lien de vérification est parti (section 2.7) |
| 401 | `identifiants_invalides` | compte professionnel existant : mot de passe professionnel faux, compte verrouillé ou désactivé ; message « Mot de passe professionnel incorrect. » (**le jeton patient reste valable** : ne pas renvoyer à la connexion) |
| 401 | `jeton_invalide`, `authentification_requise` | jeton patient absent, expiré ou révoqué |
| 409 | `demande_deja_en_cours` | une demande non validée existe déjà (brouillon, en attente ou refusée) |
| 409 | `email_different` | l'identité a déjà un autre email que `email_professionnel` |
| 409 | `email_deja_utilise` | `email_professionnel` appartient à une autre personne |
| 409 | `second_facteur_requis` | le compte professionnel a un second facteur : pas encore pris en charge ici |
| 422 | `donnees_invalides` | champ obligatoire manquant ou vide, `email` ou `email_professionnel` refusé par le contrôle ci-dessus (le message nomme le champ : « Le champ « email » n'est pas une adresse email valide. »), type inconnu |
| 422 | `type_non_disponible` | type connu mais pas encore ouvert (clinique, hôpital, laboratoire). **`pharmacie` est ouvert depuis le 2026-11-16** (officine) |
| 422 | `nom_trop_long` | nom de plus de 100 caractères |
| 422 | `nom_invalide` | nom avec un retour à la ligne ou un caractère invisible |
| 422 | `telephone_service_invalide` | numéro de service : format non reconnu (section 1.6) |
| 422 | `telephone_urgence_invalide` | numéro d'urgence : format non reconnu (section 1.6) |
| 422 | `localisation_hors_burkina` | position hors du rectangle |
| 422 | `mot_de_passe_non_conforme` | avec `regles_non_respectees` (section 1.5) |
| 500 | `erreur_interne` | panne |

#### `POST /api/structures/{id}/soumettre`

- **Appelant** : jeton patient du **créateur**. Pas de corps. La MUNASEB se soumet sans pièce justificative ; une **pharmacie** doit avoir déposé son `autorisation_ouverture_exploitation` (route suivante).
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | demande soumise : statut `en_attente` |
| 403 | `email_non_verifie` | l'email professionnel n'est pas encore vérifié (section 2.7) |
| 403 | `acces_refuse` | compte professionnel désactivé |
| 404 | `structure_inconnue` | structure inexistante, ou créée par une autre personne (jamais distingués) |
| 409 | `transition_invalide` | la structure n'est pas en brouillon ; `statut_actuel` dans la réponse |
| 409 | `pieces_manquantes` | pièces exigées non déposées ; **`pieces_manquantes`** dans la réponse, par exemple `["autorisation_ouverture_exploitation"]` (ajouté le 2026-11-16) |
| 500 | `erreur_interne` | panne |

#### `POST /api/structures/{id}/pieces/{type_piece}?numero=&date_piece=`

Dépôt d'une pièce justificative (ajouté le 2026-11-16), **une requête par fichier** (réseau faible).

- **Appelant** : jeton patient du **créateur**, sur une demande **en brouillon**.
- **`type_piece`** : `autorisation_ouverture_exploitation` pour une pharmacie (seule pièce demandée aujourd'hui ; liste à confirmer par la DGPML ou l'Ordre des pharmaciens). `autorisation_creation` n'est pas demandée à une pharmacie.
- **Paramètres** : `numero` (le numéro inscrit sur le document, 100 caractères au plus, gardé tel quel) et `date_piece` (`AAAA-MM-JJ`, jamais après aujourd'hui).
- **Corps** : **le fichier lui-même**, tel quel (pas de formulaire). PDF, JPEG ou PNG, **5 Mo au plus**. Le serveur reconnaît le type par les premiers octets du fichier : l'en-tête `Content-Type` et le nom du fichier ne servent à rien et ne sont pas gardés.
- Un nouveau dépôt du même type **remplace** la pièce active ; l'ancienne reste gardée, inactive.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 201 | — | `{"id", "type_piece", "numero", "date_piece", "type_mime", "taille", "sha256", "active", "date_depot"}` |
| 404 | `structure_inconnue` | structure inexistante, ou créée par une autre personne |
| 409 | `transition_invalide` | la demande n'est plus en brouillon ; `statut_actuel` dans la réponse |
| 413 | `fichier_trop_grand` | plus de 5 Mo |
| 415 | `format_refuse` | ni PDF, ni JPEG, ni PNG (SVG et HEIC refusés) |
| 422 | `piece_non_prevue` | ce type de pièce n'est pas demandé pour ce type de structure |
| 422 | `fichier_vide`, `donnees_invalides` | fichier vide ; numéro vide, trop long ou réduit à « N° », date future |

#### `GET /api/structures/{id}/pieces`

- **Appelant** : jeton patient du **créateur**. **Réponse 200** : `{"pieces": [ … ]}`, les pièces actives, même forme que ci-dessus, **sans le fichier**. 404 `structure_inconnue`.

#### `POST /api/structures/{id}/abandonner`

- **Appelant** : jeton patient du **créateur**. Pas de corps. **L'application demande une confirmation avant l'appel** : l'abandon est définitif.
- Possible depuis `brouillon` ou `refusee` seulement. Une demande `en_attente` ne s'abandonne pas : l'équipe LaafiCare l'examine peut-être déjà.
- **Effets** : statut `abandonnee` (état final) ; rien n'est effacé ; l'affectation `responsable` est désactivée ; la personne peut créer une nouvelle demande, avec son compte professionnel existant.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | demande abandonnée |
| 404 | `structure_inconnue` | structure inexistante, ou créée par une autre personne (jamais distingués) |
| 409 | `transition_invalide` | la structure n'est ni en brouillon ni refusée ; `statut_actuel` dans la réponse |
| 500 | `erreur_interne` | panne |

#### `GET /api/structures/mes-demandes`

- **Appelant** : jeton patient. **Réponse 200** :

```json
{
  "email_a_verifier": false,
  "demandes": [
    {
      "id": "…",
      "type": "munaseb",
      "nom": "MUNASEB",
      "statut": "refusee",
      "date_creation": "2026-10-21T08:00:00Z",
      "licence_expire_le": null,
      "motif_refus": "…"
    }
  ]
}
```

- Toutes les structures créées par la personne, les plus récentes d'abord. `motif_refus` n'est rempli que pour une structure `refusee`. `email_a_verifier` vaut `true` si le compte professionnel existe et que son email n'est pas vérifié : l'application propose alors de renvoyer le lien.

### 2.9 Administrateurs : décisions sur les demandes de structure

Réservé à l'équipe LaafiCare : jeton d'accès `laaficare:administrateur` (session, email vérifié, **second facteur actif** ; sinon 401 `jeton_invalide`). Validation (licence) et refus (motif), depuis une demande `en_attente` seulement. Chaque décision est tracée (administrateur, date, motif, licence), sans jamais être modifiée.

**Séparation des tâches** : un administrateur ne décide jamais sur une structure dont il est responsable, même avec un autre compte de la même personne. Il reçoit **403 `decision_sur_sa_propre_structure`**, et un autre membre de l'équipe doit décider. Source : NIST SP 800-53 rév. 5, contrôle AC-5 « Separation of Duties » (application : décision du porteur, 2026-10-21).

#### `GET /api/administrateurs/structures?statut=&limite=&apres=`

- `statut` facultatif : `brouillon`, `en_attente`, `validee`, `refusee`, `suspendue`, `abandonnee`. Pagination par curseur, comme la liste des remboursements : `limite` de 1 à 100 (50 par défaut), `apres` = `suivant` de la page précédente. Les plus anciennes d'abord.
- **Réponse 200** : `{"structures": [{"id", "type", "nom", "statut", "date_creation", "commune", "responsable_nom", "responsable_prenom"}], "suivant": "…" | null}`. 400 `parametre_invalide` pour une limite hors bornes.

#### `GET /api/administrateurs/structures/{id}`

- **Réponse 200** :
  - les champs de la liste ;
  - les coordonnées de la structure (`telephone`, `telephone_urgence`, `email`, `region`, `province`, `parcelle`, `lot`, `section`, `latitude`, `longitude`) et `licence_expire_le` ;
  - le responsable : `responsable_telephone`, `responsable_email`, `responsable_email_verifie` ;
  - `decisions` : l'historique, avec `decision`, `motif`, `licence_expire_le`, `date_decision`, et le nom et le prénom de l'administrateur.
- **Avertissements MUNASEB** (vides pour les autres types) :
  - `autres_demandes_munaseb_en_attente` : les autres demandes MUNASEB en attente ;
  - `munaseb_deja_validee` : la MUNASEB déjà validée ou suspendue, si elle existe.
- **`pieces`** (ajouté le 2026-11-16) : toutes les pièces, anciennes comprises (`active` à `false`), **sans le fichier** ; chacune avec **`meme_numero_dans`**, la liste des autres demandes dont une pièce active du même type porte le même numéro (comparé sans espaces, sans « N° », majuscules et tirets unifiés). Une seule structure peut être validée avec un même numéro : la seconde validation répond 409 `autorisation_deja_validee`.
- 404 `structure_inconnue`.

#### `GET /api/administrateurs/pieces/{id}/fichier`

- **Appelant** : jeton administrateur. **La seule route qui renvoie le contenu d'une pièce.**
- **Réponse 200** : le fichier, avec `Content-Type` (le type reconnu au dépôt), `Content-Disposition: attachment; filename="piece-<id>.pdf"` (nom choisi par le serveur), `X-Content-Type-Options: nosniff` et `Cache-Control: no-store`. Le navigateur l'enregistre au lieu de l'ouvrir.
- 404 `piece_inconnue`.

#### `POST /api/administrateurs/structures/{id}/valider`

- **Corps** : `{"licence_expire_le": "2027-10-21"}`, ou `{}`.
  - **`licence_expire_le` absent ou `null` : licence sans fin.**
  - Présent : la licence est valable **jusqu'à cette date comprise** (heure du Burkina Faso), puis l'établissement n'est plus exerçable. Elle doit être **postérieure à aujourd'hui**.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | structure validée ; l'affectation `responsable` devient exerçable ; un email au texte minimal prévient le responsable (« Votre demande de structure a été traitée… ») |
| 403 | `decision_sur_sa_propre_structure` | séparation des tâches (voir plus haut) |
| 404 | `structure_inconnue` | identifiant inconnu |
| 409 | `transition_invalide` | la demande n'est pas `en_attente` ; `statut_actuel` dans la réponse |
| 409 | `email_responsable_non_verifie` | l'email professionnel du responsable n'est pas vérifié : la validation attend ; le refus reste possible |
| 409 | `munaseb_deja_validee` | une MUNASEB est déjà validée ou suspendue : une seule est admise |
| 409 | `autorisation_deja_validee` | un numéro d'autorisation de la demande est déjà validé pour une autre structure (pas pour la MUNASEB, sans pièce) |
| 422 | `licence_invalide` | date de fin de licence passée ou égale à aujourd'hui |
| 500 | `erreur_interne` | panne |

#### `POST /api/administrateurs/structures/{id}/refuser`

- **Corps** : `{"motif": "…"}`, obligatoire, 1 000 caractères au plus. Le motif est visible par le responsable dans `GET /api/structures/mes-demandes`, mais n'est jamais mis dans un email.
- **Réponses** :
  - 204 : demande refusée. Si l'email du responsable est vérifié, il reçoit le même email minimal ; sinon, aucun email ne part.
  - 403, 404, 409 `transition_invalide` : comme pour la validation.
  - 422 `motif_vide`, `motif_trop_long`.
- Après un refus, le responsable abandonne sa demande (section 2.8) et en crée une nouvelle. Corriger et soumettre à nouveau la même demande viendra plus tard.

### 2.10 Invitations (MUNASEB, pharmacie, clinique)

Le **responsable** d'une structure validée invite au numéro de téléphone. La personne répond depuis son **compte patient**. Une invitation vaut **7 jours**. Une invitation expirée se lit `expiree` et n'a plus aucune suite.

- **Ce que voit l'invitant** : jamais si un compte existe pour le numéro. La réponse et le SMS sont les mêmes dans tous les cas, et aucun nom n'apparaît avant l'acceptation.
- **Le SMS**, sans lien ni code, entièrement dans l'alphabet GSM 7 bits (3GPP TS 23.038) : `LaafiCare : invitation à rejoindre "{structure}" comme {agent MUNASEB | responsable | pharmacien | auxiliaire de pharmacie | medecin | infirmier | agent d'accueil}. Ouvrez ou installez LaafiCare avec ce numéro pour répondre avant le {JJ/MM/AAAA}. LaafiCare ne demande jamais votre mot de passe.` Avec un nom de 100 caractères GSM 7 bits, il tient en 2 parties au plus. Un nom qui contient un caractère hors de cet alphabet (un `ê`, par exemple) le fait partir en UCS-2, plus long.
- **En développement**, le SMS s'affiche dans le terminal du serveur.
- **Pour la MUNASEB**, deux rôles : `agent_assurance_munaseb` et `responsable`.
- **Pour une pharmacie** (ajouté le 2026-11-16) : `pharmacien`, `responsable` et, depuis le 2026-11-17, `auxiliaire_pharmacie` (remplace la décision D5 « pas de préparateur »).
- **Pour une clinique** (ajouté le 2026-11-17) : `medecin`, `infirmier` (la sage-femme est une spécialité d'infirmier, section 2.17), `agent_accueil` et `responsable`.
- Tout autre rôle : 422 `role_non_permis`.

#### Côté responsable : jeton `laaficare:professionnel_en_exercice` sur une affectation `responsable`

La structure est celle de l'affectation active, jamais un identifiant passé dans l'adresse. Avec une autre affectation, la réponse est **403 `acces_refuse`**.

**`POST /api/professionnels/invitations`**, corps `{"telephone": "70 00 00 02", "role": "agent_assurance_munaseb"}` :

| Code | `code` | Cas |
|---|---|---|
| 201 | — | `{"invitation_id", "expire_le"}`, **la même réponse que le numéro ait un compte ou non** |
| 409 | `invitation_deja_en_attente` | une invitation en attente existe déjà pour ce numéro et ce rôle (l'annuler pour en refaire une) |
| 409 | `trop_d_invitations_en_attente` | 50 invitations en attente dans la structure |
| 409 | `structure_non_validee` | la structure n'est plus validée |
| 422 | `telephone_invalide` | numéro mal formé (section 1.6) |
| 422 | `role_non_permis` | rôle impossible pour ce type de structure |
| 422 | `auto_invitation` | le responsable invite son propre numéro (séparation des tâches, NIST SP 800-53 AC-5) |
| 429 | `trop_de_demandes` | 20 invitations créées par la structure en 24 heures |

Au-delà de **3 SMS d'invitation par numéro en 24 heures**, toutes structures confondues, l'invitation est enregistrée et la réponse est la même, mais **aucun SMS ne part**. La personne la voit quand même dans son application.

**`GET /api/professionnels/invitations?limite=&avant=`** :
- réponse 200 `{"invitations": [{"id", "telephone", "role", "statut", "date_creation", "expire_le", "date_reponse", "accepte_par_nom", "accepte_par_prenom"}], "suivant"}` ;
- les plus récentes d'abord ; `avant` = `suivant` de la page précédente ; `limite` de 1 à 100 (50 par défaut) ;
- `statut` vaut `en_attente`, `expiree`, `acceptee`, `refusee` ou `annulee` ;
- le nom et le prénom n'apparaissent qu'après l'acceptation.

**`POST /api/professionnels/invitations/{id}/annuler`**, par tout responsable de la structure :
- 204 ;
- 404 `invitation_inconnue` ;
- 409 `invitation_expiree` ;
- 409 `transition_invalide`, avec `statut_actuel`.

#### Côté invité : jeton `laaficare:patient`

Seules les invitations adressées au **numéro du compte patient** existent pour lui. Toute autre donne **404 `invitation_inconnue`**.

**`GET /api/patients/invitations`** : réponse 200 `{"invitations": [{"id", "structure_nom", "structure_type", "role", "expire_le", "invitant_nom", "invitant_prenom"}]}`. Seules celles qui sont en attente, non expirées, dans une structure validée y figurent. Le nom de l'invitant permet de juger si l'invitation est légitime.

**`POST /api/patients/invitations/{id}/accepter`** :
- **Sans compte professionnel** : `{"email_professionnel": "…", "mot_de_passe_professionnel": "…"}`. Le compte est créé avec **les mêmes règles qu'à la création d'une structure** (section 2.8), et un lien de vérification est envoyé. **L'affectation n'est exerçable qu'une fois l'email vérifié.**
- **Avec un compte professionnel** : `{"mot_de_passe_professionnel": "…"}`. Le mot de passe est compté dans le verrouillage.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | `{"affectation_id", "email_a_verifier"}` |
| 401 | `identifiants_invalides` | mot de passe professionnel faux, compte verrouillé ou désactivé (« Mot de passe professionnel incorrect. ») ; **le jeton patient reste valable** |
| 404 | `invitation_inconnue` | — |
| 409 | `invitation_expiree`, `transition_invalide` | plus en attente |
| 409 | `structure_non_validee` | la structure n'est plus validée |
| 409 | `affectation_deja_active` | la personne a déjà ce rôle dans cette structure |
| 409 | `email_different`, `email_deja_utilise`, `second_facteur_requis` | comme à la création d'une structure |
| 409 | `requete_simultanee` | une autre requête de la même personne vient de créer son compte professionnel : réessayer |
| 422 | `donnees_invalides`, `mot_de_passe_non_conforme` | comme à la création d'une structure |

**`POST /api/patients/invitations/{id}/refuser`** :
- 204 ;
- 404 `invitation_inconnue` ;
- 409 `invitation_expiree` ;
- 409 `transition_invalide`.

### 2.11 Mot de passe oublié : professionnels et administrateurs

Les patients gardent la réinitialisation par SMS (section 2.2.4). Pour un compte professionnel ou administrateur, un **lien** est envoyé à l'email du compte.

#### `POST /api/professionnels/mot-de-passe/oubli` et `POST /api/administrateurs/mot-de-passe/oubli`

- **Corps** : `{"email": "…"}`. Une route par sorte de compte : une même personne peut avoir un compte professionnel et un compte administrateur avec le même email.
- **Réponse** : **toujours 204, tout de suite**, que l'email soit connu, inconnu, non vérifié, d'un compte désactivé, ou mal formé. Le travail se fait ensuite, en tâche de fond. Seul un champ manquant donne 422 `donnees_invalides`.
- **Un lien ne part que vers un compte actif dont l'email est vérifié.** Un compte verrouillé le reçoit, puisque la réinitialisation lève le verrouillage.
- **Limites par compte** : une minute au moins entre deux liens ; au plus 3 liens valables en attente. Au-delà, rien ne part, avec la même réponse.
- **Le lien** : `<URL_APPLICATION_WEB>/reinitialiser-mot-de-passe#jeton=<64 caractères hexadécimaux>`, valable **30 minutes**, **une seule fois**. Une nouvelle demande n'annule pas les liens précédents.
- L'application affiche toujours le même message : « Si un compte correspond à cette adresse, un lien vient d'être envoyé. »

#### `POST /api/mot-de-passe/reinitialiser`

- **Appelant** : la page `/reinitialiser-mot-de-passe` de l'application web. Elle lit le jeton **après `#`**, sans jamais le garder, et l'envoie **par POST au clic, jamais par un GET**. Elle porte l'en-tête `Referrer-Policy: no-referrer`. La même page sert aux administrateurs.
- **Corps** : `{"jeton": "…", "nouveau_mot_de_passe": "…"}`.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | mot de passe changé ; l'application renvoie à l'écran de connexion |
| 400 | `lien_invalide` | lien inconnu, déjà utilisé, annulé, expiré, ou email changé depuis l'envoi (jamais distingués) |
| 422 | `mot_de_passe_non_conforme` | avec `regles_non_respectees` (section 1.5) ; le lien reste valable |
| 422 | `mot_de_passe_identique_patient` | administrateur seulement : le même mot de passe que son compte patient ; le lien reste valable |
| 500 | `erreur_interne` | panne |

**Après une réinitialisation réussie** :
- toutes les sessions sont fermées : les anciens jetons, d'accès et de renouvellement, sont refusés ;
- le verrouillage est levé ;
- les autres liens en attente sont annulés ;
- **le second facteur (TOTP) ne change pas** : il reste exigé à la connexion ;
- **aucune session n'est ouverte** : la personne se reconnecte ;
- un email d'alerte « mot de passe modifié » part, avec la date (heure du Burkina Faso) et l'adresse de l'équipe à contacter.

### 2.12 Membres d'une structure et désactivation d'un accès (MUNASEB pour l'instant)

Jeton `laaficare:professionnel_en_exercice` sur une affectation **`responsable`** ; la structure est celle de l'affectation active, jamais un identifiant passé dans l'adresse. Avec une autre affectation : **403 `acces_refuse`**. Les routes `/api/professionnels/affectations` restent réservées aux affectations de la personne connectée.

**`GET /api/professionnels/membres`** : réponse 200 `{"membres": [{"affectation_id", "nom", "prenom", "role", "date_creation"}]}`, les affectations **actives** de la structure, les plus anciennes d'abord. **Ni téléphone ni email.**

**`POST /api/professionnels/membres/{id}/desactiver`** (`id` = `affectation_id` de la liste), sans corps :

| Code | `code` | Cas |
|---|---|---|
| 204 | — | accès retiré |
| 403 | `acces_refuse` | l'affectation active n'est pas `responsable` |
| 404 | `affectation_inconnue` | inexistante, ou d'une autre structure (jamais distingués) |
| 409 | `transition_invalide` | déjà désactivée ; `statut_actuel` dans la réponse |
| 409 | `dernier_responsable` | c'est le dernier responsable actif de la structure |
| 500 | `erreur_interne` | panne |

- Pour la MUNASEB : rôles `agent_assurance_munaseb` et `responsable`. Un responsable peut retirer **son propre accès** s'il reste un autre responsable actif.
- **Effets** : l'affectation est désactivée (datée, avec l'auteur), **jamais supprimée** ; les sessions ouvertes **sur cette affectation** sont fermées tout de suite : le jeton d'accès de la personne est refusé (401 `jeton_invalide`), son renouvellement aussi. Son compte, ses autres affectations et ses autres sessions ne changent pas. Pour revenir, la personne reçoit une nouvelle invitation.
- **Email** à la personne, après l'enregistrement, si son email est vérifié : « Votre accès à une structure LaafiCare a été retiré le JJ/MM/AAAA à HH:MM (heure du Burkina Faso). Si vous pensez que c'est une erreur, contactez le responsable de la structure ou l'équipe LaafiCare à {adresse}. » Un échec d'envoi n'annule rien.
- L'application demande une confirmation avant l'appel.

### 2.13 Espace patient : droits MUNASEB, demandes, notifications

Jeton `laaficare:patient` seulement. **Le patient est toujours celui du jeton** : aucune route ne prend un NIP ni un identifiant de patient ou de contrat. Une demande ou une notification d'une autre personne répond exactement comme une qui n'existe pas (404). Un compte patient sans dossier patient donne 500 `erreur_interne`, comme `GET /api/patients/moi`.

#### `GET /api/patients/munaseb/droits`

**Toujours 200**, avec un champ `etat`. Les champs sans objet valent `null`. Le patient ne reçoit jamais l'université, l'UFR, le matricule ni la personne à prévenir.

```json
{
  "etat": "couvert",
  "numero_carte": "…",
  "periode_en_cours": {"debut": "2026-04-15", "fin": "2027-04-14"},
  "periode_suivante": null,
  "plafond_fcfa": 100000,
  "consomme_fcfa": 96000,
  "solde_fcfa": 4000,
  "debut_couverture": null,
  "fin_derniere_periode": null
}
```

| `etat` | Champs remplis | Texte conseillé |
|---|---|---|
| `aucun_contrat` | aucun | « Vous n'avez pas de contrat MUNASEB. » |
| `couvert` | `numero_carte`, `periode_en_cours`, `plafond_fcfa`, `consomme_fcfa`, `solde_fcfa` ; `periode_suivante` si un renouvellement est déjà payé | « Couvert jusqu'au JJ/MM/AAAA. Reste : N FCFA. » |
| `en_carence` | `numero_carte`, `periode_suivante`, `debut_couverture` | « Couverture à partir du JJ/MM/AAAA. » (jamais « expiré ») |
| `expire` | `numero_carte`, `fin_derniere_periode` | « Votre contrat a pris fin le JJ/MM/AAAA. » |
| `suspendu` | `numero_carte` | « Votre contrat est suspendu. Contactez la MUNASEB. » |

- Le solde est **indicatif** : c'est celui qu'utilise l'agent, recalculé au moment de chaque validation. Seuls les montants validés et payés le consomment.
- La date du jour est celle du Burkina Faso.
- `numero_carte` (précisé le 2026-11-16, étape 5) : le numéro change chaque année ; c'est celui de la **dernière période déjà commencée**. Pendant la carence d'une première adhésion, celui de la première carte. La carte d'une période qui n'a pas encore commencé n'est pas montrée.
- `plafond_fcfa` : le plafond de la période en cours, figé à sa création (100 000 FCFA aujourd'hui).

#### `GET /api/patients/munaseb/demandes?statut=&limite=&avant=`

- Les demandes du patient, **les plus récentes d'abord**. `statut` facultatif : `en_attente`, `en_cours`, `valide`, `rejete`, `paye`. `limite` de 1 à 100 (50 par défaut) ; `avant` = `suivant` de la page précédente. Sans contrat : liste vide.
- **Réponse 200** : `{"demandes": [{"id", "statut", "type_acte", "date_acte", "partenaire", "montant_acte_fcfa", "montant_demande_fcfa", "montant_rembourse_fcfa", "date_demande", "date_dernier_changement"}], "suivant": "…" | null}`.
  - `montant_demande_fcfa` : la **part MUNASEB demandée** (taux ou forfait du type d'acte) ;
  - `montant_rembourse_fcfa` : fixé à la validation, `null` avant ; il peut être plus petit que la part demandée si le plafond est atteint.
- 400 `parametre_invalide` : limite hors bornes, statut inconnu, curseur qui n'est pas un UUID.

#### `GET /api/patients/munaseb/demandes/{id}`

- **Réponse 200** : les champs de la liste, plus :
  - `motif_rejet` : rempli pour une demande rejetée. Il n'est **jamais** dans la liste ni dans une notification ;
  - `periode` : `{"debut", "fin"}`, la période d'adhésion de la demande ;
  - `plafond_atteint` : `true` quand la demande a été validée pour moins que la part demandée ;
  - `historique` : `[{"statut", "date"}]`, du plus ancien au plus récent, **sans l'agent**.
- 404 `demande_inconnue` : demande inexistante ou d'un autre patient. 400 `chemin_invalide`.
- Jamais renvoyés : le nom ou l'affectation d'un agent, le NIP, la référence de l'acte chez le partenaire.

Libellés conseillés : `en_attente` « Reçue », `en_cours` « En cours de traitement », `valide` « Validée », `rejete` « Rejetée », `paye` « Payée ». **Une demande rejetée, un contrat expiré ou suspendu ne s'affichent ni en rouge ni en orange** (couleurs réservées aux alertes de santé) : couleur neutre, avec une icône et un texte.

#### `GET /api/patients/notifications?limite=&avant=`

- **Réponse 200** : `{"notifications": [{"id", "type", "titre", "message", "lu", "date_creation", "demande_id"}], "non_lues": 3, "suivant": "…" | null}`.
  - `type` (ajouté le 2026-11-17) : `remboursement_statut` ou `acces_urgence`.
  - Les plus récentes d'abord ; `limite` de 1 à 100 (50 par défaut) ; `avant` = `suivant`.
  - `non_lues` : le nombre total de notifications non lues, pour le badge.
  - `demande_id` : la demande concernée, à ouvrir avec la route de détail ; `null` pour une notification d'**accès d'urgence** à son dossier (bris de glace, depuis le 2026-11-17, section 2.19) : l'application ouvre alors le journal des accès (section 2.18).
- Les textes restent minimaux : ni montant, ni type d'acte, ni motif.

#### `POST /api/patients/notifications/{id}/lue` et `POST /api/patients/notifications/lues`

- Sans corps. **204** : la notification est lue, même si elle l'était déjà (la requête peut être rejouée sans effet). La seconde route marque toutes les notifications de la personne.
- 404 `notification_inconnue` : notification inexistante ou d'une autre personne.

### 2.14 Agent MUNASEB : contrats et paramètres (étape 5)

Ces routes demandent un **jeton professionnel en exercice** dans la MUNASEB validée et sous licence (401 pour un autre jeton) :
- **contrats** (adhésion, recherche, détail, renouvellement) : affectation `agent_assurance_munaseb`, comme les remboursements (section 2.3) ;
- **lecture des paramètres** et de leur historique : agent **ou** responsable de la MUNASEB ;
- **modification des paramètres** : **responsable** de la MUNASEB seulement (précisé le 2026-11-16 : ces valeurs touchent l'argent de toutes les adhésions suivantes).

Tout autre rôle ou une structure qui n'est pas la MUNASEB validée : 403 `acces_refuse`, avant la lecture du corps. Ces routes remplacent le contrat posé à la main en SQL.

**Règles de calcul d'une période** (CLAUDE.md §12, réponses du porteur du 2026-10-22, **à confirmer par la MUNASEB**) :
- la **date de paiement** est toujours la date du jour au Burkina, fixée par le serveur ; l'agent ne la saisit jamais ;
- **première adhésion** : début = date de paiement + carence (1 mois) ; fin = date de paiement + 12 mois, **jour anniversaire compris** (adhésion le 09/04/2024 : couvert du 09/05/2024 au 09/04/2025) ;
- **renouvellement payé au plus tard le dernier jour** : la nouvelle période commence le lendemain de la fin, sans carence, et finit 12 mois après la fin précédente ;
- **renouvellement payé dans le délai de grâce** (10 jours après la fin) : couvert **à partir du jour du paiement**, sans carence ; les jours entre la fin et le paiement **ne sont pas couverts** ; la fin reste la date anniversaire suivante ;
- **au-delà du délai de grâce** : comme une première adhésion (nouvelle carence) ;
- au plus **une période d'avance** : pas de renouvellement pendant une carence, ni quand la période suivante est déjà payée.

Le numéro de carte **change chaque année** : chaque période porte le sien, unique. Le plafond d'une période est **figé à sa création** (un changement de paramètre ne touche que les périodes suivantes). La **suspension** d'un contrat n'a pas de route : ses règles sont à demander à la MUNASEB.

#### `GET /api/assurance-munaseb/parametres`

- **Réponse 200** : la version en vigueur.

```json
{
  "id": "…",
  "cotisation_fcfa": 5000,
  "fiche_fcfa": 100,
  "delai_grace_jours": 10,
  "carence_mois": 1,
  "plafond_periode_fcfa": 100000,
  "date_effet": "2026-11-16T09:16:59.418551Z",
  "auteur_nom": null,
  "auteur_prenom": null
}
```

- `auteur_nom`, `auteur_prenom` : le responsable de la MUNASEB qui a posé cette version ; `null` pour les valeurs initiales.

#### `PUT /api/assurance-munaseb/parametres`

- **Appelant** : le **responsable** de la MUNASEB, son affectation `responsable` active. Un agent reçoit **403 `acces_refuse`**.
- **Corps** : les cinq valeurs, toutes obligatoires (`cotisation_fcfa`, `fiche_fcfa`, `delai_grace_jours`, `carence_mois`, `plafond_periode_fcfa`).
- **Réponse 200** : la version en vigueur après l'écriture, même forme que ci-dessus. Des valeurs identiques aux valeurs en vigueur ne créent pas de nouvelle version (la même est renvoyée).
- 422 `donnees_invalides` : montant négatif, délai de grâce hors de 0 à 60 jours, carence hors de 0 à 12 mois, plafond nul ou négatif (garde-fous contre une faute de frappe), ou champ manquant.
- Une version ne se modifie ni ne s'efface jamais : l'historique garde l'ancienne et la nouvelle valeur de chaque changement.

#### `GET /api/assurance-munaseb/parametres/historique?limite=&avant=`

- **Réponse 200** : `{"versions": [ … ], "suivant": "…" | null}`, la plus récente d'abord, chaque version de la même forme que ci-dessus. `limite` de 1 à 100 (50 par défaut) ; `avant` = `suivant` de la page précédente.
- 400 `parametre_invalide` : limite hors bornes, curseur qui n'est pas un UUID.

#### `POST /api/assurance-munaseb/contrats`

Adhésion d'un patient. Le patient est désigné par son **NIP** (montré par son application) **et sa date de naissance** (lue sur sa pièce d'identité) : les deux doivent correspondre.

- **Corps** :

```json
{
  "nip": "1000000000000008",
  "date_naissance": "2000-01-01",
  "numero_carte": "0552/2026",
  "ufr": "UFR SEA",
  "universite": "Université Joseph Ki-Zerbo",
  "num_matricule": "MAT-0001",
  "personne_a_prevenir": null,
  "niveau_etudes": "Licence 2",
  "antenne": null,
  "paiement": {"mode": "mobile_money", "reference_transaction": "OM-123456", "date_declaree": "2026-11-16"}
}
```

- Obligatoires : `nip`, `date_naissance`, `numero_carte` (50 caractères au plus), `ufr`, `universite` (100 au plus), `num_matricule` (50 au plus), `paiement.mode`. Facultatifs : `personne_a_prevenir` (200 au plus), `niveau_etudes`, `antenne` (100 au plus). Les espaces de début et de fin sont retirées ; un champ facultatif vide vaut `null`.
- `paiement.mode` : `especes` ou `mobile_money`. `reference_transaction` est **obligatoire pour le mobile money et refusée pour les espèces** (100 caractères au plus ; une référence ne sert qu'une fois). `date_declaree` (facultative) : la date que l'agent lit, par exemple sur la transaction ; **pour information seulement**, jamais après aujourd'hui. Les montants payés ne sont pas saisis : ce sont la cotisation et la fiche en vigueur.
- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 201 | — | le détail du contrat (route suivante) |
| 422 | `nip_invalide` | NIP mal formé (16 chiffres, clé de Luhn) |
| 404 | `patient_inconnu` | aucun patient pour ce NIP **et** cette date de naissance (les deux cas ne sont jamais distingués) |
| 409 | `contrat_deja_existant` | ce patient a déjà un contrat (utiliser le renouvellement) |
| 409 | `numero_carte_deja_utilise` | numéro de carte déjà pris (toutes années confondues) |
| 409 | `reference_transaction_deja_utilisee` | référence mobile money déjà enregistrée |
| 422 | `donnees_invalides` | champ vide, trop long, caractère de contrôle, référence manquante ou en trop, date déclarée future ; le message nomme le champ |
| 429 | `trop_de_demandes` | cet agent a eu **5 adhésions sans patient** (`patient_inconnu`) dans les **15 dernières minutes** ; en-tête `Retry-After` en secondes |

**Limite d'échecs** (ajoutée le 2026-11-16) : chaque `patient_inconnu` est enregistré avec l'affectation de l'agent (ni le NIP ni la date essayés ne sont gardés). Au 5ᵉ dans une fenêtre glissante de 15 minutes, **toute** adhésion de cet agent reçoit 429, même avec un NIP et une date corrects, jusqu'à ce que le plus ancien échec sorte de la fenêtre (`Retry-After`). Une adhésion réussie ne remet pas le compte à zéro. Un NIP mal formé (`nip_invalide`) ne compte pas. Les autres agents ne sont pas touchés. L'application affiche : « Trop d'essais sans résultat. Réessayez dans N minutes. »

#### `GET /api/assurance-munaseb/contrats?numero_carte=` et `GET /api/assurance-munaseb/contrats/{id}`

La seule recherche de l'agent est le **numéro de carte** (de n'importe quelle année) : il n'existe pas de recherche par NIP.

**Téléphone de l'adhérent** (décision du porteur du 2026-11-17, qui remplace « l'agent ne voit jamais le téléphone ») : `GET /api/assurance-munaseb/contrats/{id}` ajoute le champ `"telephone": "+226…"`, pour enregistrer sans erreur d'identité et pouvoir rappeler l'adhérent. **Chaque affichage est écrit dans le journal des accès**, que le patient lit (section 2.18) ; si l'écriture échoue, la réponse est 500 et rien n'est montré. La recherche par carte, l'adhésion et le renouvellement ne renvoient **pas** ce champ (il est absent). Le NIP n'est jamais renvoyé.

- **Réponse 200** :

```json
{
  "id": "…",
  "patient_nom": "OUEDRAOGO",
  "patient_prenom": "Awa",
  "ufr": "UFR SEA",
  "universite": "Université Joseph Ki-Zerbo",
  "num_matricule": "MAT-0001",
  "personne_a_prevenir": null,
  "statut": "actif",
  "etat": "en_carence",
  "date_creation": "2026-11-16T09:30:00Z",
  "periodes": [
    {
      "id": "…",
      "numero_carte": "0552/2026",
      "date_paiement": "2026-11-16",
      "debut": "2026-12-16",
      "fin": "2027-11-16",
      "antenne": null,
      "niveau_etudes": "Licence 2",
      "en_cours": false,
      "plafond_fcfa": 100000,
      "consomme_fcfa": 0,
      "solde_fcfa": 100000,
      "paiement": {
        "mode": "mobile_money",
        "montant_cotisation_fcfa": 5000,
        "montant_fiche_fcfa": 100,
        "reference_transaction": "OM-123456",
        "date_declaree": "2026-11-16",
        "date_enregistrement": "2026-11-16T09:30:00Z",
        "agent_nom": "KABRE",
        "agent_prenom": "Inoussa"
      }
    }
  ]
}
```

- `etat` : `couvert`, `en_carence`, `expire` ou `suspendu`, comme pour le patient (section 2.13).
- `periodes` : la plus récente d'abord. Le solde est **indicatif**, recalculé à chaque validation. `paiement` vaut `null` et `numero_carte` peut valoir `null` pour une période enregistrée avant l'étape 5.
- **Jamais renvoyés** : le NIP et le téléphone du patient.
- 404 `contrat_inconnu` : aucun contrat pour ce numéro ou cet identifiant. 400 `parametre_invalide` sans `numero_carte` ; 400 `chemin_invalide`.

#### `POST /api/assurance-munaseb/contrats/{id}/renouvellements`

- **Corps** : `{"numero_carte": "0552/2027", "niveau_etudes": null, "antenne": null, "paiement": { … }}`, mêmes règles que l'adhésion. La nouvelle carte doit être différente de toutes les cartes déjà enregistrées.
- **Réponses** : 201 et le détail du contrat ; 404 `contrat_inconnu` ; 409 `periode_future_deja_payee` (carence en cours, ou période suivante déjà payée) ; 409 `contrat_suspendu` (règles de suspension inconnues : le renouvellement reste bloqué) ; 409 `numero_carte_deja_utilise` ; 409 `reference_transaction_deja_utilisee` ; 422 `donnees_invalides`.

### 2.15 Pharmacie : catalogue, stock, ventes (2026-11-16)

Pour une **officine** validée et sous licence (type `pharmacie`, section 2.8). Jeton `laaficare:professionnel_en_exercice` sur une affectation de cette pharmacie :

- **pharmacien ou responsable** : toutes les routes ci-dessous, sauf deux ;
- **responsable seulement** : `PUT /api/pharmacie/prix/{medicament_id}` et `PUT /api/pharmacie/reglages`. Un pharmacien reçoit **403 `acces_refuse`** ;
- **auxiliaire de pharmacie** (`auxiliaire_pharmacie`, 2026-11-17) : les lectures (catalogue, équivalents, stock, lots, journal, ventes, alertes) et les écritures **entrées, ventes, pertes et inventaires**. Il reçoit **403 `acces_refuse`** sur les annulations, les seuils, les prix, les réglages, les dépenses et le rapport. **Il ne reçoit jamais de prix d'achat** : les champs `prix_achat_unitaire_fcfa` et `valeur_achat_fcfa` sont **absents** de toutes ses réponses (le serveur les retire). Il saisit le prix d'achat d'une entrée (lu sur la facture du fournisseur), mais la réponse ne le reprend pas.

Tout autre rôle, une autre sorte de structure ou une pharmacie non validée : 403 `acces_refuse`.

**La pharmacie est toujours celle de l'affectation du jeton.** Aucune route ne prend l'identifiant d'une pharmacie. Un lot, une vente ou un mouvement d'une autre pharmacie répond **404**, exactement comme un objet qui n'existe pas.

**Ce qui n'existe pas encore** : ordonnance numérique, dossier du patient, alerte d'allergie (module médecin, plus tard, D2) ; aucune donnée patient n'est enregistrée avec une vente (D10).

#### Règles du stock

- **Le stock est un journal.** Chaque entrée crée un **lot** (numéro, date d'expiration, prix d'achat, fournisseur). Chaque entrée, vente, perte, écart d'inventaire ou annulation est un **mouvement**, jamais modifié ni effacé. Une erreur se corrige par une **annulation**, qui écrit le mouvement inverse.
- **Lot périmé** : un lot se vend jusqu'à sa date d'expiration **comprise** (date du Burkina). Le lendemain, il ne se vend plus et ne compte plus dans le stock disponible. Il reste en stock jusqu'à sa sortie en perte (motif `peremption`). Un lot déjà périmé ne peut pas entrer.
- **FEFO** : une vente sort d'abord le lot qui périme le plus tôt, sauf si le pharmacien choisit un lot (`lot_id`).
- **Prix figés** : chaque mouvement garde le prix d'achat de son lot ; chaque ligne de vente garde aussi son prix de vente. Un nouveau prix ne change jamais une vente passée.
- **Sans prix de vente fixé par le responsable, un médicament ne se vend pas** (409 `prix_non_defini`).
- Garde-fous techniques : 1 à 1 000 000 unités par ligne, prix de 0 à 100 000 000 FCFA, 100 lignes par vente ou par inventaire.

#### Clé d'idempotence (coupure de réseau)

Chaque création (`entrees`, `ventes`, `ventes/{id}/annuler`, `pertes`, `inventaires`, `mouvements/{id}/annuler`) accepte l'en-tête facultatif :

```
Idempotency-Key: 8e03978e-40d5-43e8-bc93-6894a57f9324
```

Source : draft IETF *The Idempotency-Key HTTP Header Field*.

- L'application crée **une clé par opération** (un UUID par exemple), **avant** le premier envoi, et la garde jusqu'à la réponse.
- Si la réponse se perd, elle renvoie **la même requête avec la même clé** : le serveur renvoie l'objet déjà créé, **sans rien créer une seconde fois**.
- La même clé avec une **autre** requête : **422 `cle_idempotence_reutilisee`**.
- Une clé de plus de 255 caractères, vide, avec une espace ou un caractère non ASCII, ou deux en-têtes : **400 `cle_idempotence_invalide`**.
- Une clé vaut pour **la pharmacie**, sans limite de durée.

#### Catalogue

- `GET /api/pharmacie/medicaments?q=&limite=` : recherche par dénomination ou par DCI, sans tenir compte des majuscules. `q` : 2 à 100 caractères, sinon 400 `parametre_invalide`. Réponse : `{"medicaments": [{"id", "denomination", "forme", "dosage", "dci": ["…"]}]}`.
- `GET /api/pharmacie/medicaments/{id}/equivalents` : médicaments qui ont **exactement les mêmes DCI**, avec le stock disponible et le prix de **cette** pharmacie. Réponse : `{"medicament": {…}, "equivalents": [{…, "correspondance", "meme_dosage", "meme_forme", "quantite_disponible", "prix_vente_unitaire_fcfa"}]}`. 404 `medicament_inconnu`.
  - **Classement, pour la sécurité du patient** : d'abord `"correspondance": "meme_dci_meme_dosage_meme_forme"` ; ensuite `"correspondance": "meme_dci_dosage_ou_forme_different"`, avec `meme_dosage` et `meme_forme` qui disent lequel diffère. Dans chaque groupe, ceux en stock d'abord, puis par nom.
  - **Rien n'est caché** : un équivalent de dosage ou de forme différent reste dans la liste, même en stock ; **le pharmacien décide**. L'application doit l'afficher comme tel (par exemple « Dosage différent : 1 g au lieu de 500 mg »), avec un texte, pas seulement une couleur.
  - Le dosage et la forme sont comparés sans tenir compte des majuscules ni des espaces en trop ; « 500 mg » et « 500mg » restent **différents** (sens prudent : signalé, jamais confondu).
  - Une association (plusieurs DCI) n'est jamais l'équivalent d'une molécule seule, et inversement.

Le catalogue est **national et unique**. Il n'est rempli que par un outil local de l'équipe LaafiCare (`importer_catalogue`), jamais par une route. **Il reste vide tant que la liste officielle n'est pas déposée** (D6).

#### Lectures du stock

- `GET /api/pharmacie/stock?limite=&apres=` : un élément par médicament qui a eu au moins un lot : le médicament, `quantite_disponible` (lots non périmés), `quantite_perimee`, `valeur_achat_fcfa` (tout le stock au prix d'achat), `seuil_min`, `seuil_max`, `prix_vente_unitaire_fcfa` (ou `null`). Pages : `{"medicaments": […], "suivant": "<id ou null>"}`, à passer en `apres`.
- `GET /api/pharmacie/stock/{medicament_id}` : `{"medicament_id", "lots": [{"id", "numero_lot", "date_expiration", "mois_expiration_imprime", "prix_achat_unitaire_fcfa", "fournisseur", "quantite", "perime", "date_creation"}]}`, lots encore en stock, **dans l'ordre FEFO**. `mois_expiration_imprime` (`AAAA-MM` ou `null`) : le mois imprimé sur une boîte qui n'indiquait que le mois (voir « Entrée d'une boîte au mois seul »).
- `GET /api/pharmacie/mouvements?limite=&avant=` : journal, le plus récent d'abord : `{"mouvements": [{"id", "nature", "lot_id", "medicament_id", "denomination", "numero_lot", "quantite", "prix_achat_unitaire_fcfa", "prix_vente_unitaire_fcfa", "vente_id", "inventaire_id", "motif_perte", "commentaire", "annule_mouvement_id", "annule_par", "date_mouvement", "auteur_nom", "auteur_prenom"}], "suivant": …}`. `nature` : `entree`, `vente`, `perte`, `inventaire`, `annulation`. `quantite` est positive pour ce qui entre, négative pour ce qui sort.
- `GET /api/pharmacie/ventes/{id}` : `{"id", "sur_ordonnance_papier", "date_vente", "auteur_nom", "auteur_prenom", "lignes": [mouvements], "total_fcfa", "annulee"}`. `total_fcfa` ne compte que les lignes non annulées. 404 `vente_inconnue`.
- `limite` : 1 à 100 (50 par défaut), sinon 400 `parametre_invalide`.

#### Écritures

| Route | Corps | Réponse |
|---|---|---|
| `POST /api/pharmacie/entrees` | `{"medicament_id", "numero_lot", "date_expiration": "2027-06-15", "quantite", "prix_achat_unitaire_fcfa", "fournisseur"}`, ou `"mois_expiration": "2027-06"` à la place de `date_expiration` (exactement l'un des deux) | 201, le lot |
| `POST /api/pharmacie/ventes` | `{"sur_ordonnance_papier": false, "lignes": [{"medicament_id", "quantite", "lot_id": null}]}` | 201, la vente |
| `POST /api/pharmacie/ventes/{id}/annuler` | `{"motif": "…"}` | 200, la vente annulée (toute la vente ; le stock revient dans ses lots) |
| `POST /api/pharmacie/pertes` | `{"lot_id", "quantite", "motif": "peremption" \| "casse" \| "autre", "commentaire": null}` (commentaire obligatoire pour `autre`) | 201, le mouvement |
| `POST /api/pharmacie/inventaires` | `{"lignes": [{"lot_id", "quantite_comptee"}]}` | 201, `{"inventaire_id", "ecarts": [mouvements]}` (un mouvement par lot dont le compte diffère du stock) |
| `POST /api/pharmacie/mouvements/{id}/annuler` | `{"motif": "…"}` | 201, l'écriture inverse (une entrée, une perte ou un écart d'inventaire) |
| `PUT /api/pharmacie/prix/{medicament_id}` (responsable) | `{"prix_unitaire_fcfa": 250}` | 200, `{"medicament_id", "prix_unitaire_fcfa", "date_effet"}` |
| `PUT /api/pharmacie/seuils/{medicament_id}` | `{"seuil_min": 20, "seuil_max": null}` | 204 |

**Entrée d'une boîte au mois seul** (« EXP 06/2027 ») : envoyer `"mois_expiration": "2027-06"`. Aucune source officielle burkinabè ni de l'UEMOA n'a été trouvée sur la convention ; la règle retenue est **la plus prudente pour le patient** : le lot est **périmé dès le premier jour du mois imprimé**. Le serveur enregistre donc `date_expiration` = **dernier jour du mois précédent** (`2027-06` donne `2027-05-31`) et rend `"mois_expiration_imprime": "2027-06"`. Une boîte du mois en cours est déjà périmée : 422 `donnees_invalides`. L'application doit afficher les deux : « Boîte : 06/2027, ne plus vendre à partir du 01/06/2027 ».
| `PUT /api/pharmacie/reglages` (responsable) | `{"delai_alerte_peremption_jours": 90}` (1 à 365, `null` désactive) | 200, les réglages |

Refus :

- 404 `medicament_inconnu`, `lot_inconnu`, `vente_inconnue`, `mouvement_inconnu` (inexistant ou d'une autre pharmacie) ;
- 409 `stock_insuffisant`, avec `medicament_id` (si connu) et `quantite_disponible` ;
- 409 `prix_non_defini`, avec `medicament_id` ;
- 409 `lot_perime` (lot choisi périmé) ;
- 409 `deja_annule` ; 409 `annulation_impossible` (une vente s'annule par sa propre route ; une annulation ne s'annule pas) ;
- 409 `stock_insuffisant` aussi pour l'annulation d'une entrée dont une partie est déjà sortie ;
- 422 `donnees_invalides` (le message nomme le champ) ; 400 et 422 pour la clé d'idempotence (ci-dessus).

#### Alertes (dans l'application, jamais par SMS, D11)

`GET /api/pharmacie/alertes`, calculées à chaque lecture sur le stock réel :

```json
{
  "sous_seuil_min": [{"medicament_id", "denomination", "quantite_disponible", "seuil_min", "seuil_max"}],
  "au_dessus_seuil_max": […],
  "alerte_peremption_active": false,
  "delai_alerte_peremption_jours": null,
  "bientot_perimes": [{"lot_id", "medicament_id", "denomination", "numero_lot", "date_expiration", "quantite", "jours_restants"}],
  "perimes_en_stock": […]
}
```

- `bientot_perimes` reste **vide tant que le responsable n'a pas réglé le délai** (D9 : aucune valeur par défaut).
- `perimes_en_stock` : lots périmés encore en stock, à sortir en perte ; toujours affichés.

### 2.16 Pharmacie : dépenses et bénéfice net (2026-11-16)

> **Ce n'est pas la comptabilité officielle (SYSCOHADA).** C'est un tableau de gestion pour le responsable de la pharmacie, en FCFA entiers. **Aucun calcul d'impôt** : les impôts et taxes sont une dépense saisie (catégorie `impots_taxes`).

**Responsable de la pharmacie seulement** (jeton en exercice, affectation `responsable` d'une pharmacie). Un pharmacien reçoit **403 `acces_refuse`**. Une dépense d'une autre pharmacie répond 404, comme une dépense qui n'existe pas.

#### `POST /api/pharmacie/depenses`

- **Corps** : `{"date_depense": "2026-11-16", "categorie": "loyer", "montant_fcfa": 50000, "libelle": "Loyer de novembre"}`.
- `categorie` : `loyer`, `salaires`, `electricite`, `eau`, `transport`, `impots_taxes`, `entretien`, `autres`.
- `date_depense` : jamais dans le futur (date du Burkina). `montant_fcfa` : entier positif. `libelle` : 1 à 200 caractères.
- En-tête `Idempotency-Key` facultatif (section 2.15).
- **Réponses** : 201 et la dépense `{"id", "date_depense", "categorie", "montant_fcfa", "libelle", "annule_depense_id", "motif_annulation", "annulee_par", "date_creation", "auteur_nom", "auteur_prenom"}` ; 422 `donnees_invalides`.

**Une dépense ne se modifie ni ne s'efface jamais.** Une erreur se corrige par une annulation.

#### `POST /api/pharmacie/depenses/{id}/annuler`

- **Corps** : `{"motif": "Saisie en double"}` (obligatoire, 1 000 caractères au plus). En-tête `Idempotency-Key` facultatif.
- Crée une écriture au **montant opposé**, avec la **même date** et la **même catégorie** : la dépense disparaît du rapport de **sa** période (règle commune des annulations, ci-dessous).
- **Réponses** : 201 et l'annulation ; 404 `depense_inconnue` ; 409 `deja_annule` ; 409 `annulation_impossible` (une annulation ne s'annule pas).

#### `GET /api/pharmacie/depenses?limite=&avant=`

Le journal des dépenses, la plus récemment saisie d'abord, annulations comprises : `{"depenses": […], "suivant": …}`.

#### `GET /api/pharmacie/rapport`

Deux façons de choisir la période (dates du Burkina, comprises) :

- `?periode=semaine` (du lundi au dimanche), `?periode=mois` ou `?periode=annee`, avec `&date=AAAA-MM-JJ` facultatif (la période qui contient cette date ; aujourd'hui par défaut) ;
- ou `?du=AAAA-MM-JJ&au=AAAA-MM-JJ`.

Autre chose (rien, les deux à la fois, `au` avant `du`, plus de 3 660 jours) : 422 `donnees_invalides`.

```json
{
  "du": "2026-11-16", "au": "2026-11-22",
  "ventes_fcfa": 600,
  "cout_achat_vendu_fcfa": 300,
  "marge_brute_fcfa": 300,
  "pertes": {"peremption_fcfa": 200, "casse_fcfa": 100, "autre_fcfa": 0, "ecarts_inventaire_fcfa": -20, "total_fcfa": 280},
  "depenses": {"par_categorie": [{"categorie": "loyer", "montant_fcfa": 50000}, …], "total_fcfa": 50000},
  "benefice_net_fcfa": -49980,
  "valeur_stock_fin_fcfa": 1320,
  "dont_perime_fcfa": 100
}
```

Comment chaque nombre est calculé :

- **ventes** : les lignes de vente au prix de vente figé, moins les ventes annulées ;
- **coût d'achat des produits vendus** : les mêmes lignes au prix d'achat de leur lot ;
- **marge brute** = ventes − coût d'achat des produits vendus ;
- **pertes**, au prix d'achat du lot : péremption, casse, autre, et **écarts d'inventaire** (un manque est une perte ; un excédent la diminue, d'où un nombre qui peut être négatif) ;
- **dépenses** : le journal, par catégorie (les huit, même à 0) ;
- **bénéfice net** = marge brute − pertes − dépenses ;
- **valeur du stock restant** à la fin du dernier jour, au prix d'achat, **dont** les lots déjà périmés à cette date.

À savoir :

- **annulations, une seule règle** : toute annulation (vente, perte, écart d'inventaire, entrée, dépense) **corrige la période du mouvement d'origine**. Le journal ne change jamais : l'annulation y garde sa propre date ; mais le rapport la compte à la date du mouvement annulé. Exemple : une vente du 12 octobre annulée le 3 novembre disparaît des ventes d'octobre (et le produit revient dans le stock de fin octobre) ; novembre n'en garde aucune trace. **Le rapport d'une période passée peut donc changer** après une annulation ;
- les **achats** (entrées en stock) ne sont pas des dépenses : leur coût compte quand le produit est vendu ou perdu ; le reste est dans la valeur du stock.

---

### 2.17 Profil professionnel : numéro d'ordre et spécialité (2026-11-17)

Le numéro d'ordre et la spécialité appartiennent à la **personne**, pas à l'affectation : un médecin les garde d'une structure à l'autre. Le numéro est en **texte libre** (aucune source n'en donne le format) ; l'**équipe LaafiCare le contrôle** et le marque vérifié. **Tout changement annule la vérification.** La vérification n'est exigée nulle part pour l'instant (question dans `docs/points-ouverts.md`).

Spécialités (liste courte, à valider par le porteur) : `medecine_generale`, `gynecologie_obstetrique`, `pediatrie`, `chirurgie`, `sage_femme` (la sage-femme est une spécialité d'infirmier).

#### `GET /api/professionnels/profil` et `PUT /api/professionnels/profil`

- **Appelant** : jeton `laaficare:professionnel_sans_affectation` ou `laaficare:professionnel_en_exercice`.
- **Corps du PUT** : `{"numero_ordre": "…" | null, "specialite": "pediatrie" | null}`. Un champ absent ou `null` est vidé. Numéro : 1 à 50 caractères, sans retour à la ligne (espaces autour retirés).
- **Réponse 200** (les deux) : `{"numero_ordre", "specialite", "verifie": false, "verifie_le": null}`.
- **Erreurs** : 422 `numero_ordre_invalide` ; 422 `donnees_invalides` (spécialité hors liste).

#### `GET /api/administrateurs/profils-professionnels?limite=&apres=`

Jeton administrateur. Profils avec un numéro, pas encore vérifiés : `{"profils": [{"compte_id", "nom", "prenom", "email", "numero_ordre", "specialite", "modifie_le"}], "suivant"}`.

#### `POST /api/administrateurs/profils-professionnels/{compte_id}/verifier`

Jeton administrateur. Corps : `{"numero_ordre": "…", "specialite": "…" | null}`, **les valeurs que l'équipe a lues et contrôlées**. 204 ; **409 `verification_impossible`** si le profil a changé depuis, n'a pas de numéro, est déjà vérifié ou n'existe pas.

### 2.18 Accès aux données d'un patient : scan du NIP et journal (2026-11-17)

**Règle de base** : le NIP seul, écrit ou scanné, **n'ouvre jamais le dossier**. Le scan renvoie seulement une **fiche d'identité**, dont les champs dépendent du **rôle de l'affectation en exercice** (loi n° 001-2021/AN : données adéquates et non excessives). Le filtrage est fait par le serveur : un champ que le rôle ne doit pas voir est **absent** de la réponse. Le dossier ne s'ouvre que par une prise en charge (code de soin ou CNIB vérifiée) ou un bris de glace (section 2.19).

**Journal des accès** : chaque scan, chaque lecture ou écriture du dossier et chaque affichage du téléphone est écrit **avant** que les données soient renvoyées ; si l'écriture échoue, rien n'est montré (500). Le journal n'est jamais modifié ni effacé, même par un administrateur. Le patient le lit.

#### `POST /api/professionnels/patients/scan`

- **Appelant** : jeton `laaficare:professionnel_en_exercice`.
- **Corps** : `{"nip": "1000000000000008"}` (dans le corps, jamais dans l'adresse ; espaces autour acceptées).
- **Réponse 200**, selon le rôle :

| Rôle (type d'établissement) | Champs |
|---|---|
| `agent_accueil` (clinique), `medecin`, `infirmier` (clinique) | `nom`, `prenom`, `sexe`, `date_naissance`, `lieu_naissance` |
| `agent_assurance_munaseb` (MUNASEB) | les mêmes, plus `telephone` |
| `pharmacien` (pharmacie) | les mêmes que l'accueil, plus `droits_munaseb` : `{"etat": "couvert"}` (`aucun_contrat`, `couvert`, `en_carence`, `expire`, `suspendu`) |
| `auxiliaire_pharmacie` (pharmacie) | `nom`, `prenom`, `droits_munaseb` |

`sexe` vaut `null` si le patient ne l'a pas donné. Le NIP n'est jamais renvoyé.

- **Erreurs** :

| Code | `code` | Cas |
|---|---|---|
| 403 | `acces_refuse` | tout autre rôle (responsable compris), ou un rôle hors de son type d'établissement |
| 404 | `patient_inconnu` | NIP inconnu **ou** mal formé (jamais distingués) |
| 429 | `trop_de_demandes` | 5 essais sans résultat en 15 minutes pour cette affectation, ou 60 scans réussis en 15 minutes ; en-tête `Retry-After` (secondes) |

#### `GET /api/patients/journal-acces?limite=&avant=`

- **Appelant** : jeton `laaficare:patient` ; le patient est celui du jeton.
- **Réponse 200**, le plus récent d'abord : `{"acces": [{"id", "date_acces", "nature", "structure_nom", "role", "professionnel_nom", "professionnel_prenom", "telephone_affiche", "moyen", "motif_categorie", "motif"}], "suivant"}`.
- `nature` : `scan`, `detail_contrat`, `recherche_identite`, `ouverture_prise_en_charge`, `prolongation_prise_en_charge`, `bris_de_glace`, `lecture_dossier`, `ecriture_dossier`.
- `moyen`, `motif_categorie`, `motif` : pour un accès sous une prise en charge (section 2.19), sinon `null`.
- Pagination : `suivant` dans `avant` ; `limite` 1 à 100 (50 par défaut).

### 2.19 Prise en charge, code de soin, bris de glace (2026-11-17)

**Modèle d'accès (décision MD1 du porteur)** : le dossier ne s'ouvre que sous une **prise en charge** de l'établissement de l'affectation, encore en cours. Elle est ouverte :
- avec le **code de soin** du patient (moyen préféré : il prouve que le patient est là) ;
- ou après **vérification de sa CNIB** par le professionnel (attestation tracée, le numéro n'est jamais enregistré) ;
- ou par un **bris de glace** en urgence (médecin ou infirmier seulement, motif obligatoire, contrôlé ensuite par le responsable, patient averti).

Une prise en charge dure **24 heures** ; un médecin ou un infirmier la **prolonge** de 24 heures pendant une hospitalisation. Elle ne vaut que pour **son établissement**. Rien ne se modifie ni ne s'efface. Chaque ouverture, prolongation et bris de glace est écrit dans le journal des accès (section 2.18).

Objet rendu : `{"id", "moyen": "code_soin" | "cnib_verifiee" | "bris_de_glace", "debut", "fin"}` (`fin` : fin effective, prolongations comprises). L'`id` sert aux routes du dossier (section 2.20).

#### `POST /api/patients/code-soin` (jeton patient)

Crée ou remplace le secret du **code de soin** : un code à 6 chiffres qui change toutes les 30 secondes (TOTP, RFC 6238), que le téléphone calcule **sans réseau**.

- **Corps** : `{"mot_de_passe": "…"}` (exigé, compté dans le verrouillage).
- **Réponse 201** (en-tête `Cache-Control: no-store`) : `{"cle_base32", "url_otpauth", "algorithme": "SHA1", "chiffres": 6, "periode_secondes": 30}`. **Montrée une seule fois** : l'application la garde dans le stockage sécurisé du téléphone. Une nouvelle demande remplace l'ancien secret.
- **Erreurs** : 401 `identifiants_invalides` (mot de passe faux, compte verrouillé).

#### `POST /api/professionnels/patients/recherche` (accueil, médecin, infirmier d'une clinique)

Patient sans téléphone ni carte (décision MD19). **Corps** : `{"nom", "prenom", "date_naissance": "1995-04-12", "lieu_naissance"}`, comparés **exactement** (seules les majuscules et les espaces autour sont ignorées). **Réponse 200** : la fiche de l'accueil (section 2.18) et `"nip"`. **Une seule personne, jamais une liste** : aucun résultat ou plusieurs (homonymes) → 404 `patient_inconnu`. 5 échecs en 15 minutes par affectation, puis 429. Chaque recherche réussie est au journal. Autre rôle : 403.

#### `POST /api/prises-en-charge` (accueil, médecin, infirmier d'une clinique ; pharmacien d'une pharmacie)

- **Corps** : `{"nip", "code_soin": "123456"}` **ou** `{"nip", "cnib_verifiee": true}` (exactement l'un des deux, sinon 422 `donnees_invalides`).
- **Réponse 201** : la prise en charge.
- **Erreurs** : 403 `acces_refuse` (auxiliaire, agent MUNASEB, responsable…) ; 404 `patient_inconnu` ; 422 `code_soin_invalide` (code faux, déjà utilisé, ou patient sans code de soin) ; 429 `trop_de_demandes` (5 essais sans résultat par affectation en 15 minutes ; **5 codes faux par patient** en 15 minutes, tous établissements confondus ; 60 ouvertures réussies par affectation en 15 minutes). Le bris de glace a la même limite de 60.

#### `POST /api/prises-en-charge/bris-de-glace` (médecin ou infirmier d'une clinique)

- **Corps** : `{"nip", "motif_categorie": "patient_inconscient" | "urgence_vitale" | "patient_incapable_de_communiquer" | "autre", "motif": "…"}` (motif obligatoire, 1 000 caractères au plus).
- **Réponse 201** : la prise en charge (`moyen` : `bris_de_glace`).
- Le patient reçoit une **notification** (`type` `acces_urgence`, texte sans établissement ni motif : « Votre dossier médical a été ouvert en urgence. Consultez le journal des accès. »).
- **Erreurs** : 403 `acces_refuse` (pharmacien, accueil, tout autre rôle) ; 404 `patient_inconnu` ; 422 `donnees_invalides` (motif vide ou trop long, catégorie inconnue).

#### `POST /api/prises-en-charge/{id}/prolonger` (médecin ou infirmier de l'établissement)

24 heures de plus, à partir de la fin effective. **Réponse 200** : la prise en charge. **Erreurs** : 403 `acces_refuse` ; 403 `prise_en_charge_expiree` ; 404 `prise_en_charge_inconnue` (inexistante ou d'un autre établissement).

#### `GET /api/structures/bris-de-glace?a_controler=true&limite=&avant=` (responsable d'une clinique)

`{"bris_de_glace": [{"id", "debut", "role", "professionnel_nom", "professionnel_prenom", "motif_categorie", "motif", "patient_nom", "patient_prenom", "conclusion", "commentaire", "date_controle"}], "suivant"}`. `a_controler=true` : seulement ceux pas encore contrôlés.

#### `POST /api/structures/bris-de-glace/{id}/controler` (responsable d'une clinique)

**Corps** : `{"conclusion": "justifie" | "injustifie", "commentaire": "…" | null}`. **Réponse 204**. **Erreurs** : 404 `prise_en_charge_inconnue` (autre établissement, ou pas un bris de glace) ; 409 `deja_controle` (un seul contrôle).

### 2.20 Dossier médical (2026-11-17)

Toutes ces routes passent par une **prise en charge en cours de l'établissement de l'affectation** (`{id}`, section 2.19), **jamais par le NIP**. Chaque lecture et chaque écriture est écrite dans le journal des accès avant la réponse. Erreurs communes : 403 `acces_refuse` (rôle sans droit), 404 `prise_en_charge_inconnue` (inexistante ou d'un autre établissement), 403 `prise_en_charge_expiree`, 422 `donnees_invalides` (le message nomme le champ). **Rien ne se modifie ni ne s'efface** : une allergie se retire, une consultation se corrige par un ajout.

#### `GET /api/prises-en-charge/{id}/dossier`

La réponse dépend du rôle ; un champ que le rôle ne doit pas voir est **absent**. Les **allergies viennent en premier** : l'application les affiche en priorité, en rouge, avec un texte (règle du modèle et WCAG 2.2, 1.4.1).

- **Médecin, infirmier** (clinique) : `{"allergies", "patient", "allergies_retirees", "antecedents", "consultations", "ordonnances"}`.
- **Pharmacien** (pharmacie, sous une prise en charge ouverte par la pharmacie) : `{"allergies", "patient", "antecedents_medicamenteux", "ordonnances"}`. **Jamais** les consultations, les diagnostics, les examens ni le lien d'une ordonnance vers sa consultation.
- **Accueil, auxiliaire, agent MUNASEB, responsable** : 403 `acces_refuse`.

Contenu :
- `allergies` : `[{"id", "substance", "dci_id", "dci_nom", "reaction", "date_saisie"}]` (actives) ; `allergies_retirees` : les mêmes, avec `motif_retrait` et `date_retrait` ;
- `patient` : `{"nom", "prenom", "sexe", "date_naissance", "lieu_naissance"}` ;
- `antecedents` : `[{"id", "type_antecedent": "medical" | "chirurgical" | "familial" | "medicamenteux", "description", "date_debut", "date_saisie"}]` ;
- `consultations` : `[{"id", "motif", "examen", "diagnostic", "date_consultation", "structure_nom", "medecin_nom", "medecin_prenom", "ajouts": [{"texte", "date_ajout", "medecin_nom", "medecin_prenom"}]}]` ;
- `ordonnances` : `[{"id", "consultation_id" (vue complète seulement), "date_emission", "structure_nom", "medecin_nom", "medecin_prenom", "lignes": [{"rang", "medicament_id", "denomination", "forme", "dosage", "hors_catalogue", "posologie", "quantite", "duree_jours", "alerte_allergie_depassee"}]}]`. **Aucune durée de validité** n'est imposée (en attente de l'Ordre) : la date est toujours rendue.

#### Écritures

| Route | Qui | Corps | Réponse |
|---|---|---|---|
| `POST /api/prises-en-charge/{id}/allergies` | médecin, infirmier | `{"substance", "dci_id": "…" \| null, "reaction": "…" \| null}` (`dci_id` : une DCI du catalogue, pour le contrôle des ordonnances) | 201, l'allergie |
| `POST /api/prises-en-charge/{id}/allergies/{allergie_id}/retirer` | médecin | `{"motif"}` | 204 ; 404 `allergie_inconnue` ; 409 `deja_retiree` |
| `POST /api/prises-en-charge/{id}/antecedents` | médecin, infirmier | `{"type_antecedent", "description", "date_debut": null}` | 201, l'antécédent |
| `POST /api/prises-en-charge/{id}/consultations` | médecin | `{"motif", "examen": null, "diagnostic": null}` (texte libre) | 201, `{"id", "date_consultation"}` |
| `POST /api/prises-en-charge/{id}/consultations/{consultation_id}/ajouts` | médecin | `{"texte"}` | 204 ; 404 `consultation_inconnue` |
| `POST /api/prises-en-charge/{id}/ordonnances` | médecin | voir ci-dessous | 201 |

#### Ordonnance et contrôle d'allergie

- **Corps** : `{"consultation_id": null, "lignes": [{"medicament_id": "…", "posologie": "1 comprimé matin et soir", "quantite": 10, "duree_jours": 5}], "depassement_allergie_motif": null}`. Chaque ligne a **soit** `medicament_id` (catalogue national), **soit** `denomination` (hors catalogue, permis, sans contrôle possible). 1 à 50 lignes ; quantité 1 à 10 000 ; durée 1 à 365 jours ou `null`.
- **Alerte bloquante** (CDC, BF-03) : si une DCI d'un médicament prescrit est celle d'une **allergie active** du patient, la réponse est **409 `alerte_allergie`**, rien n'est créé, avec `"conflits": [{"rang", "medicament_id", "allergie_id", "substance"}]`. Pour prescrire quand même, le médecin renvoie la même ordonnance avec `"depassement_allergie_motif"` : chaque dépassement est tracé, et la ligne porte `"alerte_allergie_depassee": true`.
- **Réponse 201** : l'ordonnance, plus `"avertissements": {"lignes_hors_catalogue": [2], "allergies_non_comparees": ["Arachide"]}` : ce qui n'a pas pu être comparé (lignes hors catalogue, allergies sans DCI). L'application doit les afficher.
- **Erreurs** : 404 `medicament_inconnu`, 404 `consultation_inconnue` (d'un autre patient), 409 `alerte_allergie`.

## 3. Codes d'erreur : index

| `code` | HTTP | Routes |
|---|---|---|
| `authentification_requise` | 401 | routes protégées |
| `jeton_invalide` | 401 | routes protégées |
| `acces_refuse` | 403 | routes protégées (routes MUNASEB : pas agent MUNASEB, affectation désactivée, établissement suspendu, licence expirée ; invitations, membres et modification des paramètres MUNASEB : affectation qui n'est pas `responsable` ; pharmacie : pas pharmacien ni responsable d'une pharmacie, et prix, réglages, dépenses et rapports pour un pharmacien) |
| `affectation_non_exercable` | 403 | activation d'une affectation |
| `affectation_inconnue` | 404 | activation et désactivation d'une affectation |
| `dernier_responsable` | 409 | désactivation d'une affectation |
| `json_invalide`, `donnees_invalides`, `content_type_invalide`, `corps_illisible` | 400, 422, 415, 413 | routes avec corps (413 : plus de 256 Kio) |
| `chemin_invalide` | 400 | routes avec `{id}` |
| `parametre_invalide` | 400 | listes paginées (remboursements, structures, invitations, demandes et notifications du patient) |
| `telephone_invalide` | 422 | routes patient, création d'une invitation |
| `telephone_service_invalide`, `telephone_urgence_invalide` | 422 | création de structure |
| `code_invalide` | 401 | inscription, réinitialisation, second facteur (professionnel, administrateur), confirmation du réenrôlement |
| `trop_de_demandes` | 429 | demande de code, renvoi du lien de vérification et adhésion à la MUNASEB (avec `Retry-After`), inscription, réinitialisation et création d'une invitation (sans) |
| `telephone_deja_utilise` | 409 | inscription |
| `telephone_inconnu` | 404 | réinitialisation |
| `mot_de_passe_non_conforme` | 422 | inscription, réinitialisation (patient, professionnel, administrateur), création de structure, acceptation d'une invitation |
| `mot_de_passe_identique_patient` | 422 | réinitialisation du mot de passe d'un administrateur |
| `identifiants_invalides` | 401 | connexions, début du réenrôlement administrateur, renvoi du lien, création de structure (mot de passe professionnel) |
| `email_non_verifie` | 403 | connexions professionnelle et administrateur, soumission d'une structure |
| `lien_invalide` | 400 | vérification de l'email, réinitialisation du mot de passe |
| `type_non_disponible`, `localisation_hors_burkina`, `nom_trop_long`, `nom_invalide` | 422 | création de structure |
| `demande_deja_en_cours`, `email_different`, `email_deja_utilise`, `second_facteur_requis` | 409 | création de structure |
| `structure_inconnue` | 404 | soumission et abandon d'une structure, pièces justificatives, routes administrateur des structures |
| `pieces_manquantes` | 409 | soumission d'une structure (avec `pieces_manquantes`) |
| `piece_non_prevue`, `fichier_vide` | 422 | dépôt d'une pièce |
| `fichier_trop_grand` | 413 | dépôt d'une pièce |
| `format_refuse` | 415 | dépôt d'une pièce |
| `piece_inconnue` | 404 | téléchargement d'une pièce |
| `decision_sur_sa_propre_structure` | 403 | validation, refus d'une structure |
| `email_responsable_non_verifie`, `munaseb_deja_validee`, `autorisation_deja_validee` | 409 | validation d'une structure |
| `licence_invalide` | 422 | validation d'une structure |
| `role_non_permis`, `auto_invitation` | 422 | création d'une invitation |
| `invitation_deja_en_attente`, `trop_d_invitations_en_attente`, `structure_non_validee` | 409 | invitations |
| `invitation_inconnue` | 404 | invitations |
| `invitation_expiree`, `affectation_deja_active`, `requete_simultanee` | 409 | invitations |
| `montant_invalide`, `nip_invalide`, `partenaire_suspendu`, `non_couvert`, `en_carence` | 422 | simuler-acte |
| `patient_inconnu`, `partenaire_inconnu` | 404 | simuler-acte |
| `reference_acte_deja_utilisee` | 409 | simuler-acte |
| `nip_invalide` | 422 | adhésion à la MUNASEB (section 2.14) |
| `patient_inconnu` | 404 | adhésion à la MUNASEB (NIP inconnu ou date de naissance différente) |
| `contrat_inconnu` | 404 | détail, recherche et renouvellement d'un contrat |
| `contrat_deja_existant`, `numero_carte_deja_utilise`, `reference_transaction_deja_utilisee` | 409 | adhésion, renouvellement |
| `periode_future_deja_payee`, `contrat_suspendu` | 409 | renouvellement |
| `donnees_invalides` | 422 | aussi : adhésion, renouvellement, modification des paramètres (le message nomme le champ) |
| `demande_inconnue` | 404 | détail, transitions ; détail d'une demande du patient (inexistante ou d'un autre patient) |
| `notification_inconnue` | 404 | notification lue (inexistante ou d'une autre personne) |
| `transition_invalide` | 409 | transitions, soumission, abandon, validation et refus d'une structure, désactivation d'une affectation |
| `motif_vide`, `motif_trop_long` | 422 | rejet d'une demande de remboursement, refus d'une structure |
| `medicament_inconnu`, `lot_inconnu`, `vente_inconnue`, `mouvement_inconnu` | 404 | pharmacie (section 2.15 ; aussi pour un objet d'une autre pharmacie) |
| `stock_insuffisant`, `prix_non_defini`, `lot_perime`, `deja_annule`, `annulation_impossible` | 409 | pharmacie (avec `medicament_id`, et `quantite_disponible` pour le stock) ; `deja_annule` et `annulation_impossible` aussi pour les dépenses |
| `depense_inconnue` | 404 | annulation d'une dépense (section 2.16 ; aussi pour une dépense d'une autre pharmacie) |
| `cle_idempotence_invalide` | 400 | créations de la pharmacie (en-tête `Idempotency-Key`) |
| `cle_idempotence_reutilisee` | 422 | créations de la pharmacie (même clé, autre requête) |
| `numero_ordre_invalide` | 422 | profil professionnel (section 2.17) |
| `verification_impossible` | 409 | vérification d'un profil professionnel par l'équipe |
| `patient_inconnu` | 404 | aussi : scan du NIP (inconnu ou mal formé), section 2.18 |
| `trop_de_demandes` | 429 | aussi : scan du NIP, recherche par identité, ouverture d'une prise en charge (avec `Retry-After`) |
| `code_soin_invalide` | 422 | ouverture d'une prise en charge par le code de soin |
| `prise_en_charge_inconnue` | 404 | prolongation, contrôle d'un bris de glace, dossier (inexistante ou d'un autre établissement) |
| `prise_en_charge_expiree` | 403 | prolongation, dossier (prise en charge terminée) |
| `deja_controle` | 409 | contrôle d'un bris de glace |
| `alerte_allergie` | 409 | ordonnance (avec `conflits`), section 2.20 |
| `allergie_inconnue`, `consultation_inconnue` | 404 | dossier médical |
| `deja_retiree` | 409 | retrait d'une allergie |
| `medicament_inconnu` | 404 | aussi : ordonnance |
| `erreur_interne` | 500 | toutes |

---

## 4. Routes prévues (pas encore codées)

Rien de cette section n'existe dans le code. **Les chemins, corps et réponses ne sont pas encore fixés** : chacun sera présenté au porteur et validé à son étape, puis décrit en section 2. Seules les fonctions et les règles déjà décidées (CLAUDE.md, sections 12 et 14) sont listées ici, pour que les applications puissent préparer leurs écrans.

### 4.1 Authentification professionnelle et administrateur (étape 3.6)

- ✅ Connexion professionnelle : **codée**, section 2.4.
- ✅ Second temps de la connexion professionnelle : **codé**, section 2.4. La même forme vaudra pour le patient qui aura activé le TOTP (après l'étape 3.11).
- ✅ Connexion administrateur LaafiCare et réenrôlement : **codés**, section 2.5.

### 4.2 Second facteur TOTP

Le module existe déjà côté serveur ; les routes manquent.

- Activation : demande le mot de passe ; le serveur renvoie l'URL `otpauth://` et la clé en base 32 (l'application dessine le QR code) ; l'activation n'est effective qu'après un premier code correct ; 10 codes de secours `XXXXX-XXXXX` montrés une seule fois.
- Facultatif pour patients et professionnels, obligatoire pour les administrateurs.
- La personne ne désactive pas elle-même son TOTP en V1 (seule l'équipe LaafiCare, par un outil local).

### 4.3 Structures et pièces justificatives (étapes 3.7 et 3.8)

- ✅ Création, soumission et suivi, **type MUNASEB seulement** : **codés**, section 2.8 ; vérification de l'email, section 2.7.
- ✅ Abandon : **codé**, section 2.8.
- Restent : modification du brouillon, autres types de structure.
- Téléversement d'une pièce, **une requête par fichier** (réseau faible) : PDF, JPEG ou PNG, 5 Mo au plus.
- ✅ Côté administrateur : liste, détail (avertissements MUNASEB), validation (licence), refus (motif) : **codés**, section 2.9.
- Restent côté administrateur : avertissement des numéros d'autorisation en double, consultation et téléchargement des pièces, suspension et réactivation (après le pilote).

### 4.4 Affectations et rôle actif (étape 3.9)

- ✅ **Codé**, section 2.6 : liste des affectations, activation, changement de structure sans mot de passe ; routes MUNASEB rebranchées (section 2.3).

### 4.5 Invitations (étape 3.10)

- ✅ **Codées pour la MUNASEB**, section 2.10 : création, liste, annulation ; côté invité, liste, acceptation, refus ; expiration après 7 jours.
- Restent : les autres rôles et types de structure ; les invitations d'administrateurs (après le pilote).

### 4.6 Désactivation d'une affectation et bascule (étape 3.11)

- ✅ Désactivation d'une affectation par un responsable, **codée pour la MUNASEB**, section 2.12.
- Restent : un agent qui quitte lui-même une structure ; les autres rôles (chef de service, major) ; la bascule.
- Bascule professionnel → patient sans mot de passe ; patient → professionnel avec le mot de passe professionnel.

### 4.7 Contrat MUNASEB côté agent (étape 5)

- ✅ Adhésion, renouvellement, détail et recherche par carte, paramètres : **codés**, section 2.14.
- Restent, en attente des réponses de la MUNASEB (docs/points-ouverts.md) : suspension d'un contrat, ayants droit, tarifs par circuit.

### 4.8 Côté patient (étape 6)

- ✅ **Codé**, section 2.13 : droits MUNASEB, demandes de remboursement, notifications.
