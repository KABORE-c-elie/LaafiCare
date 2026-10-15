import 'package:flutter/material.dart';

import '../theme/theme_laaficare.dart';

/// Message d'erreur de saisie ou de réseau, sous un champ ou un formulaire :
/// bloc errorContainer, icône « info » et texte (maquettes).
///
/// Jamais InputDecoration.errorText seul : il s'affiche sans icône, et la
/// couleur porterait seule l'information (WCAG 2.2, 1.4.1 ; CLAUDE.md §6).
/// Couleur neutre (colorScheme.error), jamais celle des alertes de santé.
class MessageErreur extends StatelessWidget {
  const MessageErreur(this.texte, {super.key});

  /// Texte déjà traduit, choisi d'après le `code` de l'API, jamais le
  /// message du serveur (docs/api.md §1.3).
  final String texte;

  @override
  Widget build(BuildContext context) {
    final schema = Theme.of(context).colorScheme;
    // Annoncé par le lecteur d'écran dès qu'il apparaît. SDK, Semantics.
    // liveRegion : « On Android and iOS, live region causes a polite
    // announcement to be generated automatically, even if the widget does
    // not have accessibility focus. »
    return Semantics(
      liveRegion: true,
      child: Container(
        padding: const EdgeInsets.all(Espacements.paddingMessage),
        decoration: BoxDecoration(
          color: schema.errorContainer,
          borderRadius: BorderRadius.circular(Espacements.rayonBouton),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Décorative : le texte porte le sens, l'icône n'est pas lue.
            ExcludeSemantics(
              child: Icon(Icons.info_outline,
                  color: schema.onErrorContainer, size: 20),
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                texte,
                style: Theme.of(context)
                    .textTheme
                    .bodySmall
                    ?.copyWith(color: schema.onErrorContainer),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
