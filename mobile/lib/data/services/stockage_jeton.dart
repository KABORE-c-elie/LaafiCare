import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Garde les jetons de la session patient dans le stockage sécurisé du
/// téléphone, jamais dans shared_preferences.
///
/// Aujourd'hui, un seul jeton : le jeton d'accès `laaficare:patient`
/// (docs/api.md §1.2). Le jeton de renouvellement, pas encore décrit dans
/// docs/api.md, s'ajoutera ici : une clé de plus dans [_clesSession] et ses
/// méthodes de lecture et d'écriture, sans rien changer au reste.
class StockageJeton {
  // Paramètre nommé privé (Dart ≥ 3.12) : l'appelant écrit stockage:.
  // https://dart.dev/language/constructors
  StockageJeton({this._stockage = _stockageParDefaut});

  final FlutterSecureStorage _stockage;

  static const _cleJetonAcces = 'jeton_acces';

  /// Toutes les clés de la session : [effacer] les supprime toutes. Une
  /// liste explicite plutôt que deleteAll(), pour ne jamais toucher aux
  /// autres secrets de l'application.
  static const _clesSession = [_cleJetonAcces];

  static const _stockageParDefaut = FlutterSecureStorage(
    // Android : clé AES-GCM enveloppée par une clé RSA du Keystore (défaut du
    // paquet : « Standard secure storage with RSA OAEP key wrapping »).
    // Espace séparé : storageNamespace isole « **all** storage artifacts »
    // (données, clés du Keystore). resetOnError, actif par défaut, « will
    // PERMANENTLY erase the data when an error occurs » : sans espace
    // séparé, une erreur sur la session effacerait aussi les autres secrets
    // de l'application (ex. future clé de la base des rappels).
    aOptions: AndroidOptions(storageNamespace: 'session'),
    // iOS : lisible seulement téléphone déverrouillé, et jamais copié vers un
    // autre appareil : « Items with this attribute do not migrate to a new
    // device » (KeychainAccessibility.unlocked_this_device).
    iOptions: IOSOptions(
      accessibility: KeychainAccessibility.unlocked_this_device,
    ),
  );

  /// Jeton d'accès enregistré, ou null s'il n'y en a pas.
  Future<String?> lireJetonAcces() => _lire(_cleJetonAcces);

  Future<void> enregistrerJetonAcces(String jeton) =>
      _ecrire(_cleJetonAcces, jeton);

  /// Fin de session (déconnexion, ou 401 jeton_invalide) : efface tous les
  /// jetons de la session. Décision du porteur : la déconnexion efface les
  /// jetons sur le téléphone.
  Future<void> effacer() async {
    for (final cle in _clesSession) {
      await _stockage.delete(key: cle);
    }
  }

  /// Une valeur illisible est traitée comme absente, puis effacée : la
  /// personne se reconnecte. Cas connu : restauration d'une sauvegarde
  /// Android, dont le README du paquet dit qu'elle « can cause exception
  /// java.security.InvalidKeyException: Failed to unwrap key ».
  /// https://pub.dev/packages/flutter_secure_storage
  Future<String?> _lire(String cle) async {
    try {
      return await _stockage.read(key: cle);
    } on PlatformException {
      try {
        await _stockage.delete(key: cle);
      } on PlatformException {
        // Rien de plus à faire : la valeur reste illisible, donc inutilisable.
      }
      return null;
    }
  }

  Future<void> _ecrire(String cle, String valeur) {
    if (valeur.isEmpty) {
      throw ArgumentError.value(valeur, cle, 'ne doit pas être vide');
    }
    return _stockage.write(key: cle, value: valeur);
  }
}
