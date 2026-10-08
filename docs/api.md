# API LaafiCare : contrat entre le backend et les applications

Ce document décrit les routes de l'API Rust/Axum telles qu'elles sont **codées aujourd'hui**, puis celles qui sont **seulement prévues**. Les applications Angular (web) et Flutter (mobile) s'appuient sur lui. Il est mis à jour avec chaque ajout ou modification d'une route.

État au 2026-10-08, branche `dev`.

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

Le jeton est un JWT signé en HS256, **valable 24 heures**. Il contient :

| Champ | Contenu |
|---|---|
| `sub` | identifiant de l'utilisateur (`utilisateur.id`) |
| `role` | `patient` ou `agent_assurance_munaseb` (aujourd'hui) |
| `iat`, `exp` | date d'émission et d'expiration (secondes depuis 1970) |

Les applications ne doivent **pas** lire le contenu du jeton pour décider de ce qu'elles affichent : elles le gardent et le renvoient tel quel. Sa forme va changer à l'étape 3.6 (voir section 4).

Refus d'une route protégée (RFC 6750) :

| Code HTTP | `code` | Cas | En-tête renvoyé |
|---|---|---|---|
| 401 | `authentification_requise` | en-tête `Authorization` absent ou mal formé | `WWW-Authenticate: Bearer` |
| 401 | `jeton_invalide` | signature fausse, jeton expiré ou illisible | `WWW-Authenticate: Bearer error="invalid_token"` |
| 403 | `acces_refuse` | jeton valide mais mauvais rôle, ou droit retiré depuis l'émission | — |
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
| POST | `/api/assurance-munaseb/connexion` | agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/simuler-acte` | jeton agent MUNASEB | **hors service** |
| GET | `/api/assurance-munaseb/remboursements` | jeton agent MUNASEB | **hors service** |
| GET | `/api/assurance-munaseb/remboursements/{id}` | jeton agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/prendre-en-charge` | jeton agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/valider` | jeton agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/rejeter` | jeton agent MUNASEB | **hors service** |
| POST | `/api/assurance-munaseb/remboursements/{id}/payer` | jeton agent MUNASEB | **hors service** |

> **Routes MUNASEB hors service.** Elles vérifient le compte agent dans la table `agent_assurance_munaseb`, supprimée par la migration 0011 (passage aux comptes séparés et aux affectations, section 14 du CLAUDE.md). La connexion agent et toute route qui exige un jeton agent répondent donc aujourd'hui `500 erreur_interne`. Elles seront rebranchées sur les affectations aux étapes 3.6 et 3.9 : la connexion changera (section 4), mais les chemins, corps et réponses des remboursements décrits ci-dessous devraient rester les mêmes, à confirmer à l'étape 3.9. Leur description sert donc de référence pour l'écran agent.

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
| 500 | `erreur_interne` | panne |

- Le code est valable **5 minutes**. Une nouvelle demande remplace le code précédent.
- La réponse est la même que le numéro ait un compte ou non.
- En développement, aucun SMS ne part : le code s'affiche dans les journaux du serveur.
- **Pas encore de limite** du nombre de demandes par numéro (point ouvert avant la production).

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
| 201 | — | compte créé ; corps `{"jeton": "<JWT>"}`, rôle `patient` |
| 422 | `telephone_invalide` | format du numéro non reconnu |
| 422 | `mot_de_passe_non_conforme` | + `regles_non_respectees` (voir 1.5) |
| 401 | `code_invalide` | code OTP faux, expiré ou déjà utilisé |
| 409 | `telephone_deja_utilise` | un compte existe déjà pour ce numéro |
| 500 | `erreur_interne` | panne |

- Ordre des contrôles : numéro, puis mot de passe, puis code OTP. Un mot de passe refusé **ne consomme pas** le code : la personne corrige et renvoie la même requête.
- Le 409 n'arrive qu'après un code OTP correct : seul le titulaire du numéro l'apprend.

#### 2.2.3 `POST /api/patients/connexion`

- **Appelant** : tout le monde, sans jeton.
- **Corps** :

```json
{ "telephone": "70 00 00 00", "mot_de_passe": "Exemple#2026" }
```

- **Réponses** :

| Code | `code` | Cas |
|---|---|---|
| 200 | — | corps `{"jeton": "<JWT>"}`, rôle `patient` |
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
| 404 | `telephone_inconnu` | aucun compte pour ce numéro (seulement après un code correct) |
| 500 | `erreur_interne` | panne |

- Effets : le verrouillage est levé, le compteur d'échecs remis à zéro.
- Aucun jeton n'est renvoyé : la personne se connecte ensuite avec son nouveau mot de passe.
- La réinitialisation **ne touche jamais au second facteur** (TOTP).

### 2.3 Agent MUNASEB (hors service, voir l'encadré plus haut)

#### 2.3.1 `POST /api/assurance-munaseb/connexion`

Sera **remplacée** par la connexion professionnelle (section 4). Décrite pour mémoire.

- **Corps** : `{"email": "...", "mot_de_passe": "..."}`
- **Réponses** : 200 `{"jeton": "<JWT>"}` (rôle `agent_assurance_munaseb`) ; 401 `identifiants_invalides` ; 423 `compte_verrouille` ; 500 `erreur_interne`.
- Le 423 est **obsolète** (décision L1 du 2026-10-02 : un compte verrouillé répond 401 `identifiants_invalides`, comme un compte inconnu). Il disparaîtra avec cette route.

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
| `telephone_deja_utilise` | 409 | inscription |
| `telephone_inconnu` | 404 | réinitialisation |
| `mot_de_passe_non_conforme` | 422 | inscription, réinitialisation |
| `identifiants_invalides` | 401 | connexions |
| `compte_verrouille` | 423 | connexion agent (obsolète) |
| `montant_invalide`, `nip_invalide`, `partenaire_suspendu`, `non_couvert`, `en_carence` | 422 | simuler-acte |
| `patient_inconnu`, `partenaire_inconnu` | 404 | simuler-acte |
| `reference_acte_deja_utilisee` | 409 | simuler-acte |
| `demande_inconnue` | 404 | détail, transitions |
| `transition_invalide` | 409 | transitions |
| `motif_vide`, `motif_trop_long` | 422 | rejet |
| `erreur_interne` | 500 | toutes |

---

## 4. Routes prévues (pas encore codées)

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
