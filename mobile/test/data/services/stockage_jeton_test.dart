import 'package:flutter/services.dart' show PlatformException;
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:patient/data/services/stockage_jeton.dart';

/// Stockage dont la lecture échoue, comme après la restauration d'une
/// sauvegarde Android (« Failed to unwrap key »).
class _StockageIllisible extends Fake implements FlutterSecureStorage {
  final suppressions = <String>[];

  @override
  Future<String?> read({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async =>
      throw PlatformException(code: 'Exception encountered');

  @override
  Future<void> delete({
    required String key,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async =>
      suppressions.add(key);
}

/// Stockage qui note l'ordre des écritures.
class _StockageEnregistreur extends Fake implements FlutterSecureStorage {
  final ecritures = <String>[];

  @override
  Future<void> write({
    required String key,
    required String? value,
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async =>
      ecritures.add(key);
}

void main() {
  // Simulation fournie par le paquet : « Initializes the shared preferences
  // with mock values for testing » (FlutterSecureStorage.setMockInitialValues).
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('aucun jeton au départ', () async {
    expect(await StockageJeton().lireJetonAcces(), isNull);
  });

  test('enregistre puis relit le jeton', () async {
    final stockage = StockageJeton();
    await stockage.enregistrerPaire(
        jetonAcces: 'jeton-exemple', jetonRenouvellement: 'renouv-exemple');
    expect(await stockage.lireJetonAcces(), 'jeton-exemple');
  });

  test('effacer supprime le jeton', () async {
    final stockage = StockageJeton();
    await stockage.enregistrerPaire(
        jetonAcces: 'jeton-exemple', jetonRenouvellement: 'renouv-exemple');
    await stockage.effacer();
    expect(await stockage.lireJetonAcces(), isNull);
  });

  test("effacer ne touche pas aux autres secrets de l'application", () async {
    FlutterSecureStorage.setMockInitialValues({'autre_secret': 'garde'});
    final stockage = StockageJeton();
    await stockage.enregistrerPaire(
        jetonAcces: 'jeton-exemple', jetonRenouvellement: 'renouv-exemple');
    await stockage.effacer();
    expect(
      await const FlutterSecureStorage().read(key: 'autre_secret'),
      'garde',
    );
  });

  test('jeton illisible : traité comme absent, puis effacé', () async {
    final illisible = _StockageIllisible();
    expect(
      await StockageJeton(stockage: illisible).lireJetonAcces(),
      isNull,
    );
    expect(illisible.suppressions, ['jeton_acces']);
  });

  test('7. enregistrerPaire puis lecture des deux jetons', () async {
    final stockage = StockageJeton();
    await stockage.enregistrerPaire(
        jetonAcces: 'acces-exemple', jetonRenouvellement: 'renouv-exemple');
    expect(await stockage.lireJetonAcces(), 'acces-exemple');
    expect(await stockage.lireJetonRenouvellement(), 'renouv-exemple');
  });

  test('8. ordre d\'écriture : renouvellement avant accès (choix C1)',
      () async {
    final enregistreur = _StockageEnregistreur();
    await StockageJeton(stockage: enregistreur).enregistrerPaire(
        jetonAcces: 'acces-exemple', jetonRenouvellement: 'renouv-exemple');
    expect(enregistreur.ecritures, ['jeton_renouvellement', 'jeton_acces']);
  });

  test('9. un jeton vide dans la paire : refus, et aucune écriture',
      () async {
    for (final (acces, renouvellement) in [('', 'r'), ('a', '')]) {
      final enregistreur = _StockageEnregistreur();
      await expectLater(
        StockageJeton(stockage: enregistreur).enregistrerPaire(
            jetonAcces: acces, jetonRenouvellement: renouvellement),
        throwsArgumentError,
      );
      expect(enregistreur.ecritures, isEmpty);
    }
  });

  test('10. effacer supprime les deux jetons, pas les autres secrets',
      () async {
    FlutterSecureStorage.setMockInitialValues({'autre_secret': 'garde'});
    final stockage = StockageJeton();
    await stockage.enregistrerPaire(
        jetonAcces: 'acces-exemple', jetonRenouvellement: 'renouv-exemple');
    await stockage.effacer();
    expect(await stockage.lireJetonAcces(), isNull);
    expect(await stockage.lireJetonRenouvellement(), isNull);
    expect(
      await const FlutterSecureStorage().read(key: 'autre_secret'),
      'garde',
    );
  });

  test('11. jeton de renouvellement illisible : absent, puis effacé',
      () async {
    final illisible = _StockageIllisible();
    expect(
      await StockageJeton(stockage: illisible).lireJetonRenouvellement(),
      isNull,
    );
    expect(illisible.suppressions, ['jeton_renouvellement']);
  });
}
