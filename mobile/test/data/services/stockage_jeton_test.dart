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

void main() {
  // Simulation fournie par le paquet : « Initializes the shared preferences
  // with mock values for testing » (FlutterSecureStorage.setMockInitialValues).
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('aucun jeton au départ', () async {
    expect(await StockageJeton().lireJetonAcces(), isNull);
  });

  test('enregistre puis relit le jeton', () async {
    final stockage = StockageJeton();
    await stockage.enregistrerJetonAcces('jeton-exemple');
    expect(await stockage.lireJetonAcces(), 'jeton-exemple');
  });

  test('effacer supprime le jeton', () async {
    final stockage = StockageJeton();
    await stockage.enregistrerJetonAcces('jeton-exemple');
    await stockage.effacer();
    expect(await stockage.lireJetonAcces(), isNull);
  });

  test("effacer ne touche pas aux autres secrets de l'application", () async {
    FlutterSecureStorage.setMockInitialValues({'autre_secret': 'garde'});
    final stockage = StockageJeton();
    await stockage.enregistrerJetonAcces('jeton-exemple');
    await stockage.effacer();
    expect(
      await const FlutterSecureStorage().read(key: 'autre_secret'),
      'garde',
    );
  });

  test('refuse un jeton vide', () {
    expect(
      () => StockageJeton().enregistrerJetonAcces(''),
      throwsArgumentError,
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
}
