import 'package:flutter/foundation.dart';

/// Réglages de l'application fixés à la compilation.
class Configuration {
  const Configuration._(this.adresseApi);

  /// Adresse de base de l'API LaafiCare (docs/api.md §1.1), sans barre
  /// finale : le client y ajoute directement /api/….
  final Uri adresseApi;

  /// Délai d'attente d'une requête : choix du projet pour un réseau mobile
  /// lent, pas une valeur tirée d'une source officielle. La doc de Dart ne
  /// fixe aucun délai par défaut (HttpClient.connectionTimeout vaut null) :
  /// https://api.dart.dev/dart-io/HttpClient/connectionTimeout.html
  static const Duration delaiAttente = Duration(seconds: 30);

  /// Adresse donnée au lancement, jamais écrite en dur :
  ///   flutter run --dart-define=API_URL=http://127.0.0.1:8080
  /// Aide de `flutter run` : « `--dart-define=<foo=bar>` Additional key-value
  /// pairs that will be available as constants from the
  /// String.fromEnvironment […] constructors ». Sans --dart-define, la valeur
  /// est la chaîne vide (doc de String.fromEnvironment :
  /// https://api.flutter.dev/flutter/dart-core/String/String.fromEnvironment.html).
  static const String _adresseApiCompilee = String.fromEnvironment('API_URL');

  /// Configuration de l'application en cours d'exécution.
  static Configuration charger() =>
      depuis(_adresseApiCompilee, versionPubliee: kReleaseMode);

  /// Contrôle une adresse ; séparé de [charger] pour être testé sans
  /// recompiler l'application.
  ///
  /// En version publiée, seul https est accepté. Les réglages Android
  /// (network_security_config) et iOS (App Transport Security) ne protègent
  /// pas ici : le paquet http passe par les connexions de Dart, et la doc
  /// Flutter précise « If the socket is owned by Dart/Flutter, no policy will
  /// be enforced. »
  /// https://docs.flutter.dev/release/breaking-changes/network-policy-ios-android
  /// kReleaseMode : https://api.flutter.dev/flutter/foundation/kReleaseMode-constant.html
  static Configuration depuis(String adresse, {required bool versionPubliee}) {
    if (adresse.isEmpty) {
      throw const ErreurConfiguration(
        "Adresse de l'API absente : lancez l'application avec "
        '--dart-define=API_URL=<adresse>.',
      );
    }
    final uri = Uri.tryParse(adresse);
    // La valeur de --dart-define est intégrée dans l'application compilée,
    // lisible par quiconque l'examine : aucun secret ne doit pouvoir y passer
    // (identifiant ou mot de passe dans l'adresse), ni de paramètre caché
    // ajouté à chaque requête (requête, fragment). Contrôlé en premier, et
    // l'adresse n'est pas reprise dans le message, pour ne jamais recopier
    // un secret dans une erreur.
    if (uri != null &&
        (uri.userInfo.isNotEmpty || uri.hasQuery || uri.hasFragment)) {
      throw const ErreurConfiguration(
        "Adresse de l'API invalide : pas d'identifiant, de requête ni de "
        'fragment.',
      );
    }
    if (uri == null ||
        !(uri.isScheme('http') || uri.isScheme('https')) ||
        uri.host.isEmpty) {
      throw ErreurConfiguration(
        "Adresse de l'API invalide : « $adresse ». "
        'Forme attendue : http://hote:port ou https://hote.',
      );
    }
    if (versionPubliee && !uri.isScheme('https')) {
      throw ErreurConfiguration(
        "La version publiée exige une adresse en https (reçu : « $adresse »).",
      );
    }
    // Barre finale retirée, pour que le client ajoute /api/… sans obtenir
    // //api. Uri.replace : « Creates a new Uri based on this one, but with
    // some parts replaced. » https://api.dart.dev/dart-core/Uri/replace.html
    var chemin = uri.path;
    while (chemin.endsWith('/')) {
      chemin = chemin.substring(0, chemin.length - 1);
    }
    return Configuration._(uri.replace(path: chemin));
  }
}

/// Configuration inutilisable : l'application ne doit pas démarrer.
class ErreurConfiguration implements Exception {
  const ErreurConfiguration(this.message);

  final String message;

  @override
  String toString() => 'ErreurConfiguration : $message';
}
