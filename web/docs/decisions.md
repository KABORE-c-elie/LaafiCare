# Décisions du front-end web

Auteur : Calliste Elie KABORE.

## 1. Angular pour les applications web (2026-10-19)

Les applications web (MUNASEB d'abord, puis hôpital, pharmacie, laboratoire, administration LaafiCare) sont écrites avec **Angular 22**, dans un seul espace de travail (`web/`) :

- une bibliothèque `commun`, partagée par toutes les applications : appels au serveur, session, messages d'erreur, composants ;
- une application par usage, à commencer par `munaseb`.

Une comparaison avec Leptos (Rust compilé en WebAssembly) a été faite sur documents. Angular a été retenu pour la sécurité (échappement et nettoyage du HTML intégrés), le calendrier de support publié (24 mois par version majeure) et les composants accessibles officiels (CDK).

## 2. Où sont gardés les jetons dans le navigateur : option A (2026-10-19)

**Décision.** Le jeton d'accès et le jeton de renouvellement restent **en mémoire seulement**, dans l'application. Jamais dans `localStorage`, `sessionStorage` ni un cookie.

**Conséquence voulue.** Un rechargement de la page, ou un nouvel onglet, redemande la connexion. Chaque onglet a sa propre session.

**Pourquoi pas le stockage du navigateur.** OWASP, *Session Management Cheat Sheet* : « Do not store authentication tokens, session IDs, JWTs, refresh tokens, or any credential in `localStorage` ». Ces espaces sont lisibles par tout script de la page : une seule faille XSS livrerait les jetons.

**Limite.** La mémoire ne protège pas complètement non plus. RFC 10017 (*OAuth 2.0 for Browser-Based Applications*, BCP 212, août 2026), §8 : « none of these options can fully mitigate token exfiltration when the attacker can execute malicious code ». La première défense reste d'empêcher toute injection de script (aucun `innerHTML`, aucun `bypassSecurityTrust*`, politique CSP stricte au déploiement).

## 3. Option C, à décider côté backend avant le pilote

**Option C.** Le jeton de renouvellement passe dans un cookie posé par le serveur : `__Host-`, `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`. Le jeton d'accès reste en mémoire.

- **Avantage.** Un script, même malveillant, ne peut pas lire le jeton de renouvellement (OWASP, *Session Management Cheat Sheet*, attributs `HttpOnly`, `Secure`, `SameSite` et préfixe `__Host-`). La session survit au rechargement de la page.
- **Ce que cela demande au backend** : `POST /api/sessions/renouveler` et `POST /api/sessions/deconnexion` liraient le cookie au lieu du corps JSON, et poseraient ou effaceraient le cookie (`Set-Cookie`). Il faudrait aussi une protection contre les requêtes intersites (CSRF) : `SameSite=Strict`, application et API sur la même origine, contrôle d'un en-tête.
- **Référence.** RFC 10017, §6 : les architectures y sont classées « in decreasing order of security », le *Backend For Frontend* en premier, puisque le navigateur n'y détient aucun jeton.

**C'est une décision du backend** (`docs/api.md` et le Claude Code du dossier `LaafiCare`), à prendre **avant le pilote**. Le front-end n'aura alors qu'à ne plus garder le jeton de renouvellement.

## 4. Autres choix de la partie 1

- **Zoneless**, défaut d'Angular depuis la version 21 (angular.dev/guide/zoneless) : ni `zone.js`, ni `provideZoneChangeDetection`.
- **`withFetch` n'est pas appelé** : Angular 22 le déclare déprécié, « FetchBackend is the default HttpBackend » (angular.dev/api/common/http/withFetch). Les requêtes passent bien par `fetch`.
- **Délai de 10 s** sur chaque requête (option `timeout` d'HttpClient). Un renouvellement sans réponse est relancé une seule fois avec le même jeton, sous les 30 s de `docs/api.md` §1.7.
- **Une seule feuille de style** par application, sans style dans les composants, avec `optimization.styles.inlineCritical = false` : la politique `style-src 'self'` suffit pour nos styles. **À vérifier au déploiement** : le CDK peut insérer ses propres styles pendant l'exécution.
- **La bibliothèque est utilisée depuis `dist/commun`**, comme le demande la doc Angular (« TypeScript path mappings should *not* point to the library source `.ts` files »). Il faut donc construire `commun` avant l'application (`npm run build`, `npm start`).
- **Budgets de construction** : avertissement à 320 kB et erreur à 400 kB pour le chargement initial (mesuré : 270 kB bruts à la partie 1, 301 kB à la partie 2) ; tout style de composant déclenche un avertissement.
- **Point d'entrée secondaire `commun/fenetre`** (partie 2) : la fenêtre du CDK était entraînée dans le paquet initial (environ 75 kB de Dialog, Overlay et A11y) parce que `commun` n'avait qu'un point d'entrée. Placée à part (doc Angular, « secondary entry points »), elle n'est chargée qu'avec la page de détail.
- **Pas de favicon** : aucune icône web n'est encore dans le dépôt. Celle d'Angular a été retirée ; `index.html` déclare une icône vide (`data:,`) pour éviter une erreur 404.

## 5. Politique de sécurité du contenu (CSP), partie 3 (2026-10-20)

**Politique servie** (essayée avec `outils/serveur-production.mjs`, à reprendre telle quelle sur le serveur réel) :

```
default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:;
object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'
X-Content-Type-Options: nosniff
Referrer-Policy: no-referrer
```

**Ce que la console a signalé, et ce qui a été fait, sans assouplir la politique :**

| Blocage | Cause | Correction |
|---|---|---|
| `base-uri 'none'` | la balise `<base href="/">` créée par Angular | balise retirée ; fichiers à chemins absolus (`deployUrl: "/"`, schéma du constructeur Angular : « base path for the URLs of resources in index.html ») ; routeur configuré par `APP_BASE_HREF` (angular.dev/api/common/APP_BASE_HREF) ; logo en chemin absolu |
| 404 sur `favicon.ico` | aucune icône | icône vide `data:,` (permise par `img-src data:`) |
| 3 × `style-src-elem` (inline) | la CDK insère ses styles par des éléments `<style>` pendant l'exécution (overlay, texte masqué, conteneur de fenêtre) | les **mêmes règles** sont chargées depuis la feuille globale (`overlay-prebuilt.css`, `a11y-prebuilt.css`, règle `.cdk-dialog-container`) : tout s'affiche correctement. **Reste** : la console affiche toujours ces 3 blocages quand une fenêtre s'ouvre. Pour les faire taire sans `'unsafe-inline'`, il faudrait ajouter leurs empreintes (`'sha256-…'`) à `style-src` : c'est une modification de la politique, **non faite**, à décider par le porteur. |

**Trusted Types : non activés.** Angular les prend en charge (politiques `angular` et `angular#bundler`, angular.dev/best-practices/security), mais l'essai du 2026-10-20 a bloqué l'enregistrement du service worker : `navigator.serviceWorker.register` exige un `TrustedScriptURL` (« This document requires 'TrustedScriptURL' assignment », puis l'erreur Angular `NG05604`). Les activer demanderait du code (une politique à nous, limitée à `ngsw-worker.js`, et l'enregistrement du service worker fait à la main). Hors de la consigne « sans changement de code » : à décider plus tard.

## 6. Service worker et hors connexion, partie 3 (2026-10-20)

- `@angular/service-worker` 22.2.3, configuré par `projects/munaseb/ngsw-config.json` : **seulement les fichiers de l'application** (`index.html`, JavaScript, CSS, logo). **Aucun `dataGroups`** : la doc Angular précise que les requêtes de données ne sont mises en cache que selon les règles configurées. Les réponses de `/api` ne sont donc jamais gardées. Vérifié le 2026-10-20 : les caches ne contiennent aucune adresse `/api`. `/api/**` est aussi exclu des navigations.
- Activé seulement hors mode développement (`enabled: !isDevMode()`), enregistré quand l'application est stable (`registerWhenStable:30000`).
- **Hors connexion**, après une première visite, l'application s'ouvre et affiche « Pas de connexion. Vos demandes s'afficheront dès le retour du réseau. » La liste et le détail se rechargent seuls au retour du réseau (événements `online` / `offline`, MDN).
- **Nouvelle version** : un bandeau la propose, avec un bouton « Recharger maintenant ». Rien ne se recharge sans le clic de l'agent (doc Angular : demander l'accord avant de recharger). Recharger redemande la connexion (jetons en mémoire), et le bandeau le dit.

## 7. Journal des erreurs en développement (2026-10-20)

Pour tester le backend depuis l'application, sans Postman : en développement seulement, chaque erreur de l'API écrit une ligne dans la console du navigateur (`intercepteurJournalDev`, dans `commun`).

- **Contenu** : code HTTP, champ `code`, méthode et chemin, **sans** les paramètres après `?`. Jamais le corps envoyé ou reçu, ni les en-têtes, ni le message du serveur. Un test vérifie qu'aucun mot de passe, jeton, code TOTP, code de secours ni donnée d'adhérent n'y apparaît.
- **Jamais en production** : l'intercepteur n'est déclaré que dans `environment.development.ts`, qui remplace `environment.ts` seulement pour `ng serve` (`fileReplacements`, angular.dev/tools/cli/environments). Contrôlé sur la construction de production : le texte `[API]` n'y figure pas.

## 8. Points ouverts du front-end web

| Point | État | Décision |
|---|---|---|
| **Styles en ligne de la CDK** | La CDK insère 3 éléments `<style>` (overlay, texte masqué, conteneur de fenêtre), bloqués par `style-src 'self'`. L'affichage reste correct (mêmes règles dans la feuille globale), mais la console affiche ces blocages quand une fenêtre s'ouvre. | **On ne touche pas à la CSP** (porteur, 2026-10-20). Les empreintes `sha256` ne sont pas ajoutées. |
| **Trusted Types** | Bloquent l'enregistrement du service worker d'Angular : `navigator.serviceWorker.register` exige un `TrustedScriptURL`, d'où l'erreur `NG05604`. | Non activés ; les activer demanderait une politique propre à l'application et un enregistrement du service worker fait à la main. |
| **vite 8.3.0** (`npm audit` du 2026-10-22 : 1 faille élevée, 1 faible ; `launch-editor` et vite, domaine vulnérable 8.3.0 à 8.3.2) | Épinglé exactement par `@angular/build` 22.2.3, **et encore par la 22.2.4** sortie le 2026-10-22, qui ne corrige donc rien. Concerne le serveur de développement seulement, pas la construction de production. | **Pas de mise à jour vers 22.2.4** (porteur, 2026-10-22) : tous les paquets Angular restent sur la même version ; on attend une version qui fixe **vite en 8.3.3 ou plus**. Ni `overrides`, ni retour à la 22.1.9. `ng serve` reste sur `localhost`, jamais `--host`. `npm audit` est relancé au début de chaque séance. |
| **Option C des jetons** | Voir section 3. | À décider côté backend, avant le pilote. |
| **Favicon** | Aucune icône web dans le dépôt. | En attente des fichiers. |
| **Conseil sous le mot de passe professionnel** : « Choisissez un mot de passe différent de celui de votre compte patient » (`docs/points-ouverts.md`, étape 3.7 ; un conseil, pas une obligation, le serveur accepte un mot de passe identique) | Aucun formulaire de création de mot de passe dans le web pour l'instant : la connexion et le renvoi du lien demandent un mot de passe existant. | À placer sous le champ « mot de passe professionnel » de l'**écran de création de structure**, dont l'emplacement (application mobile ou espace patient du web, docs/api.md §2.8) reste à décider. |

## 9. Palette de l'application MUNASEB

Toutes les couleurs sont des variables CSS dans `projects/munaseb/src/styles.css`. Contrastes calculés avec la formule WCAG 2.2 : tous les textes atteignent au moins 4,5:1.

| Usage | Couleurs | Contraste |
|---|---|---|
| Bouton principal | blanc sur rose `#8C4C65` (survol `#7A3F56`) | 6,29:1 (7,86:1) |
| Titre, pastille d'espace | rose `#8C4C65` sur rose clair `#F7EBEF` | 5,42:1 |
| Texte | `#13201B` sur fond `#F7FAF8` / surface `#FFFFFF` | 15,99:1 / 16,80:1 |
| Texte secondaire | `#45524C` sur surface | 8,19:1 |
| En attente | `#13201B` sur `#E1EBE6` | 13,78:1 |
| En cours, information, focus | `#2A5F94` sur `#E6EEF6` | 5,67:1 |
| Validée | `#0A6650` sur `#E3FAF1` | 6,34:1 |
| Payée | blanc sur `#0A6650` | 6,93:1 |
| Rejetée, erreur de saisie | `#2F3E4C` sur `#EDF1F4` (bord `#828D87`) | 9,66:1 |
| Bouton « Rejeter » | blanc sur `#13201B` | 16,80:1 |
| Bord des champs (non textuel, 3:1 exigé) | `#828D87` sur blanc | 3,44:1 |

Jamais de rouge ni d'orange (réservés aux alertes de santé, CLAUDE.md §6). Chaque statut a une icône et un texte. Le contour de focus (3 px, `#2A5F94`) est dessiné à 2 px de l'élément, sur le fond de la page (6,64:1).

## 10. Vérification de l'email et renvoi du lien (2026-10-21)

Contrat : `docs/api.md` §2.4 (403 `email_non_verifie`) et §2.7 (`/verifier`, `/renvoyer`).

**Page `/verifier-email`**, sans connexion ni garde (`pages/verifier-email.ts`) :
- **Le jeton est lu après `#`**, puis le fragment est **retiré aussitôt de l'adresse** par `history.replaceState()` (MDN : remplace l'entrée courante de l'historique, sans recharger la page). L'état de l'historique est gardé (le routeur y range son numéro de navigation). Le fragment n'est jamais envoyé au serveur par le navigateur (RFC 3986 §3.5).
- **Un second lien ouvert dans le même onglet** ne change que le fragment : la page ne se recharge pas et le routeur réutilise la page. Le fragment est donc relu à chaque fin de navigation. Trouvé à l'essai dans Chrome : sans cela, le second jeton restait dans l'adresse.
- **Ouvrir le lien ne vérifie rien** : le jeton part au clic sur « Confirmer mon adresse » (§2.7), par **POST**, dans le corps, jamais par GET (RFC 9110 §9.2.1). Il reste en mémoire dans la page seulement (ni stockage du navigateur, ni journal), et il est oublié dès la réponse 204 ou `lien_invalide`. Après une erreur réseau, il est gardé pour réessayer.
- **Pas de jeton dans l'adresse** : même écran que `lien_invalide`, avec le formulaire de renvoi.

**Renvoi du lien** (`renvoi-lien.ts`, partagé par la connexion et `/verifier-email`) :
- **Sur l'écran de connexion**, après un 403 `email_non_verifie`, le mot de passe reste dans son champ (masqué) au lieu d'être effacé. « Renvoyer le lien » réutilise l'email et le mot de passe saisis, puis efface le mot de passe : il n'est gardé que le temps de cet envoi.
- **Sur 429 `trop_de_demandes`**, la page lit `Retry-After` (jamais le message), ou 60 s s'il manque (« moins d'une minute », §2.7). Le bouton est désactivé pendant ce délai, avec le décompte sur le bouton. Le message « Vous pourrez demander un nouveau lien dans 42 s. » est écrit une seule fois : réécrit chaque seconde, il serait relu sans cesse par les lecteurs d'écran. Le format suit §2.2.5 : en secondes jusqu'à une minute, en minutes au-delà.

**Textes affichés**, du porteur :

| Situation | Texte |
|---|---|
| 204 | « Votre email est vérifié. Vous pouvez maintenant vous connecter. » |
| 400 `lien_invalide`, ou aucun jeton | « Ce lien n'est plus valable ou a déjà servi. Demandez-en un nouveau. » |
| Renvoi réussi | « Si votre compte existe, un nouveau lien vous a été envoyé. » |

docs/api.md ne fixe pas de texte pour ces cas. Le message `erreur` du serveur pour `lien_invalide` (« Ce lien n'est plus valable. Demandez-en un nouveau. ») n'est jamais affiché, comme tous les messages du serveur.

**Aide « Sans accent ni espace »** sous les champs email (connexion, renvoi), reliée au champ par `aria-describedby` (`lc-champ`, entrée `aide`).

**Service worker** : `/verifier-email` est exclu des navigations qu'il sert (`"!/verifier-email"` dans `ngsw-config.json`). La page vient donc toujours du réseau, dans sa dernière version. Sinon, une ancienne version gardée en cache, qui ne connaîtrait pas encore cette page, renverrait vers la connexion et perdrait le jeton. Les fichiers JavaScript de la nouvelle version, absents de l'ancien manifeste, passent eux aussi par le réseau (code de `ngsw-worker.js` : une requête qu'aucun groupe ne sert part au réseau). Hors connexion, la page ne s'ouvre pas, mais la vérification exige le réseau de toute façon. Le POST n'est jamais mis en cache : le service worker d'Angular ne sert que les GET, et il n'a aucun `dataGroups`.

**Vérifié le 2026-10-21** dans Chrome 155 sans interface, sur la construction de production servie par `serveur-production.mjs` (CSP stricte), avec un faux backend :
- `/connexion` vient du cache du service worker, `/verifier-email` du réseau ;
- aucune requête avant le clic, puis un POST avec le jeton dans le corps ;
- adresse et historique sans jeton, second lien compris ;
- aucune violation de la CSP ;
- ni jeton ni mot de passe dans la console.

## 11. Rôles de l'application MUNASEB et invitations du responsable (2026-10-22)

Décisions A1 à A6 du porteur (2026-10-22). Contrat : `docs/api.md` §2.6 et §2.10.

**Rôles.**
- **Choix de l'espace** : il montre les affectations des structures `munaseb` dont le rôle est `agent_assurance_munaseb` ou `responsable` (`roles.ts`). Une autre affectation s'ouvre dans l'application de son métier.
- **Page d'arrivée par rôle** : l'agent arrive sur `/demandes`, le responsable sur `/invitations`.
- **Gardes par rôle** (`exigerRole`, après `exigerEnExercice`) : un autre rôle revient au choix de l'espace. Elles ne protègent pas les données : le serveur répond 403 `acces_refuse`, et l'intercepteur ramène alors au choix de l'espace avec « Votre accès à cet espace a été retiré. ».

**Inviter.**
- **Numéro** : envoyé **tel que saisi**. Le serveur le normalise (§1.6 : « L'application peut donc envoyer le numéro tel que saisi »), et aucune de ses règles n'est recopiée. L'aide sous le champ garde « Un numéro étranger commence par + » : §1.6 accepte un numéro étranger de 7 à 15 chiffres.
- **Rôle** : « Agent MUNASEB » est choisi par défaut. « Responsable » demande une **confirmation** (porteur, A3), car ce rôle peut inviter et retirer des accès. La fenêtre fait l'envoi elle-même, comme celle de la validation : jamais deux envois.
- **Réponse** : toujours « Invitation envoyée. Valable jusqu'au JJ/MM/AAAA. », que le numéro ait un compte ou non (§2.10). La date est le jour du Burkina, comme dans le SMS.

**Liste.**
- **Colonnes** : téléphone (affiché en entier : c'est le responsable qui l'a saisi), rôle, statut, dates d'envoi et d'expiration, et le nom de la personne **après l'acceptation seulement**.
- **Pagination** : 20 par page, avec le paramètre `avant`.
- **Annuler** : seulement pour une invitation en attente, après une confirmation.
- **Statuts** : neutres, « Acceptée » en vert. Jamais de rouge ni d'orange (CLAUDE.md §6) ; toujours une icône et un texte (WCAG 2.2, 1.4.1).

**Messages** : un texte par `code` (M1, porteur, 2026-10-22), dans `MESSAGES_INVITATION` (`pages/invitations.ts`). Le message du serveur n'est jamais affiché. Le 429 `trop_de_demandes` n'a pas de décompte, puisque §2.10 ne donne pas de `Retry-After` pour cette limite.

**Numéro jamais dans la console** : le journal de développement n'écrit que la méthode et le chemin. Un test vérifie que le numéro (saisi, normalisé, chiffres seuls) n'apparaît dans aucune ligne de la console.

**Reporté (A6)** : liste des membres et retrait d'un accès (§2.12).

## 12. Mise en commun pour l'application d'administration (2026-10-22)

Décision B2 du porteur : ce qui sert aux deux applications est dans la bibliothèque `commun`, et rien n'est recopié. Ce lot ne change ni le comportement ni l'affichage de munaseb.

| Où | Quoi |
|---|---|
| `commun` (point d'entrée principal) | `PARCOURS_CONNEXION` : routes de l'API (§2.4 ou §2.5) et pages du parcours ; parcours professionnel par défaut, que l'administration remplacera. `AccesService` le lit. `ESPACE_APPLICATION` : logo, titre et phrase du panneau d'accueil, fourni par chaque application. Il est dans le point d'entrée principal parce que la configuration de l'application le fournit au démarrage, et qu'il ne doit pas tirer les écrans de connexion dans le paquet initial. |
| `commun/connexion` (nouveau point d'entrée secondaire) | `lc-accueil` (panneau et zone centrée), `lc-formulaire-connexion` (email, mot de passe, navigation selon `etape`), `SecondFacteurPage`. La page de connexion de munaseb n'y ajoute que « Renvoyer le lien ». Point d'entrée secondaire, comme `commun/fenetre`, pour rester dans les pages chargées à la demande. |
| `commun/fenetre` | `IssueFenetre`, la fenêtre de confirmation, et la **fenêtre de motif** générique (textes, longueur maximale, `envoyer`), utilisée pour le rejet d'une demande et, au lot D, pour le refus d'une structure. |
| `projects/commun/styles/base.css` | Tous les styles communs. La feuille de chaque application ne garde que sa palette et ses pages, et `angular.json` charge `base.css` avant elle. Les couleurs de marque ont des noms neutres : `--principal`, `--principal-survol`, `--accent-fond`, `--sur-principal`. |

**Tests des écrans communs (décision du porteur, 2026-10-22)** : ils sont testés dans `commun` lui-même, sans dépendre d'une version construite. Les tests des applications restent aussi.
- **`projects/commun/tsconfig.spec.json`**, utilisé par les tests de `commun` seulement : `paths` fait pointer `commun` vers `./src/public-api.ts` et `commun/*` vers `./*/src/public-api.ts`. TSConfig Reference, « paths » : sans `baseUrl`, les chemins sont lus depuis le fichier qui les déclare ; une option redéfinie dans `compilerOptions` remplace celle du fichier étendu (« extends »). Le `tsconfig.json` racine garde `dist/commun` pour la construction et les applications.
- **`include`** de ce fichier, et option **`include`** du constructeur de tests (`angular.json`, `commun:test`) : elle ajoute `../connexion/**/*.spec.ts` et `../fenetre/**/*.spec.ts`. D'après le schéma du constructeur `@angular/build:unit-test`, les motifs sont « relative to the project source root », c'est-à-dire `projects/commun/src`.
- **Vérifié** : `dist/commun` supprimé, `ng test commun` passe (49 tests, dont 11 pour `commun/connexion` et `commun/fenetre`). La construction de production est inchangée (312,25 kB), et aucun test n'entre dans la bibliothèque construite.

**Même affichage, vérifié** : 9 captures de Chrome (connexion, vérification de l'email, second facteur, choix de l'espace, demandes, détail, fenêtre de rejet, invitations, invitation envoyée), avant et après le lot, sur la construction de production avec un faux backend. Les 9 sont identiques au pixel près, avec la même empreinte SHA-256.

**Défaut corrigé (2026-10-22, commit séparé)** : sur l'écran du second facteur, « Valider » rechargeait la page au lieu d'envoyer le code, et la session (en mémoire) était perdue. Le `<form>` avait `(ngSubmit)` sans `[formGroup]`. Avec les formulaires réactifs, seule `FormGroupDirective` (sélecteur `[formGroup]`) émet `ngSubmit` ; `NgForm` est dans `FormsModule`. Le navigateur faisait donc sa soumission native. Corrigé par un `FormGroup`. Un test vérifie que l'évènement `submit` est annulé et que le code part : sans le correctif, ses 4 cas échouent.

## 13. Application d'administration : connexion et réenrôlement (2026-10-22)

Décisions B1, B3, B4, B5, B10 et B11 du porteur (2026-10-22). Contrat : `docs/api.md` §2.5.

**Projet `administration`** (`projects/administration`) :
- en développement, `npm run start:administration`, sur `localhost:4201` seulement ;
- construction dans `dist/administration` ;
- essai de production avec `npm run serve:prod:administration`, sur le port 8083 : même CSP stricte que munaseb, et le serveur prend maintenant le nom de l'application ;
- **pas de service worker** (B11) : aucun besoin hors connexion, et l'application vient toujours du serveur ;
- le logo est pris dans `projects/munaseb/public` (option `assets` d'`angular.json`), sans copie du fichier.

**Ce que `commun` gagne pour ce parcours :**
- `PARCOURS_ADMINISTRATEUR`, que `app.config.ts` fournit à la place du parcours professionnel ;
- la session : états `reenrolement` (jeton de réenrôlement, 10 min, jamais renouvelé) et `administrateur` (jeton d'accès renouvelé comme les autres, §1.7) ;
- le contexte de requête `reenrolement`, présenté par l'intercepteur.

**Connexion (§2.5) :**
- **Le second temps est toujours demandé.** La réponse `reenrolement` mène à la page `/reenrolement`. Une réponse `complete` n'ouvre rien : §2.5 dit « Jamais d'`etape: "complete"` pour un administrateur », et l'écran affiche alors l'erreur interne.
- **403 `email_non_verifie`** : « Votre email n'est pas encore vérifié. Contactez l'équipe LaafiCare. » (validé, B3). Aucune route ne permet à un administrateur de renvoyer le lien.

**Réenrôlement (`pages/reenrolement.ts`), en trois étapes :**
1. le mot de passe est redemandé (`commencer`). Refusé : « Mot de passe incorrect. » (§2.5) ;
2. le QR code, puis la clé à la demande, par groupes de 4, puis le premier code (`confirmer`). Refusé : « Code incorrect. », et le champ est vidé. « Obtenir un nouveau QR code » revient à la première étape, puisque `commencer` peut être relancé tant que la fenêtre est ouverte ;
3. les **10 codes de secours, montrés une seule fois**. La case « J'ai noté mes 10 codes de secours en lieu sûr » est obligatoire avant « Revenir à la connexion ». Il n'y a ni copie, ni téléchargement, ni impression (B3). Aucune session n'est ouverte (§2.5).

Si le jeton est refusé (`jeton_invalide` : fenêtre expirée…), la page revient à la connexion avec « Le délai est dépassé. Reconnectez-vous. ».

**Le secret ne quitte jamais le navigateur (B5) :**
- le QR code est dessiné par `qrcode-generator` 2.0.4 (MIT, épinglée sans `^`, B4). Seule sa matrice sert (`addData`, `make`, `getModuleCount`, `isDark`), et elle devient un seul chemin SVG dans le gabarit : ni `innerHTML`, ni style en ligne, ni image `data:` ;
- niveau de correction M, marge de 4 modules (ISO/IEC 18004) ;
- la bibliothèque n'est chargée qu'avec la page de réenrôlement (morceau de 30,4 kB, 9,9 kB compressés) ;
- l'adresse `otpauth://`, la clé et les codes restent dans la page, en mémoire, et sont effacés dès qu'ils ne servent plus, puis quand on quitte la page ;
- des tests vérifient qu'aucun stockage du navigateur n'est écrit et que rien de tout cela n'apparaît dans la console, journal de développement compris.

**Palette dorée (B10)** : voir l'en-tête de `projects/administration/src/styles.css`.
- **Contrastes**, calculés avec la formule de WCAG 2.2, critère 1.4.3 : blanc sur `#7A5C00`, 6,25:1 ; blanc sur `#5E4700` (survol), 8,83:1 ; `#13201B` sur `#F7F0D9`, 14,74:1.
- **Focus** : il reste bleu `#2A5F94` (5,82:1 sur `#F7F0D9`).
- **Or vif `#D4AF37`** : décoratif seulement (trait sous l'en-tête, bord du panneau d'accueil). Jamais du texte, jamais le seul contour d'un élément d'interface (critère 1.4.11), jamais porteur d'une information.

**Essai du 2026-10-22** dans Chrome sans interface, sur la construction de production (CSP stricte) avec un faux backend. Parcours complet : réenrôlement, codes de secours, reconnexion avec second facteur, page des structures.
- **Résultat** : aucune violation de la CSP, rien dans le stockage du navigateur, et la clé n'est plus dans la page à la fin.
- **Console** : une recommandation de Chrome, pas un blocage. Sur l'étape du mot de passe redemandé, « Password forms should have (optionally hidden) username fields for accessibility ». L'écran ne connaît pas l'email : la session ne le garde pas.
- **QR code** : Chrome ne fournit pas `BarcodeDetector` sous Windows. Le décodage du QR se vérifie avec un téléphone, à l'essai manuel.
