import 'package:flutter/widgets.dart';
import 'package:flutter_svg/flutter_svg.dart';

/// Variantes du logo, déclarées une par une dans pubspec.yaml.
enum VarianteLogo {
  /// Écran de connexion (sur fond clair).
  horizontal('assets/branding/logo_horizontal.svg'),

  /// Écran de démarrage (sur le fond vert de la marque).
  verticalSurVert('assets/branding/logo_vertical_sur_vert.svg');

  const VarianteLogo(this.chemin);
  final String chemin;
}

/// Logo LaafiCare en SVG (flutter_svg) : jamais en PNG dans l'interface,
/// pour rester net à toutes les tailles.
class LogoLaafi extends StatelessWidget {
  const LogoLaafi({
    super.key,
    this.variante = VarianteLogo.horizontal,
    this.hauteur = 40,
    required this.libelle,
  });

  final VarianteLogo variante;

  /// Hauteur 40 à la connexion (maquette du pack).
  final double hauteur;

  /// Nom lu par le lecteur d'écran (« LaafiCare », texte traduit) : le mot
  /// est dessiné en tracés dans le SVG, sans texte lisible par la machine.
  final String libelle;

  @override
  Widget build(BuildContext context) => SvgPicture.asset(
        variante.chemin,
        height: hauteur,
        // flutter_svg 2.3.0 : paramètre semanticsLabel de SvgPicture.asset.
        semanticsLabel: libelle,
      );
}
