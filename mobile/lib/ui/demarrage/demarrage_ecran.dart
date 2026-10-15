import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../l10n/app_localizations.dart';
import '../core/theme/theme_laaficare.dart';
import '../core/widgets/logo_laafi.dart';

/// Écran de démarrage PROVISOIRE, affiché pendant la lecture de la session :
/// fond de la marque et logo vertical, sans animation. Il sera remplacé par
/// l'écran animé (étape de l'écran de démarrage, d'après EventHub).
class DemarrageEcran extends StatelessWidget {
  const DemarrageEcran({super.key});

  @override
  Widget build(BuildContext context) => Scaffold(
        backgroundColor:
            Theme.of(context).extension<CouleursLaafiCare>()!.marque,
        // Logo de 120 au plus, réduit au tiers de la hauteur disponible sur
        // un écran très bas (téléphone en paysage) : jamais de débordement.
        body: SafeArea(
          child: LayoutBuilder(
            builder: (context, contraintes) => Center(
              child: LogoLaafi(
                variante: VarianteLogo.verticalSurVert,
                hauteur: math.min(120, contraintes.maxHeight / 3),
                libelle: Textes.of(context).nomApplication,
              ),
            ),
          ),
        ),
      );
}
