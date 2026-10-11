import 'package:flutter_test/flutter_test.dart';
import 'package:patient/config/configuration.dart';

void main() {
  group('Configuration.depuis', () {
    test('accepte http en débogage (adb reverse vers 127.0.0.1)', () {
      final config = Configuration.depuis('http://127.0.0.1:8080',
          versionPubliee: false);
      expect(config.adresseApi, Uri.parse('http://127.0.0.1:8080'));
    });

    test('accepte https en version publiée', () {
      final config = Configuration.depuis('https://api.exemple.bf',
          versionPubliee: true);
      expect(config.adresseApi.scheme, 'https');
    });

    test('refuse http en version publiée', () {
      expect(
        () => Configuration.depuis('http://api.exemple.bf',
            versionPubliee: true),
        throwsA(isA<ErreurConfiguration>()),
      );
    });

    test('refuse une adresse vide (lancement sans --dart-define)', () {
      expect(
        () => Configuration.depuis('', versionPubliee: false),
        throwsA(isA<ErreurConfiguration>()),
      );
    });

    test('refuse une adresse sans schéma ou sans hôte', () {
      for (final adresse in ['127.0.0.1:8080', 'ftp://hote', 'http://']) {
        expect(
          () => Configuration.depuis(adresse, versionPubliee: false),
          throwsA(isA<ErreurConfiguration>()),
          reason: adresse,
        );
      }
    });

    test('refuse un identifiant, une requête ou un fragment, sans les '
        'recopier dans le message', () {
      for (final adresse in [
        'http://nom:secret@127.0.0.1:8080',
        'http://127.0.0.1:8080?x=1',
        'http://127.0.0.1:8080#x',
      ]) {
        expect(
          () => Configuration.depuis(adresse, versionPubliee: false),
          throwsA(isA<ErreurConfiguration>().having(
            (e) => e.message,
            'message',
            "Adresse de l'API invalide : pas d'identifiant, de requête ni de "
                'fragment.',
          )),
          reason: adresse,
        );
      }
    });

    test('retire la barre finale', () {
      final config = Configuration.depuis('http://127.0.0.1:8080/',
          versionPubliee: false);
      expect(config.adresseApi.toString(), 'http://127.0.0.1:8080');
    });
  });

  test("délai d'attente de 30 s", () {
    expect(Configuration.delaiAttente, const Duration(seconds: 30));
  });
}
