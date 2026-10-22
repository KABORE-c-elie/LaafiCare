# LaafiCare : applications web

Ce dossier contient les applications web de LaafiCare, écrites avec Angular 22. Pour l'instant, une seule application : **Assurance MUNASEB**, pour les agents et les responsables de la mutuelle.

- `projects/commun` : ce que toutes les applications partagent (connexion, session, messages, boutons, champs…).
- `projects/munaseb` : l'application MUNASEB. Après la connexion, chaque espace s'ouvre selon le rôle : un **agent** traite les demandes de remboursement ; un **responsable** invite des personnes (agents ou autres responsables) et suit ses invitations.
- `docs/decisions.md` : les choix faits, et pourquoi.

## Ce qu'il faut installer

- **Node.js 24** (24.15 ou plus récent), qui fournit aussi `npm`. Vérifier : `node --version`.
- Le **backend** LaafiCare, lancé à part (voir le README à la racine du dépôt). Il doit répondre sur `http://127.0.0.1:8080`.

## Première installation

Dans un terminal PowerShell, depuis ce dossier `web` :

```powershell
npm ci
```

`npm ci` installe exactement les versions notées dans `package-lock.json`.

## Lancer l'application en développement

```powershell
npm start
```

Puis ouvrir `http://localhost:4200` dans le navigateur.

- La commande construit d'abord la bibliothèque `commun`, puis lance l'application.
- Les appels à `/api/...` sont transmis au backend sur `127.0.0.1:8080` (`proxy.conf.json`). Le backend n'a pas besoin d'autoriser une autre origine.
- **Recharger la page déconnecte** : c'est voulu, les jetons ne restent qu'en mémoire (`docs/decisions.md`).

## Tester l'application avec le backend (sans Postman)

1. Lancer le backend (README à la racine), puis `npm start` ici, et ouvrir `http://localhost:4200`.
2. Le serveur de développement n'écoute que sur `localhost` : ne jamais ajouter `--host` (une faille connue de son outil, vite 8.3.0, reste ouverte : Angular 22.2.4 ne la corrige pas ; voir `docs/decisions.md`).
3. Ouvrir les outils de développement du navigateur (**F12**) :
   - onglet **Console** : en développement seulement, chaque erreur de l'API écrit une ligne, par exemple `[API] 401 identifiants_invalides POST /api/professionnels/connexion`. Elle donne le code HTTP, le champ `code` et la route, jamais un jeton, un mot de passe, un code ou une donnée d'adhérent. Cette ligne n'existe pas dans la version de production ;
   - onglet **Réseau** : filtre **Fetch/XHR**, case **Conserver le journal**. Un clic sur une ligne montre **En-têtes** (méthode, adresse, code HTTP), **Charge utile** (ce qui a été envoyé) et **Réponse** (le JSON reçu, avec le champ `code` en cas d'erreur).
4. **Attention** : l'onglet Réseau montre l'en-tête `Authorization` et les mots de passe envoyés. Ne jamais en faire de capture d'écran ni le copier dans un message.

### Vérifier un email (lien reçu)

Un compte professionnel doit avoir vérifié son email avant de se connecter. En développement, le backend n'envoie pas l'email : il l'**affiche dans son terminal** (celui de `cargo run`), avec un lien de la forme `http://localhost:4200/verifier-email#jeton=…` (`URL_APPLICATION_WEB` dans `backend/.env`).

1. Copier ce lien dans le navigateur, avec `npm start` lancé ici. La partie après `#` disparaît aussitôt de l'adresse : c'est voulu.
2. Cliquer sur **« Confirmer mon adresse »**. Ouvrir le lien seul ne vérifie rien.
3. Ce qu'on doit voir : « Votre email est vérifié. Vous pouvez maintenant vous connecter. » Rouvrir le même lien affiche « Ce lien n'est plus valable ou a déjà servi. Demandez-en un nouveau. », avec un petit formulaire pour recevoir un nouveau lien.

Sur l'écran de connexion, un compte dont l'email n'est pas vérifié reçoit le message du serveur et un bouton **« Renvoyer le lien »**. Un second renvoi dans la minute est refusé : le bouton reste grisé, avec le temps restant.

### Inviter une personne (responsable)

Il faut une structure MUNASEB **validée** par l'équipe LaafiCare, et un compte responsable de cette structure.

1. Se connecter, puis choisir l'espace **« Responsable »** : la page **Invitations** s'ouvre.
2. Saisir un numéro **fictif** (par exemple `70 00 00 02`), choisir le rôle, puis **« Envoyer l'invitation »**. Pour le rôle « Responsable », une fenêtre demande de confirmer.
3. Ce qu'on doit voir : « Invitation envoyée. Valable jusqu'au JJ/MM/AAAA. », que le numéro ait un compte ou non. L'invitation apparaît dans la liste, « En attente ». En développement, le SMS s'affiche dans le terminal du backend.
4. **« Annuler »** sur une invitation en attente, puis confirmer : elle passe à « Annulée ».

Un agent n'ouvre que les demandes, un responsable que les invitations. Une adresse tapée à la main vers l'autre page ramène au choix de l'espace.

## Lancer les tests automatiques

```powershell
npm test
```

Lance les tests de `commun`, puis ceux de `munaseb` (Vitest). Ce qu'on doit voir : `Test Files … passed` pour chacun, aucun échec.

```powershell
npm run lint
```

Vérifie le code, y compris les règles d'accessibilité des gabarits (`angular-eslint`, configuration `templateAccessibility`). Ce qu'on doit voir : `All files pass linting.`

## Construire pour la production

```powershell
npm run build
```

Le résultat est dans `dist/munaseb/browser`. La taille de chaque fichier s'affiche à la fin.

## Essayer la version de production sur son ordinateur

```powershell
npm run build
npm run serve:prod
```

Puis ouvrir `http://127.0.0.1:8082`. Ce petit serveur (`outils/serveur-production.mjs`) envoie la politique de sécurité (CSP) prévue pour la production, compresse les fichiers et transmet `/api` au backend. Ce n'est **pas** un serveur de production.

- Le **service worker** ne fonctionne que dans cette version, pas avec `npm start`. Il ne sert jamais la page `/verifier-email` depuis son cache : elle vient toujours du serveur, dans sa dernière version.
- **Hors connexion** : après une première visite, couper le réseau (DevTools, onglet Réseau, « Hors connexion »), puis recharger. L'application s'ouvre et affiche « Pas de connexion. Vos demandes s'afficheront dès le retour du réseau. »

## Mesures (2026-10-20)

Écran de connexion, construction de production servie par `serveur-production.mjs`, Chrome 155 sans interface, une mesure par ligne. Réseau « Slow 4G » de DevTools : 1,6 Mbit/s × 0,9 en réception, 0,75 Mbit/s × 0,9 en envoi, latence 150 ms × 3,75.

| Situation | Téléchargé | Premier affichage | Écran utilisable | Mémoire JS |
|---|---|---|---|---|
| Slow 4G, cache vide | 99,5 Ko | 3,3 s | 3,2 s | 2,8 Mo |
| Slow 4G, cache rempli | 0 Ko (service worker) | 0,18 s | 0,15 s | — |
| Slow 4G + processeur ralenti 4×, cache vide | 99,5 Ko | 3,1 s | 3,1 s | — |
| Slow 4G + processeur ralenti 4×, cache rempli | 0 Ko | 0,18 s | 0,15 s | — |
| Hors connexion, après une visite | 0 Ko | 0,09 s | 0,09 s | — |

Lighthouse 13.5.0, mode « simulé » (`throttlingMethod: simulate`) :

| Réglage | Performance | Accessibilité | Bonnes pratiques |
|---|---|---|---|
| Mobile (RTT 150 ms, 1,6 Mbit/s, processeur 4×) | 99 | 100 | 100 |
| Ordinateur (`--preset=desktop`) | 100 | 100 | 100 |

Taille du chargement initial : 309,7 kB bruts, environ 85 kB compressés.
