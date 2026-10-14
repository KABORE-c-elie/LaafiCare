import 'dart:async';
import 'dart:convert';
import 'dart:io' show IOException;

import 'package:http/http.dart' as http;

import '../../config/configuration.dart';
import '../../domain/models/erreur_api.dart';

/// Réponse réussie (2xx) de l'API.
class ReponseApi {
  const ReponseApi({required this.statut, this.json});

  /// Code HTTP (200, 201, 204…).
  final int statut;

  /// Corps JSON ; null quand la réponse n'a pas de corps (204).
  final Map<String, dynamic>? json;
}

/// Le serveur n'a pas répondu : pas de réseau, serveur injoignable, échec
/// TLS, ou délai dépassé. L'application affiche un message unique dans ces
/// cas (décision du porteur) ; [delaiDepasse] permet de distinguer le délai
/// dépassé là où c'est utile (ex. inscription dont la réponse s'est perdue,
/// docs/api.md §2.2.2).
class ErreurReseau implements Exception {
  const ErreurReseau({required this.delaiDepasse});

  final bool delaiDepasse;

  @override
  String toString() => 'ErreurReseau(delaiDepasse: $delaiDepasse)';
}

/// Client de l'API LaafiCare : JSON, délai d'attente, erreurs au format
/// {"erreur", "code"} (docs/api.md §1).
///
/// Un 401 n'est jamais traité ici : avec les jetons d'accès de 15 minutes,
/// il faut d'abord tenter un renouvellement (docs/api.md §1.7). C'est le
/// gestionnaire de session qui décide, le client ne fait que lever
/// l'ErreurApi.
///
/// Un seul client http pour toute l'application, comme le recommande le
/// README du paquet http : « it's recommended to use a Client instance
/// rather than the top-level functions » (https://pub.dev/packages/http).
class ClientApi {
  // Paramètres nommés privés (Dart ≥ 3.12) : l'appelant écrit client:,
  // delaiAttente: ; les champs restent privés.
  // https://dart.dev/language/constructors
  ClientApi({
    required Configuration configuration,
    required this._client,
    this._delaiAttente = Configuration.delaiAttente,
  }) : _adresseApi = configuration.adresseApi;

  final Uri _adresseApi;
  final http.Client _client;

  /// Délai par défaut de chaque requête.
  final Duration _delaiAttente;

  /// [delaiAttente] remplace le délai par défaut pour cette seule requête
  /// (ex. renouvellement de session, plus court : docs/api.md §1.7).
  Future<ReponseApi> get(
    String chemin, {
    String? jeton,
    Duration? delaiAttente,
  }) =>
      _envoyer('GET', chemin, jeton: jeton, delaiAttente: delaiAttente);

  Future<ReponseApi> post(
    String chemin, {
    Map<String, Object?>? corps,
    String? jeton,
    Duration? delaiAttente,
  }) =>
      _envoyer('POST', chemin,
          corps: corps, jeton: jeton, delaiAttente: delaiAttente);

  /// Ferme les connexions ; le client n'est plus utilisable ensuite.
  void fermer() => _client.close();

  Future<ReponseApi> _envoyer(
    String methode,
    String chemin, {
    Map<String, Object?>? corps,
    String? jeton,
    Duration? delaiAttente,
  }) async {
    assert(chemin.startsWith('/'), 'Le chemin commence par / : $chemin');

    // Délai qui interrompt vraiment la requête (paquet http ≥ 1.5.0, doc
    // d'Abortable : « To implement a timeout (to abort the request after a
    // set time has elapsed), use [Future.delayed] »). Un Timer annulable
    // plutôt que Future.delayed, pour ne rien laisser en attente après la
    // réponse.
    final declencheur = Completer<void>();
    final minuterie =
        Timer(delaiAttente ?? _delaiAttente, declencheur.complete);

    final requete = http.AbortableRequest(
      methode,
      _adresseApi.replace(path: '${_adresseApi.path}$chemin'),
      abortTrigger: declencheur.future,
    );
    if (jeton != null) {
      requete.headers['Authorization'] = 'Bearer $jeton';
    }
    if (corps != null) {
      // docs/api.md §1.1 : Content-Type: application/json sur toute requête
      // qui envoie un corps. Le paquet http y ajoute « charset=utf-8 ».
      requete.headers['Content-Type'] = 'application/json';
      requete.body = jsonEncode(corps);
    }

    final http.Response reponse;
    try {
      reponse = await http.Response.fromStream(await _client.send(requete));
    } on http.RequestAbortedException {
      throw const ErreurReseau(delaiDepasse: true);
    } on http.ClientException {
      // Doc d'IOClient : « If there is a socket-level failure […] [IOClient]
      // will emit a [ClientException] ».
      throw const ErreurReseau(delaiDepasse: false);
    } on IOException {
      // Le code d'IOClient (paquet http 1.6.0, io_client.dart) ne convertit
      // en ClientException que SocketException et HttpException à l'envoi,
      // et seulement HttpException pendant la lecture du corps. Passent donc
      // sans conversion : les échecs TLS (TlsException, HandshakeException,
      // CertificateException) et une SocketException pendant la lecture du
      // corps. Toutes ces exceptions héritent d'IOException, « Base class for
      // all IO related exceptions » (SDK Dart, dart:io, common.dart). On
      // n'attrape que cette famille : une erreur de programmation reste
      // visible.
      throw const ErreurReseau(delaiDepasse: false);
    } finally {
      minuterie.cancel();
    }

    // Décodé en UTF-8 par nous : sans charset dans Content-Type, le paquet
    // http décoderait en latin1 (Response.body) et abîmerait les accents.
    // Le README fait de même : utf8.decode(response.bodyBytes).
    final texte = utf8.decode(reponse.bodyBytes, allowMalformed: true);

    if (reponse.statusCode < 200 || reponse.statusCode >= 300) {
      throw ErreurApi.depuisReponse(
        statut: reponse.statusCode,
        corps: texte,
        entetes: reponse.headers,
      );
    }

    if (texte.isEmpty) {
      return ReponseApi(statut: reponse.statusCode);
    }
    final Object? json;
    try {
      json = jsonDecode(texte);
    } on FormatException {
      throw ErreurApi(
        statut: reponse.statusCode,
        code: CodesErreur.reponseIllisible,
      );
    }
    if (json is! Map<String, dynamic>) {
      throw ErreurApi(
        statut: reponse.statusCode,
        code: CodesErreur.reponseIllisible,
      );
    }
    return ReponseApi(statut: reponse.statusCode, json: json);
  }
}
