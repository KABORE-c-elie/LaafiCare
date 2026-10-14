import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Garde les jetons de la session patient dans le stockage sécurisé du
/// téléphone, jamais dans shared_preferences : le jeton d'accès
/// (`laaficare:patient`, 15 minutes) et le jeton de renouvellement
/// (docs/api.md §1.7 : « garder le jeton de renouvellement dans le stockage
/// sécurisé du téléphone »). Aucun jeton n'est jamais écrit dans un journal.
class StockageJeton {
  // Paramètre nommé privé (Dart ≥ 3.12) : l'appelant écrit stockage:.
  // https://dart.dev/language/constructors
  StockageJeton({this._stockage = _stockageParDefaut});

  final FlutterSecureStorage _stockage;

  static const _cleJetonAcces = 'jeton_acces';
  static const _cleJetonRenouvellement = 'jeton_renouvellement';

  /// Toutes les clés de la session : [effacer] les supprime toutes. Une
  /// liste explicite plutôt que deleteAll(), pour ne jamais toucher aux
  /// autres secrets de l'application.
  static const _clesSession = [_cleJetonAcces, _cleJetonRenouvellement];

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

  /// Jeton de renouvellement enregistré, ou null s'il n'y en a pas.
  Future<String?> lireJetonRenouvellement() => _lire(_cleJetonRenouvellement);

  /// Enregistre la paire reçue à la connexion, à l'inscription ou au
  /// renouvellement (docs/api.md §1.7 : « toujours remplacer les deux
  /// jetons »).
  ///
  /// Les deux jetons sont contrôlés avant toute écriture : une paire n'est
  /// jamais enregistrée à moitié par erreur de programmation.
  ///
  /// Le jeton de renouvellement est écrit **d'abord** (choix du porteur,
  /// C1). Le stockage n'a pas de transaction : si l'application s'arrête
  /// entre les deux écritures, il reste le nouveau jeton de renouvellement
  /// et l'ancien jeton d'accès, sans danger (le prochain 401 renouvelle avec
  /// le bon jeton). Dans l'ordre inverse, l'ancien jeton de renouvellement
  /// resterait : renvoyé au serveur, il serait vu comme un rejeu, et toute
  /// la session serait fermée (docs/api.md §1.7).
  Future<void> enregistrerPaire({
    required String jetonAcces,
    required String jetonRenouvellement,
  }) async {
    _exigerNonVide(_cleJetonAcces, jetonAcces);
    _exigerNonVide(_cleJetonRenouvellement, jetonRenouvellement);
    await _stockage.write(
        key: _cleJetonRenouvellement, value: jetonRenouvellement);
    await _stockage.write(key: _cleJetonAcces, value: jetonAcces);
  }

  /// Ancienne écriture du seul jeton d'accès : retirée au commit 5, quand
  /// AuthRepository passe à [enregistrerPaire].
  Future<void> enregistrerJetonAcces(String jeton) {
    _exigerNonVide(_cleJetonAcces, jeton);
    return _stockage.write(key: _cleJetonAcces, value: jeton);
  }

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

  /// Le message donne le nom de la clé, jamais la valeur d'un jeton (la
  /// valeur refusée est toujours vide).
  static void _exigerNonVide(String cle, String valeur) {
    if (valeur.isEmpty) {
      throw ArgumentError.value(valeur, cle, 'ne doit pas être vide');
    }
  }
}
