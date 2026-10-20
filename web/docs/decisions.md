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
- **Pas de favicon** : aucune icône web n'est encore dans le dépôt. Celle d'Angular a été retirée.
