import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

enum LaafiLogoVariante { horizontal, horizontalSurVert, symbole }

class LaafiLogo extends StatelessWidget {
  const LaafiLogo({
    super.key,
    this.variante = LaafiLogoVariante.horizontal,
    this.hauteur = 40,
  });

  final LaafiLogoVariante variante;
  final double hauteur;

  String get _chemin => switch (variante) {
        LaafiLogoVariante.horizontal => 'assets/branding/logo_horizontal.svg',
        LaafiLogoVariante.horizontalSurVert =>
          'assets/branding/logo_horizontal_sur_vert.svg',
        LaafiLogoVariante.symbole => 'assets/branding/symbole.svg',
      };

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: 'LaafiCare',
      image: true,
      child: SvgPicture.asset(_chemin, height: hauteur),
    );
  }
}
