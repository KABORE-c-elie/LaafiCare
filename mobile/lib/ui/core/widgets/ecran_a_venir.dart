import 'package:flutter/material.dart';

import '../../../l10n/app_localizations.dart';
import '../theme/theme_laaficare.dart';
import 'page_adaptable.dart';

/// Écran en attente de sa maquette. Empilé (context.push), il a une flèche
/// retour : l'AppBar l'ajoute d'elle-même quand la page peut être fermée
/// (AppBar.automaticallyImplyLeading, vrai par défaut).
class EcranAVenir extends StatelessWidget {
  const EcranAVenir({super.key, required this.titre, this.action});

  final String titre;

  /// Bouton facultatif sous le texte (ex. « Se déconnecter » sur l'accueil
  /// provisoire).
  final Widget? action;

  @override
  Widget build(BuildContext context) => Scaffold(
        appBar: AppBar(title: Text(titre)),
        // Toutes tailles d'écran (PageAdaptable).
        body: PageAdaptable(
          contenu: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(Textes.of(context).ecranAVenir),
              if (action != null) ...[
                const SizedBox(height: Espacements.entreSections),
                action!,
              ],
            ],
          ),
        ),
      );
}
