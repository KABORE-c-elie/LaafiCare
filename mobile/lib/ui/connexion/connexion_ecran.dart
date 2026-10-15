import 'package:flutter/material.dart';

import '../../l10n/app_localizations.dart';
import '../core/theme/theme_laaficare.dart';
import '../core/widgets/logo_laafi.dart';
import '../core/widgets/message_erreur.dart';
import 'connexion_view_model.dart';

/// Écran de connexion du patient (maquette validée par le porteur).
class ConnexionEcran extends StatefulWidget {
  const ConnexionEcran({
    super.key,
    required this.vm,
    required this.surMotDePasseOublie,
    required this.surCreerCompte,
    required this.surUrgences,
  });

  final ConnexionViewModel vm;

  /// Navigation fournie par le routeur : l'écran ne connaît pas les routes.
  final VoidCallback surMotDePasseOublie;
  final VoidCallback surCreerCompte;

  /// Numéros d'urgence, accessibles sans compte.
  final VoidCallback surUrgences;

  @override
  State<ConnexionEcran> createState() => _ConnexionEcranState();
}

class _ConnexionEcranState extends State<ConnexionEcran> {
  final _telephone = TextEditingController();
  final _motDePasse = TextEditingController();

  @override
  void dispose() {
    _telephone.dispose();
    _motDePasse.dispose();
    super.dispose();
  }

  void _envoyer() => widget.vm.connecter(
        saisieTelephone: _telephone.text,
        motDePasse: _motDePasse.text,
      );

  @override
  Widget build(BuildContext context) {
    final t = Textes.of(context);
    final theme = Theme.of(context);
    final schema = theme.colorScheme;
    return Scaffold(
      body: SafeArea(
        child: ListenableBuilder(
          listenable: widget.vm,
          builder: (context, _) {
            final vm = widget.vm;
            final erreur = vm.erreur;
            final erreurTelephone = erreur == ErreurConnexion.telephoneVide ||
                erreur == ErreurConnexion.telephoneInvalide;
            final erreurMotDePasse = erreur != null && !erreurTelephone;
            return CustomScrollView(
              slivers: [
                SliverPadding(
                  padding: const EdgeInsets.all(Espacements.marge),
                  sliver: SliverFillRemaining(
                    hasScrollBody: false,
                    child: AutofillGroup(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          Align(
                            alignment: Alignment.centerLeft,
                            child: LogoLaafi(libelle: t.nomApplication),
                          ),
                          const SizedBox(height: Espacements.entreSections),
                          Semantics(
                            header: true,
                            child: Text(t.connexionTitre,
                                style: theme.textTheme.headlineSmall),
                          ),
                          const SizedBox(height: 8),
                          Text(t.connexionSousTitre,
                              style: theme.textTheme.bodyLarge
                                  ?.copyWith(color: schema.onSurfaceVariant)),
                          const SizedBox(height: Espacements.entreSections),

                          // Téléphone : indicatif fixe +226, séparé par un
                          // trait. Libellé et champ fusionnés pour le lecteur
                          // d'écran (_ChampNomme).
                          _ChampNomme(
                            libelle: t.champTelephone,
                            champ: TextField(
                              controller: _telephone,
                              keyboardType: TextInputType.phone,
                              textInputAction: TextInputAction.next,
                              autofillHints: const [
                                AutofillHints.telephoneNumberNational,
                              ],
                              onChanged: (_) => vm.effacerErreur(),
                              decoration: InputDecoration(
                                hintText: t.indicationTelephone,
                                prefixIcon:
                                    const _Indicatif(ConnexionViewModel.indicatif),
                                error: erreurTelephone ? _sansTexte : null,
                              ),
                            ),
                          ),
                          if (erreurTelephone) ...[
                            const SizedBox(height: 8),
                            MessageErreur(_texteErreur(t, erreur!)),
                          ],
                          const SizedBox(height: Espacements.marge),

                          // Mot de passe, avec le bouton œil.
                          _ChampNomme(
                            libelle: t.champMotDePasse,
                            champ: TextField(
                              controller: _motDePasse,
                              obscureText: !vm.motDePasseVisible,
                              textInputAction: TextInputAction.done,
                              autofillHints: const [AutofillHints.password],
                              onChanged: (_) => vm.effacerErreur(),
                              onSubmitted: (_) => _envoyer(),
                              decoration: InputDecoration(
                                error: erreurMotDePasse ? _sansTexte : null,
                                suffixIcon: IconButton(
                                  // tooltip : libellé lu par le lecteur
                                  // d'écran.
                                  tooltip: vm.motDePasseVisible
                                      ? t.masquerMotDePasse
                                      : t.afficherMotDePasse,
                                  icon: Icon(vm.motDePasseVisible
                                      ? Icons.visibility_off_outlined
                                      : Icons.visibility_outlined),
                                  onPressed: vm.basculerMotDePasse,
                                ),
                              ),
                            ),
                          ),
                          if (erreurMotDePasse) ...[
                            const SizedBox(height: 8),
                            MessageErreur(_texteErreur(t, erreur)),
                          ],

                          // Lien aligné à droite, zone touchable de 48.
                          Align(
                            alignment: Alignment.centerRight,
                            child: TextButton(
                              style: TextButton.styleFrom(
                                minimumSize: const Size(
                                    Espacements.zoneTouchable,
                                    Espacements.zoneTouchable),
                              ),
                              onPressed: widget.surMotDePasseOublie,
                              child: Text(t.motDePasseOublie),
                            ),
                          ),
                          const SizedBox(height: 8),
                          FilledButton(
                            onPressed: vm.enCours ? null : _envoyer,
                            child: vm.enCours
                                ? Semantics(
                                    label: t.boutonSeConnecter,
                                    child: const SizedBox.square(
                                      dimension: 24,
                                      child: CircularProgressIndicator(
                                          strokeWidth: 2),
                                    ),
                                  )
                                : Text(t.boutonSeConnecter),
                          ),
                          const SizedBox(height: Espacements.entreSections),
                          Row(
                            children: [
                              const Expanded(child: Divider()),
                              Padding(
                                padding: const EdgeInsets.symmetric(
                                    horizontal: Espacements.marge),
                                child: Text(t.pasEncoreDeCompte,
                                    style: theme.textTheme.bodySmall?.copyWith(
                                        color: schema.onSurfaceVariant)),
                              ),
                              const Expanded(child: Divider()),
                            ],
                          ),
                          const SizedBox(height: Espacements.entreSections),
                          FilledButton.tonal(
                            style: StylesBoutons.secondaire(context),
                            onPressed: widget.surCreerCompte,
                            child: Text(t.boutonCreerCompte),
                          ),

                          // Pousse la carte en bas de l'écran.
                          const Spacer(),
                          const SizedBox(height: Espacements.entreSections),
                          _CarteUrgences(
                            titre: t.urgencesTitre,
                            resume: t.urgencesResume,
                            surAppui: widget.surUrgences,
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ],
            );
          },
        ),
      ),
    );
  }

  /// Met le champ en état d'erreur (bord errorBorder du thème) sans texte
  /// Material : le message est affiché par MessageErreur, avec son icône.
  /// SDK, InputDecoration.error : « If non-null, the border's color animates
  /// to red and the helperText is not shown » (ici la couleur error du
  /// thème, neutre).
  static const _sansTexte = SizedBox.shrink();

  /// Un texte par erreur, choisi d'après le code (jamais le message du
  /// serveur, docs/api.md §1.3).
  static String _texteErreur(Textes t, ErreurConnexion erreur) =>
      switch (erreur) {
        ErreurConnexion.telephoneVide => t.telephoneVide,
        ErreurConnexion.motDePasseVide => t.motDePasseVide,
        ErreurConnexion.identifiants => t.erreurIdentifiants,
        ErreurConnexion.telephoneInvalide => t.erreurTelephoneInvalide,
        ErreurConnexion.reseau => t.erreurReseau,
        ErreurConnexion.serveur => t.erreurServeur,
        ErreurConnexion.etapeNonGeree => t.erreurEtapeNonGeree,
      };
}

/// Libellé au-dessus d'un champ : le champ est lu avec son nom, et le
/// libellé visible n'est pas lu une seconde fois.
///
/// MergeSemantics n'est pas possible ici : autour d'un TextField, il lève une
/// assertion du SDK (semantics.dart, « node.parent!.isPartOfNodeMerging »).
/// Le libellé visible est donc retiré de l'arbre d'accessibilité
/// (ExcludeSemantics, SDK : « This can be used to hide descendant widgets
/// that would otherwise be reported but that would only be confusing »), et
/// le nom est donné au champ par Semantics.label (SDK : « Provides a textual
/// description of the widget »). Le test vérifie que le champ porte ce nom
/// et qu'aucun autre nœud ne le porte.
class _ChampNomme extends StatelessWidget {
  const _ChampNomme({required this.libelle, required this.champ});

  final String libelle;
  final Widget champ;

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          ExcludeSemantics(
            child: Text(libelle,
                style: Theme.of(context).textTheme.labelLarge),
          ),
          const SizedBox(height: 8),
          Semantics(label: libelle, child: champ),
        ],
      );
}

/// Indicatif fixe devant la saisie du téléphone, séparé par un trait.
class _Indicatif extends StatelessWidget {
  const _Indicatif(this.indicatif);

  final String indicatif;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(left: Espacements.marge, right: 12),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(indicatif, style: theme.textTheme.bodyLarge),
          const SizedBox(width: 12),
          SizedBox(
            height: 24,
            child: VerticalDivider(
                width: 1, thickness: 1, color: theme.colorScheme.outline),
          ),
        ],
      ),
    );
  }
}

/// Carte « Numéros d'urgence » : pastille ronde secondary (bleu, jamais le
/// rouge, décision du porteur) avec une icône de téléphone, et une flèche.
class _CarteUrgences extends StatelessWidget {
  const _CarteUrgences({
    required this.titre,
    required this.resume,
    required this.surAppui,
  });

  final String titre;
  final String resume;
  final VoidCallback surAppui;

  @override
  Widget build(BuildContext context) {
    final schema = Theme.of(context).colorScheme;
    return Card(
      margin: EdgeInsets.zero,
      clipBehavior: Clip.antiAlias,
      child: ListTile(
        onTap: surAppui,
        leading: CircleAvatar(
          radius: 20,
          backgroundColor: schema.secondary,
          child: Icon(Icons.phone, color: schema.onSecondary),
        ),
        title: Text(titre),
        subtitle: Text(resume),
        trailing: const Icon(Icons.chevron_right),
      ),
    );
  }
}
