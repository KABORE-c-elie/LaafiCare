import 'dart:async';
import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:patient/config/configuration.dart';
import 'package:patient/data/repositories/auth_repository.dart';
import 'package:patient/data/services/client_api.dart';
import 'package:patient/data/services/gestionnaire_session.dart';
import 'package:patient/data/services/stockage_jeton.dart';
import 'package:patient/ui/connexion/connexion_view_model.dart';

final _configuration =
    Configuration.depuis('http://127.0.0.1:8080', versionPubliee: false);

/// Valeurs fictives des exemples de docs/api.md.
const _saisie = '70 00 00 00';
const _motDePasse = 'Exemple#2026';

http.Response _json(int statut, Object corps) => http.Response.bytes(
    utf8.encode(jsonEncode(corps)), statut,
    headers: {'content-type': 'application/json'});

http.Response _erreur(int statut, String code) =>
    _json(statut, {'erreur': 'x', 'code': code});

http.Response _ouverture() => _json(200, {
      'etape': 'complete',
      'jeton_acces': 'acces-1',
      'expire_dans': 900,
      'jeton_renouvellement': 'renouv-1',
    });

class _Banc {
  _Banc(Future<http.Response> Function(http.Request requete) serveur) {
    final client = ClientApi(
      configuration: _configuration,
      client: MockClient((requete) {
        requetes.add(requete);
        return serveur(requete);
      }),
    );
    auth = AuthRepository(
      client: client,
      session: GestionnaireSession(client: client, stockage: StockageJeton()),
    );
    vm = ConnexionViewModel(auth: auth);
  }

  final requetes = <http.Request>[];
  late final AuthRepository auth;
  late final ConnexionViewModel vm;

  Future<bool> connecter({
    String saisie = _saisie,
    String motDePasse = _motDePasse,
  }) =>
      vm.connecter(saisieTelephone: saisie, motDePasse: motDePasse);

  Map<String, dynamic> get corps =>
      jsonDecode(requetes.single.body) as Map<String, dynamic>;
}

void main() {
  setUp(() => FlutterSecureStorage.setMockInitialValues({}));

  test('1. champs vides : erreur locale, aucune requête', () async {
    final banc = _Banc((_) async => throw StateError('aucune requête'));
    for (final (saisie, motDePasse, attendue) in [
      ('', _motDePasse, ErreurConnexion.telephoneVide),
      ('   ', _motDePasse, ErreurConnexion.telephoneVide),
      (_saisie, '', ErreurConnexion.motDePasseVide),
    ]) {
      expect(await banc.connecter(saisie: saisie, motDePasse: motDePasse),
          isFalse);
      expect(banc.vm.erreur, attendue, reason: '"$saisie" / "$motDePasse"');
    }
    expect(banc.requetes, isEmpty);
  });

  test('2. numéro envoyé : +226 suivi de la saisie, telle quelle', () async {
    final banc = _Banc((_) async => _ouverture());
    await banc.connecter();
    expect(banc.corps['telephone'], '+22670 00 00 00');
  });

  test('3. succès : vrai, plus en cours, aucune erreur, session ouverte',
      () async {
    final banc = _Banc((_) async => _ouverture());
    expect(await banc.connecter(), isTrue);
    expect(banc.vm.enCours, isFalse);
    expect(banc.vm.erreur, isNull);
    expect(banc.auth.etat, EtatSession.connecte);
  });

  test('4. 401 identifiants_invalides : identifiants', () async {
    final banc = _Banc((_) async => _erreur(401, 'identifiants_invalides'));
    expect(await banc.connecter(), isFalse);
    expect(banc.vm.erreur, ErreurConnexion.identifiants);
  });

  test('5. 422 telephone_invalide : telephoneInvalide', () async {
    final banc = _Banc((_) async => _erreur(422, 'telephone_invalide'));
    await banc.connecter();
    expect(banc.vm.erreur, ErreurConnexion.telephoneInvalide);
  });

  test('6. 500, réponse illisible ou code inconnu : serveur', () async {
    for (final reponse in [
      _erreur(500, 'erreur_interne'),
      http.Response('<html></html>', 502),
      _erreur(400, 'json_invalide'),
    ]) {
      final banc = _Banc((_) async => reponse);
      await banc.connecter();
      expect(banc.vm.erreur, ErreurConnexion.serveur,
          reason: '${reponse.statusCode}');
    }
  });

  test('7. réseau absent : reseau', () async {
    final banc =
        _Banc((r) async => throw http.ClientException('coupure', r.url));
    await banc.connecter();
    expect(banc.vm.erreur, ErreurConnexion.reseau);
  });

  test('8. second_facteur : etapeNonGeree', () async {
    final banc = _Banc((_) async => _json(200, {
          'etape': 'second_facteur',
          'jeton_intermediaire': 'jeton-intermediaire-exemple',
        }));
    await banc.connecter();
    expect(banc.vm.erreur, ErreurConnexion.etapeNonGeree);
  });

  test('9. en cours pendant l\'envoi ; double appui : une seule requête',
      () async {
    final retenue = Completer<void>();
    final banc = _Banc((_) async {
      await retenue.future;
      return _ouverture();
    });
    final premier = banc.connecter();
    await Future<void>.delayed(Duration.zero);
    expect(banc.vm.enCours, isTrue);
    expect(await banc.connecter(), isFalse); // double appui ignoré
    retenue.complete();
    expect(await premier, isTrue);
    expect(banc.requetes, hasLength(1));
  });

  test('10. basculerMotDePasse alterne l\'affichage', () {
    final banc = _Banc((_) async => _ouverture());
    expect(banc.vm.motDePasseVisible, isFalse);
    banc.vm.basculerMotDePasse();
    expect(banc.vm.motDePasseVisible, isTrue);
    banc.vm.basculerMotDePasse();
    expect(banc.vm.motDePasseVisible, isFalse);
  });

  test('11. fermé pendant l\'envoi : aucune erreur', () async {
    final retenue = Completer<void>();
    final banc = _Banc((_) async {
      await retenue.future;
      return _ouverture();
    });
    final envoi = banc.connecter();
    await Future<void>.delayed(Duration.zero);
    banc.vm.dispose();
    retenue.complete();
    expect(await envoi, isTrue);
  });

  test('12. mot de passe avec des espaces : envoyé tel quel', () async {
    final banc = _Banc((_) async => _ouverture());
    await banc.connecter(motDePasse: ' mot de passe# 1 ');
    expect(banc.corps['mot_de_passe'], ' mot de passe# 1 ');
  });

  test('13. effacerErreur retire le message', () async {
    final banc = _Banc((_) async => _erreur(401, 'identifiants_invalides'));
    await banc.connecter();
    expect(banc.vm.erreur, isNotNull);
    banc.vm.effacerErreur();
    expect(banc.vm.erreur, isNull);
  });
}
