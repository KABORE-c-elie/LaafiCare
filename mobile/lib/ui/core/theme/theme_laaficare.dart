import 'package:flutter/material.dart';

/// Thème de l'application patient, d'après le système de design
/// « LaafiCare Patient » (maquettes validées par le porteur), inspiré du
/// projet MediGuide et réécrit pour LaafiCare. Thème clair seulement
/// (décision du porteur) ; police du système (aucune fontFamily).
///
/// Règle commune à toutes les applications (CLAUDE.md §6, règles non
/// négociables) : le rouge est réservé aux alertes de santé, l'orange à la
/// vigilance, et les erreurs de saisie prennent une couleur neutre
/// (colorScheme.error ci-dessous). La couleur ne porte jamais seule
/// l'information : chaque erreur, alerte ou vigilance a aussi une icône et
/// un texte (WCAG 2.2, critère 1.4.1). Un errorText seul sous un champ
/// s'affiche sans icône : les écrans utilisent un message avec icône et
/// texte, dans le bloc errorContainer des maquettes.
abstract final class ThemeLaafiCare {
  static const schema = ColorScheme.light(
    primary: Color(0xFF0A6650),
    onPrimary: Color(0xFFFFFFFF),
    primaryContainer: Color(0xFFE3FAF1),
    onPrimaryContainer: Color(0xFF0A6650),
    // Boutons « Appeler », y compris les numéros d'urgence : jamais le
    // rouge (décision du porteur).
    secondary: Color(0xFF2A5F94),
    onSecondary: Color(0xFFFFFFFF),
    surface: Color(0xFFFFFFFF),
    onSurface: Color(0xFF13201B),
    onSurfaceVariant: Color(0xFF45524C),
    surfaceContainerLow: Color(0xFFEEF4F1),
    surfaceContainerHighest: Color(0xFFE1EBE6),
    // Bord des champs : au moins 3:1 (WCAG 2.2, 1.4.11) sur les trois fonds
    // où un champ peut être posé : 3,44 sur surface, 3,27 sur le fond des
    // écrans, 3,08 sur surfaceContainerLow. Corrigé par le porteur : la
    // valeur des maquettes (#8A9690) ne donnait que 2,92:1 sur le fond.
    outline: Color(0xFF828D87),
    // Séparateurs seulement.
    outlineVariant: Color(0xFFC5CEC9),
    // Couleur NEUTRE des erreurs de saisie, jamais du rouge (CLAUDE.md §6).
    error: Color(0xFF2F3E4C),
    onError: Color(0xFFFFFFFF),
    errorContainer: Color(0xFFEDF1F4),
    onErrorContainer: Color(0xFF2F3E4C),
  );

  /// Fond des écrans.
  static const fond = Color(0xFFF7FAF8);

  static ThemeData get clair {
    final base = ThemeData(colorScheme: schema);
    final texte = base.textTheme;
    return base.copyWith(
      scaffoldBackgroundColor: fond,
      textTheme: texte.copyWith(
        // Numéro d'urgence : 32/40, 700.
        headlineLarge: _style(texte.headlineLarge, 32, 40, FontWeight.w700),
        // Titre d'écran : 24/32, 600.
        headlineSmall: _style(texte.headlineSmall, 24, 32, FontWeight.w600),
        // Titre de section : 20/28, 600.
        titleLarge: _style(texte.titleLarge, 20, 28, FontWeight.w600),
        // Corps : 16/24, 400. bodyMedium aussi, car c'est le style par
        // défaut d'un Text dans Material (sinon le corps serait en 14).
        bodyLarge: _style(texte.bodyLarge, 16, 24, FontWeight.w400),
        bodyMedium: _style(texte.bodyMedium, 16, 24, FontWeight.w400),
        // Corps petit : 14/20, 400.
        bodySmall: _style(texte.bodySmall, 14, 20, FontWeight.w400),
        // Libellé et bouton : 16/24, 600 (labelLarge est le style des
        // boutons Material).
        labelLarge: _style(texte.labelLarge, 16, 24, FontWeight.w600),
        // Légende : 12/16, 500.
        labelMedium: _style(texte.labelMedium, 12, 16, FontWeight.w500),
      ),
      // Zone touchable de 48 px au moins. SDK Flutter : « Expands the
      // minimum tap target size to 48px by 48px. This is the default value
      // […] and the recommended size to conform to Android accessibility
      // scanner recommendations. » Écrit explicitement pour ne pas en
      // dépendre.
      materialTapTargetSize: MaterialTapTargetSize.padded,
      // Forme seulement : FilledButton et FilledButton.tonal lisent le même
      // thème (SDK, filled_button.dart, themeStyleOf). Les couleurs viennent
      // du ColorScheme (primary pour le bouton plein), et de
      // StylesBoutons.secondaire pour le bouton secondaire.
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          minimumSize: const Size.fromHeight(Espacements.hauteurBouton),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(Espacements.rayonBouton),
          ),
          textStyle: _style(texte.labelLarge, 16, 24, FontWeight.w600),
        ),
      ),
      // Champs : fond blanc, bord 1 px outline, focus 2 px primary, erreur
      // 2 px error (neutre). Hauteur 56 et marge de 16 à gauche et à droite
      // (maquettes) : 24 de texte + 2 × 16.
      inputDecorationTheme: InputDecorationThemeData(
        filled: true,
        fillColor: schema.surface,
        contentPadding: const EdgeInsets.symmetric(
          horizontal: Espacements.marge,
          vertical: 16,
        ),
        constraints:
            const BoxConstraints(minHeight: Espacements.hauteurChamp),
        enabledBorder: _bord(schema.outline, 1),
        border: _bord(schema.outline, 1),
        focusedBorder: _bord(schema.primary, 2),
        errorBorder: _bord(schema.error, 2),
        focusedErrorBorder: _bord(schema.error, 2),
      ),
      // Cartes : élévation 1 et ombre noire à 8 % (choix du porteur pour
      // « ombre très légère »), arrondi 16.
      cardTheme: CardThemeData(
        color: schema.surface,
        elevation: 1,
        shadowColor: Colors.black.withValues(alpha: 0.08),
        surfaceTintColor: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(Espacements.rayonCarte),
        ),
      ),
      // Puces en pilule.
      chipTheme: const ChipThemeData(shape: StadiumBorder()),
      dividerTheme: DividerThemeData(color: schema.outlineVariant),
      extensions: const [CouleursLaafiCare.clair],
    );
  }

  static TextStyle _style(
    TextStyle? base,
    double taille,
    double interligne,
    FontWeight graisse,
  ) =>
      (base ?? const TextStyle()).copyWith(
        fontSize: taille,
        // Interligne exprimé en multiple de la taille (doc de
        // TextStyle.height).
        height: interligne / taille,
        fontWeight: graisse,
      );

  static OutlineInputBorder _bord(Color couleur, double epaisseur) =>
      OutlineInputBorder(
        borderRadius: BorderRadius.circular(Espacements.rayonChamp),
        borderSide: BorderSide(color: couleur, width: epaisseur),
      );
}

/// Bouton secondaire (« Créer mon compte ») : fond #E1EBE6, texte primaire,
/// sur un FilledButton.tonal (option b du porteur, sans ajouter de rôle au
/// ColorScheme).
abstract final class StylesBoutons {
  static ButtonStyle secondaire(BuildContext context) {
    final schema = Theme.of(context).colorScheme;
    return FilledButton.styleFrom(
      backgroundColor: schema.surfaceContainerHighest,
      foregroundColor: schema.primary,
    );
  }
}

/// Espacements et formes des maquettes.
abstract final class Espacements {
  static const marge = 16.0;
  static const entreSections = 24.0;
  static const zoneTouchable = 48.0;
  static const hauteurBouton = 52.0;
  static const hauteurChamp = 56.0;
  static const rayonBouton = 12.0;
  static const rayonChamp = 16.0;
  static const rayonCarte = 16.0;

  /// Marge intérieure du bloc de message d'erreur (maquettes).
  static const paddingMessage = 12.0;
}

/// Couleurs hors ColorScheme (ThemeExtension, doc :
/// https://api.flutter.dev/flutter/material/ThemeExtension-class.html).
/// Lues par `Theme.of(context).extension<CouleursLaafiCare>()`.
class CouleursLaafiCare extends ThemeExtension<CouleursLaafiCare> {
  const CouleursLaafiCare({
    required this.marque,
    required this.menthe,
    required this.alerte,
    required this.alerteFond,
    required this.surAlerteFond,
    required this.vigilance,
    required this.vigilanceFond,
    required this.surVigilanceFond,
  });

  /// Démarrage et logo.
  final Color marque;

  /// Pastilles d'information positive.
  final Color menthe;

  /// Réservées aux ALERTES DE SANTÉ (allergies, tension ≥ 140 mmHg, signes
  /// de danger…), jamais à un bouton, à la marque ou à une erreur de saisie
  /// (CLAUDE.md §6). Toujours avec une icône et un texte.
  final Color alerte;
  final Color alerteFond;
  final Color surAlerteFond;

  /// Réservées aux valeurs de santé à RISQUE MODÉRÉ (vigilance), jamais à
  /// autre chose (CLAUDE.md §6). Toujours avec une icône et un texte.
  final Color vigilance;
  final Color vigilanceFond;
  final Color surVigilanceFond;

  static const clair = CouleursLaafiCare(
    marque: Color(0xFF0E8466),
    menthe: Color(0xFFA6F0D5),
    alerte: Color(0xFFB3261E),
    alerteFond: Color(0xFFFCE4E1),
    surAlerteFond: Color(0xFF5C0F0A),
    vigilance: Color(0xFF9A4A06),
    vigilanceFond: Color(0xFFFEF0DC),
    surVigilanceFond: Color(0xFF5A2E04),
  );

  @override
  CouleursLaafiCare copyWith({
    Color? marque,
    Color? menthe,
    Color? alerte,
    Color? alerteFond,
    Color? surAlerteFond,
    Color? vigilance,
    Color? vigilanceFond,
    Color? surVigilanceFond,
  }) =>
      CouleursLaafiCare(
        marque: marque ?? this.marque,
        menthe: menthe ?? this.menthe,
        alerte: alerte ?? this.alerte,
        alerteFond: alerteFond ?? this.alerteFond,
        surAlerteFond: surAlerteFond ?? this.surAlerteFond,
        vigilance: vigilance ?? this.vigilance,
        vigilanceFond: vigilanceFond ?? this.vigilanceFond,
        surVigilanceFond: surVigilanceFond ?? this.surVigilanceFond,
      );

  @override
  CouleursLaafiCare lerp(CouleursLaafiCare? other, double t) {
    if (other == null) return this;
    return CouleursLaafiCare(
      marque: Color.lerp(marque, other.marque, t)!,
      menthe: Color.lerp(menthe, other.menthe, t)!,
      alerte: Color.lerp(alerte, other.alerte, t)!,
      alerteFond: Color.lerp(alerteFond, other.alerteFond, t)!,
      surAlerteFond: Color.lerp(surAlerteFond, other.surAlerteFond, t)!,
      vigilance: Color.lerp(vigilance, other.vigilance, t)!,
      vigilanceFond: Color.lerp(vigilanceFond, other.vigilanceFond, t)!,
      surVigilanceFond:
          Color.lerp(surVigilanceFond, other.surVigilanceFond, t)!,
    );
  }
}
