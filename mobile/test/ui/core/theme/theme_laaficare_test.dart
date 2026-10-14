import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patient/ui/core/theme/theme_laaficare.dart';

/// Rapport de contraste WCAG : (L1 + 0,05) / (L2 + 0,05), L1 la plus claire.
/// Luminance relative calculée par Color.computeLuminance.
double _contraste(Color a, Color b) {
  final la = a.computeLuminance();
  final lb = b.computeLuminance();
  final (claire, sombre) = la > lb ? (la, lb) : (lb, la);
  return (claire + 0.05) / (sombre + 0.05);
}

void main() {
  final theme = ThemeLaafiCare.clair;
  final schema = theme.colorScheme;
  final sante = theme.extension<CouleursLaafiCare>()!;

  test('1. couleurs du ColorScheme et de CouleursLaafiCare', () {
    expect(schema.primary, const Color(0xFF0A6650));
    expect(schema.onPrimary, const Color(0xFFFFFFFF));
    expect(schema.primaryContainer, const Color(0xFFE3FAF1));
    expect(schema.onPrimaryContainer, const Color(0xFF0A6650));
    expect(schema.secondary, const Color(0xFF2A5F94));
    expect(schema.onSecondary, const Color(0xFFFFFFFF));
    expect(schema.surface, const Color(0xFFFFFFFF));
    expect(schema.onSurface, const Color(0xFF13201B));
    expect(schema.onSurfaceVariant, const Color(0xFF45524C));
    expect(schema.surfaceContainerLow, const Color(0xFFEEF4F1));
    expect(schema.surfaceContainerHighest, const Color(0xFFE1EBE6));
    expect(schema.outline, const Color(0xFF828D87));
    expect(schema.outlineVariant, const Color(0xFFC5CEC9));
    expect(schema.error, const Color(0xFF2F3E4C));
    expect(schema.onError, const Color(0xFFFFFFFF));
    expect(schema.errorContainer, const Color(0xFFEDF1F4));
    expect(schema.onErrorContainer, const Color(0xFF2F3E4C));
    expect(theme.scaffoldBackgroundColor, const Color(0xFFF7FAF8));

    expect(sante.marque, const Color(0xFF0E8466));
    expect(sante.menthe, const Color(0xFFA6F0D5));
    expect(sante.alerte, const Color(0xFFB3261E));
    expect(sante.alerteFond, const Color(0xFFFCE4E1));
    expect(sante.surAlerteFond, const Color(0xFF5C0F0A));
    expect(sante.vigilance, const Color(0xFF9A4A06));
    expect(sante.vigilanceFond, const Color(0xFFFEF0DC));
    expect(sante.surVigilanceFond, const Color(0xFF5A2E04));
  });

  test("2. l'erreur de saisie n'est ni l'alerte ni la vigilance "
      '(CLAUDE.md §6)', () {
    for (final reservee in [
      sante.alerte,
      sante.alerteFond,
      sante.vigilance,
      sante.vigilanceFond,
    ]) {
      expect(schema.error, isNot(reservee));
      expect(schema.errorContainer, isNot(reservee));
    }
  });

  test('3. contrastes WCAG 2.2 : texte 4,5:1 (1.4.3), bord 3:1 (1.4.11)',
      () {
    final fond = theme.scaffoldBackgroundColor;
    final texte = <(String, Color, Color)>[
      ('onSurface / surface', schema.onSurface, schema.surface),
      ('onPrimary / primary', schema.onPrimary, schema.primary),
      ('onSecondary / secondary', schema.onSecondary, schema.secondary),
      ('onError / error', schema.onError, schema.error),
      ('onErrorContainer / errorContainer', schema.onErrorContainer,
          schema.errorContainer),
      ('surAlerteFond / alerteFond', sante.surAlerteFond, sante.alerteFond),
      ('surVigilanceFond / vigilanceFond', sante.surVigilanceFond,
          sante.vigilanceFond),
      ('onSurfaceVariant / fond', schema.onSurfaceVariant, fond),
      ('onSurfaceVariant / surface', schema.onSurfaceVariant, schema.surface),
      ('primary / bouton secondaire', schema.primary,
          schema.surfaceContainerHighest),
      ('onPrimaryContainer / primaryContainer', schema.onPrimaryContainer,
          schema.primaryContainer),
      ('error / surface (texte sous un champ)', schema.error, schema.surface),
    ];
    for (final (nom, avant, arriere) in texte) {
      expect(_contraste(avant, arriere), greaterThanOrEqualTo(4.5),
          reason: nom);
    }
    // Bord des champs, contre les trois fonds où un champ peut être posé
    // (correction du porteur).
    for (final (nom, arriere) in [
      ('surface', schema.surface),
      ('fond des écrans', fond),
      ('surfaceContainerLow', schema.surfaceContainerLow),
    ]) {
      expect(_contraste(schema.outline, arriere), greaterThanOrEqualTo(3.0),
          reason: 'outline / $nom');
    }
  });

  test('4. styles de texte : taille, interligne, graisse', () {
    final t = theme.textTheme;
    for (final (nom, style, taille, interligne, graisse) in [
      ('headlineLarge', t.headlineLarge, 32.0, 40.0, FontWeight.w700),
      ('headlineSmall', t.headlineSmall, 24.0, 32.0, FontWeight.w600),
      ('titleLarge', t.titleLarge, 20.0, 28.0, FontWeight.w600),
      ('bodyLarge', t.bodyLarge, 16.0, 24.0, FontWeight.w400),
      ('bodyMedium', t.bodyMedium, 16.0, 24.0, FontWeight.w400),
      ('bodySmall', t.bodySmall, 14.0, 20.0, FontWeight.w400),
      ('labelLarge', t.labelLarge, 16.0, 24.0, FontWeight.w600),
      ('labelMedium', t.labelMedium, 12.0, 16.0, FontWeight.w500),
    ]) {
      expect(style!.fontSize, taille, reason: nom);
      expect(style.height! * taille, closeTo(interligne, 0.001), reason: nom);
      expect(style.fontWeight, graisse, reason: nom);
    }
  });

  testWidgets('5. boutons : hauteur 52, arrondi 12 ; bouton secondaire '
      '#E1EBE6 et texte primaire', (tester) async {
    final style = theme.filledButtonTheme.style!;
    expect(style.minimumSize!.resolve({})!.height, 52);
    final forme = style.shape!.resolve({})! as RoundedRectangleBorder;
    expect(forme.borderRadius, BorderRadius.circular(12));

    late ButtonStyle secondaire;
    await tester.pumpWidget(MaterialApp(
      theme: theme,
      home: Builder(builder: (context) {
        secondaire = StylesBoutons.secondaire(context);
        return const SizedBox();
      }),
    ));
    expect(secondaire.backgroundColor!.resolve({}), const Color(0xFFE1EBE6));
    expect(secondaire.foregroundColor!.resolve({}), const Color(0xFF0A6650));
  });

  test('6. champs : bords au repos, au focus, en erreur ; hauteur 56', () {
    final champ = theme.inputDecorationTheme;
    expect(champ.filled, isTrue);
    expect(champ.fillColor, const Color(0xFFFFFFFF));
    expect(champ.constraints!.minHeight, 56);
    expect(champ.contentPadding,
        const EdgeInsets.symmetric(horizontal: 16, vertical: 16));
    for (final (nom, bord, couleur, epaisseur) in [
      ('repos', champ.enabledBorder, const Color(0xFF828D87), 1.0),
      ('focus', champ.focusedBorder, const Color(0xFF0A6650), 2.0),
      ('erreur', champ.errorBorder, const Color(0xFF2F3E4C), 2.0),
      ('erreur au focus', champ.focusedErrorBorder, const Color(0xFF2F3E4C),
          2.0),
    ]) {
      final contour = bord! as OutlineInputBorder;
      expect(contour.borderSide.color, couleur, reason: nom);
      expect(contour.borderSide.width, epaisseur, reason: nom);
      expect(contour.borderRadius, BorderRadius.circular(16), reason: nom);
    }
  });

  test('7. cartes : élévation 1, arrondi 16 ; zone touchable padded', () {
    expect(theme.cardTheme.elevation, 1);
    final forme = theme.cardTheme.shape! as RoundedRectangleBorder;
    expect(forme.borderRadius, BorderRadius.circular(16));
    expect(theme.materialTapTargetSize, MaterialTapTargetSize.padded);
  });
}
