# API LaafiCare : contrat entre le backend et les applications

Ce document décrit les routes de l'API Rust/Axum telles qu'elles sont **codées aujourd'hui**, puis celles qui sont **seulement prévues**. Les applications Angular (web) et Flutter (mobile) s'appuient sur lui. Il est mis à jour avec chaque ajout ou modification d'une route.

État au 2026-10-12, branche `dev`.

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
| `iat`, `exp` | date d'émission et d'expiration (secondes depuis 1970) |

Il existe **huit sortes de jetons**. Chaque route n'en accepte qu'une : un jeton d'une autre sorte est refusé en 401 `jeton_invalide`, même s'il est encore valable.

| Sorte (`aud`) | Sert à | Durée | État |
|---|---|---|---|
| `laaficare:patient` | routes du patient (`GET /api/patients/moi`) | 24 h | émis par l'inscription et la connexion patient ; refusé après une réinitialisation du mot de passe |
| `laaficare:professionnel_sans_affectation` | lister ses affectations, en activer une | 24 h | prévu (étape 3.6) |
| `laaficare:professionnel_en_exercice` | routes métier (remboursements MUNASEB…) | 24 h | attendu par les routes MUNASEB ; émis à l'étape 3.9 |
| `laaficare:administrateur` | routes de l'équipe LaafiCare | 24 h | prévu (étape 3.6) |
| `laaficare:second_facteur_patient`, `…_professionnel`, `…_administrateur` | saisir le code du second facteur, après un mot de passe correct | 5 min | prévu (étape 3.6) |
| `laaficare:reenrolement_administrateur` | activer un nouveau second facteur, rien d'autre | 10 min | prévu (étape 3.6) |

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
| `corps_illisible` | 400 | corps illisible pour une autre raison |
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

### 1.7 Sessions et jeton de renouvellement — **EN COURS, pas encore codé**

> ⚠️ **Contrat décidé, pas encore en service.** Tant que cette section porte la mention « en cours », le serveur répond encore `{"jeton": "<JWT>"}` (section 2) et les trois routes ci-dessous n'existent pas. Les applications peuvent s'y préparer, mais ne doivent pas encore s'en servir.

**Le principe.** Après la connexion, l'application reçoit deux jetons :
- un **jeton d'accès** (`jeton_acces`), de courte durée, à présenter à chaque demande (`Authorization: Bearer …`) ;
- un **jeton de renouvellement** (`jeton_renouvellement`), qui sert uniquement à obtenir une nouvelle paire quand le jeton d'accès expire. Il **change à chaque usage** : l'ancien ne sert plus. Le serveur n'en garde qu'une empreinte.

Chaque connexion ouvre une **session** (une par appareil). Le jeton d'accès porte l'identifiant de sa session : une session déconnectée refuse aussitôt ses jetons.

| Compte | Jeton d'accès | Sans renouvellement pendant… | Au plus tard, mot de passe redemandé après… |
|---|---|---|---|
| Patient | 15 min | 30 jours | 30 jours |
| Professionnel | 15 min | 1 h | 12 h |
| Administrateur | 15 min | 1 h | 24 h |

**Nouvelle réponse de la connexion et de l'inscription** (remplacera `{"jeton": …}`) :

```json
{
  "etape": "complete",
  "jeton_acces": "<JWT>",
  "expire_dans": 900,
  "jeton_renouvellement": "<64 caractères>"
}
```

- `expire_dans` : durée de vie du jeton d'accès, en secondes.
- Si le second facteur est actif : `{"etape": "second_facteur", "jeton_intermediaire": "<JWT>"}`, puis la paire après la saisie du code.
- L'inscription (201) renvoie la même forme, avec `"etape": "complete"`.

**Les trois routes :**

| Route | Appelant | Corps | Réponses |
|---|---|---|---|
| `POST /api/sessions/renouveler` | tout le monde, **sans** en-tête `Authorization` (le jeton d'accès peut être expiré) | `{"jeton_renouvellement": "…"}` | 200 : nouvelle paire, même forme que ci-dessus sans `etape` ; 401 `jeton_invalide` : jeton inconnu, expiré, session révoquée ou expirée, ou vol détecté |
| `POST /api/sessions/deconnexion` | tout le monde | `{"jeton_renouvellement": "…"}` | **toujours 204**, même pour un jeton inconnu : se déconnecter n'échoue jamais |
| `POST /api/sessions/deconnexion-partout` | jeton d'accès **valide** (si expiré : renouveler d'abord) | aucun | 204 : toutes les sessions du compte sont fermées, sur tous les appareils |

**Ce que fait l'application :**
- garder le jeton de renouvellement dans le **stockage sécurisé** du téléphone (Keystore sous Android, Keychain sous iOS) ;
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
| POST | `/api/assurance-munaseb/remboursements/simuler-acte` | jeton en exercice, agent MUNASEB | **hors service** |
| GET | `/api/assurance-munaseb/remboursements` | jeton en exercice, agent MUNASEB | **hors service** |
| GET | `/api/assurance-munaseb/remboursements/{id}` | jeton en exercice, agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/prendre-en-charge` | jeton en exercice, agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/valider` | jeton en exercice, agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/rejeter` | jeton en exercice, agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/payer` | jeton en exercice, agent MUNASEB | **hors service** |

> **Routes MUNASEB hors service.** L'ancienne connexion agent (`POST /api/assurance-munaseb/connexion`) est **supprimée** : un agent se connectera par la connexion professionnelle, puis choisira son affectation (étapes 3.6 et 3.9, section 4). Les routes de remboursement attendent déjà un jeton `laaficare:professionnel_en_exercice` ; tout autre jeton est refusé en 401. Avec un jeton de la bonne sorte, elles répondent encore `500 erreur_interne` : elles vérifient le compte agent dans une table supprimée par la migration 0011. Elles seront rebranchées sur les affectations à l'étape 3.9. Les chemins, corps et réponses des remboursements décrits ci-dessous devraient rester les mêmes ; leur description sert de référence pour l'écran agent.

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
| 201 | — | compte créé ; corps `{"jeton": "<JWT>"}`, jeton `laaficare:patient` |
| 422 | `telephone_invalide` | format du numéro non reconnu |
| 422 | `mot_de_passe_non_conforme` | + `regles_non_respectees` (voir 1.5) |
| 401 | `code_invalide` | code OTP faux, expiré ou déjà utilisé |
| 429 | `trop_de_demandes` | code faux, et c'était le dernier essai permis : le code est effacé, pas d'en-tête `Retry-After` (voir 2.2.5) |
| 409 | `telephone_deja_utilise` | un compte existe déjà pour ce numéro |
| 500 | `erreur_interne` | panne |

- Ordre des contrôles : numéro, puis mot de passe, puis code OTP. Un mot de passe refusé **ne consomme pas** le code : la personne corrige et renvoie la même requête.
- Le 409 n'arrive qu'après un code OTP correct : seul le titulaire du numéro l'apprend.

**Si la réponse se perd** (coupure réseau, délai dépassé) : le compte a peut-être été créé quand même, et le code OTP est peut-être déjà consommé. L'application ne renvoie pas la même inscription. Elle tente une **connexion** (`POST /api/patients/connexion`, section 2.2.3) avec le même téléphone et le même mot de passe :

| Réponse de la connexion | Ce que fait l'application |
|---|---|
| 200 | Le compte existe : la session est ouverte avec ce jeton, l'inscription est terminée. |
| 401 `identifiants_invalides` | Le compte n'a pas été créé. L'application reprend l'inscription : nouvelle demande de code (`POST /api/patients/otp`, qui peut répondre 429 si le dernier code date de moins de 60 s : attendre le délai de `Retry-After`), puis nouvel envoi du formulaire, qu'elle a gardé. |
| erreur réseau | L'application réessaie la connexion plus tard ; elle ne renvoie pas l'inscription. |

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
| 200 | — | corps `{"jeton": "<JWT>"}`, jeton `laaficare:patient` |
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
  "lieu_naissance": "Koudougou"
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

- **Erreurs** :

| Code | `code` | Cas |
|---|---|---|
| 401 | `authentification_requise` | en-tête `Authorization` absent |
| 401 | `jeton_invalide` | jeton expiré, d'une autre sorte, **émis avant une réinitialisation du mot de passe**, ou compte désactivé |
| 500 | `erreur_interne` | panne, ou incohérence en base |

- **Révocation** : après une réinitialisation du mot de passe, tous les jetons émis avant elle sont refusés (401 `jeton_invalide`) : l'application renvoie la personne à l'écran de connexion.
- **Compte verrouillé** (trop d'essais de mot de passe) : un jeton déjà reçu reste valable ; le verrouillage ne bloque que les nouvelles connexions.

### 2.3 Agent MUNASEB (hors service, voir l'encadré plus haut)

#### 2.3.1 Connexion de l'agent

L'ancienne route `POST /api/assurance-munaseb/connexion` est **supprimée** (2026-10-08), avec son code `423 compte_verrouille`. L'agent passera par la connexion professionnelle (section 4.1), puis par le choix de son affectation (section 4.4).

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
- L'agent ne voit **jamais** le NIP ni le téléphone du patient.
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

---

## 3. Codes d'erreur : index

| `code` | HTTP | Routes |
|---|---|---|
| `authentification_requise` | 401 | routes protégées |
| `jeton_invalide` | 401 | routes protégées |
| `acces_refuse` | 403 | routes protégées |
| `json_invalide`, `donnees_invalides`, `content_type_invalide`, `corps_illisible` | 400, 422, 415 | routes avec corps |
| `chemin_invalide` | 400 | routes avec `{id}` |
| `parametre_invalide` | 400 | liste des remboursements |
| `telephone_invalide` | 422 | routes patient |
| `code_invalide` | 401 | inscription, réinitialisation |
| `trop_de_demandes` | 429 | demande de code (avec `Retry-After`), inscription et réinitialisation (sans) |
| `telephone_deja_utilise` | 409 | inscription |
| `telephone_inconnu` | 404 | réinitialisation |
| `mot_de_passe_non_conforme` | 422 | inscription, réinitialisation |
| `identifiants_invalides` | 401 | connexions |
| `montant_invalide`, `nip_invalide`, `partenaire_suspendu`, `non_couvert`, `en_carence` | 422 | simuler-acte |
| `patient_inconnu`, `partenaire_inconnu` | 404 | simuler-acte |
| `reference_acte_deja_utilisee` | 409 | simuler-acte |
| `demande_inconnue` | 404 | détail, transitions |
| `transition_invalide` | 409 | transitions |
| `motif_vide`, `motif_trop_long` | 422 | rejet |
| `erreur_interne` | 500 | toutes |

---

## 4. Routes prévues (pas encore codées)

### 4.0 Sessions (en cours)

Les routes `POST /api/sessions/renouveler`, `POST /api/sessions/deconnexion` et `POST /api/sessions/deconnexion-partout`, et la nouvelle réponse des connexions et de l'inscription, sont décrites en section 1.7. Elles sont écrites avant la connexion professionnelle (étape 3.6.2).

Rien de cette section n'existe dans le code. **Les chemins, corps et réponses ne sont pas encore fixés** : chacun sera présenté au porteur et validé à son étape, puis décrit en section 2. Seules les fonctions et les règles déjà décidées (CLAUDE.md, sections 12 et 14) sont listées ici, pour que les applications puissent préparer leurs écrans.

### 4.1 Authentification professionnelle et administrateur (étape 3.6)

- Connexion professionnelle par **email + mot de passe** ; mêmes règles que le patient : réponse unique `identifiants_invalides`, message « Email ou mot de passe incorrect. », verrouillage 5 / 15 min.
- **Connexion en deux temps** quand le second facteur est actif : un mot de passe correct donne un **jeton intermédiaire** de courte durée, accepté seulement par la route qui reçoit le code TOTP ou un code de secours. Vaut aussi pour le patient qui a activé le TOTP.
- Connexion administrateur LaafiCare : TOTP toujours exigé.
- Nouvelle forme du jeton : il portera la `version_jeton` du compte (un jeton est refusé après réinitialisation du mot de passe ou bascule) et, côté professionnel, l'affectation active.

### 4.2 Second facteur TOTP

Le module existe déjà côté serveur ; les routes manquent.

- Activation : demande le mot de passe ; le serveur renvoie l'URL `otpauth://` et la clé en base 32 (l'application dessine le QR code) ; l'activation n'est effective qu'après un premier code correct ; 10 codes de secours `XXXXX-XXXXX` montrés une seule fois.
- Facultatif pour patients et professionnels, obligatoire pour les administrateurs.
- La personne ne désactive pas elle-même son TOTP en V1 (seule l'équipe LaafiCare, par un outil local).

### 4.3 Structures et pièces justificatives (étapes 3.7 et 3.8)

- Création d'une structure par un patient (qui en devient responsable ; création du compte professionnel au passage s'il n'en a pas), brouillon, modification, abandon.
- Téléversement d'une pièce, **une requête par fichier** (réseau faible) : PDF, JPEG ou PNG, 5 Mo au plus.
- Soumission, suivi de la demande par le responsable.
- Côté administrateur : liste des demandes en attente (avec avertissements de numéros d'autorisation en double, de plusieurs demandes MUNASEB), consultation et téléchargement des pièces, validation (licence), refus (motif), suspension, réactivation.

### 4.4 Affectations et rôle actif (étape 3.9)

- Après la connexion professionnelle : liste des affectations de la personne, choix de l'une d'elles → jeton portant l'affectation active ; changement de structure sans mot de passe.
- Routes de remboursement MUNASEB rebranchées sur l'affectation `agent_assurance_munaseb` d'une MUNASEB validée.

### 4.5 Invitations (étape 3.10)

- Création d'une invitation à un numéro de téléphone (réponse toujours « invitation envoyée »), annulation, liste.
- Côté invité, depuis son compte patient : liste des invitations reçues, acceptation (création du compte professionnel la première fois), refus.
- Validité 7 jours.

### 4.6 Désactivation d'une affectation et bascule (étape 3.11)

- Désactivation d'une affectation (jamais d'un compte, jamais de suppression).
- Bascule professionnel → patient sans mot de passe ; patient → professionnel avec le mot de passe professionnel.

### 4.7 Contrat MUNASEB côté agent (étape 5)

- Adhésion (avec carence d'un mois), renouvellement, consultation des droits d'un adhérent, suspension (à concevoir).

### 4.8 Côté patient (étape 6)

- Ses droits MUNASEB (plafond restant, période, carence).
- Ses demandes de remboursement et leur statut.
- Ses notifications : liste, marquer comme lue.
