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

## 7. Palette de l'application MUNASEB

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
