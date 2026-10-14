# LaafiCare — pack logo et icônes (piste A, « Pousse »)

## Contenu

| Dossier | Quoi |
| --- | --- |
| `frontend-mobile/` | À copier tel quel à la racine du dossier `frontend-mobile` (Flutter) |
| `svg/` | Logos et symbole vectoriels, texte converti en tracés (aucune police nécessaire) |
| `png/` | Les mêmes en PNG haute définition (documents, réseaux sociaux, dossier de concours) |
| `web/` | Favicons et icônes pour les applications Angular (`src/favicon.ico`, `src/assets/`) |
| `OFL.txt` | Licence de la police Lexend utilisée pour le mot « LaafiCare » |

## Couleurs (système de design LaafiCare Patient)

- `primaire` #0A6650 — fond de l'icône, « Laafi »
- `marque` #0E8466 — feuille droite sur fond clair
- `menthe` #A6F0D5 — feuille droite sur fond vert
- `menthe-douce` #E3FAF1 — « Care » sur fond vert
- `encre` #13201B — « Care » sur fond clair, version une couleur

Jamais de rouge, d'orange ni de bleu dans le logo : ces couleurs sont réservées (alertes de santé, vigilance, actions « Appeler »).

## Consigne pour la session Claude Code mobile

À coller dans la session Claude Code du front-end mobile, sur la branche mobile :

> Intègre l'identité visuelle LaafiCare dans `frontend-mobile` :
> 1. Les fichiers `assets/branding/*`, `flutter_launcher_icons.yaml`, `flutter_native_splash.yaml` et `lib/core/widgets/laafi_logo.dart` sont déjà copiés. Ne les redessine pas et ne change pas leurs couleurs.
> 2. Ajoute les dépendances : `flutter pub add flutter_svg` puis `flutter pub add dev:flutter_launcher_icons dev:flutter_native_splash`.
> 3. Déclare `assets/branding/` dans la section `flutter: assets:` de `pubspec.yaml`.
> 4. Génère les icônes : `dart run flutter_launcher_icons`, puis l'écran de démarrage : `dart run flutter_native_splash:create`.
> 5. Utilise le widget `LaafiLogo` partout où le logo apparaît (écran de connexion : `LaafiLogoVariante.horizontal`, hauteur 40 ; en-têtes sur fond vert : `horizontalSurVert`). Ne mets pas le logo en image PNG dans l'interface.
> 6. Mets le nom affiché de l'application à « LaafiCare » : `android:label` dans `AndroidManifest.xml` et `CFBundleDisplayName` dans `ios/Runner/Info.plist`.
> 7. Pas de commentaires dans le code. Pas de Claude comme co-auteur dans les commits.
> 8. Vérifie avec `flutter analyze`, puis lance l'application sur un émulateur Android 13 ou plus pour contrôler l'icône (ronde, carrée, et en mode thème monochrome) et l'écran de démarrage.

## Tailles

- `icon.png` : 1024 × 1024, sans transparence (iOS et ancienne icône Android)
- `icon_foreground.png` / `icon_monochrome.png` : 1024 × 1024, symbole dans la zone sûre de l'icône adaptative Android
- `splash_android12.png` : 1152 × 1152 (288 dp), symbole dans le cercle visible d'Android 12+
- `splash_logo.png` : logo vertical blanc pour l'écran de démarrage avant Android 12 et sur iOS
