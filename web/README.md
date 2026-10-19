# LaafiCare : applications web

Ce dossier contient les applications web de LaafiCare, écrites avec Angular 22. Pour l'instant, une seule application : **Assurance MUNASEB**, pour les agents de la mutuelle.

- `projects/commun` : ce que toutes les applications partagent (connexion, session, messages, boutons, champs…).
- `projects/munaseb` : l'application des agents MUNASEB.
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

## Tester

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
