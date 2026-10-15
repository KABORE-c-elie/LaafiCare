import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../theme/theme_laaficare.dart';

/// Page qui s'adapte à toutes les tailles d'écran : petit téléphone,
/// paysage, tablette, pliable, texte agrandi, clavier ouvert.
///
/// - Défile quand le contenu dépasse (jamais de débordement).
/// - Fait au moins la hauteur de l'écran : [contenu] en haut, [bas] collé en
///   bas quand il y a de la place. Technique de la doc du SDK pour
///   SingleChildScrollView, « Centering, spacing, or aligning fixed-height
///   content » : la colonne « becomes the bigger of the minimum height
///   provided by the [ConstrainedBox] and the sum of the heights of the
///   children ». Sans IntrinsicHeight, que la même doc dit « quite
///   expensive ».
/// - Largeur limitée et centrée sur grand écran
///   ([Espacements.largeurMaxContenu]).
/// - Mise en page d'après la place reçue (LayoutBuilder), jamais d'après le
///   type d'appareil (doc Flutter, « Adaptive and responsive design » :
///   « your choice shouldn't depend on the type of device, but on the
///   device's available window size »).
class PageAdaptable extends StatelessWidget {
  const PageAdaptable({super.key, required this.contenu, this.bas});

  final Widget contenu;

  /// Bloc collé en bas de l'écran s'il y a de la place (ex. carte des
  /// numéros d'urgence).
  final Widget? bas;

  @override
  Widget build(BuildContext context) => SafeArea(
        child: LayoutBuilder(
          builder: (context, contraintes) => SingleChildScrollView(
            padding: const EdgeInsets.all(Espacements.marge),
            child: Align(
              alignment: Alignment.topCenter,
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  maxWidth: Espacements.largeurMaxContenu,
                  minHeight: math.max(
                      0, contraintes.maxHeight - 2 * Espacements.marge),
                ),
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    contenu,
                    if (bas != null)
                      Padding(
                        padding: const EdgeInsets.only(
                            top: Espacements.entreSections),
                        child: bas,
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
}
