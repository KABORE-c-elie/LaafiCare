# LaafiCare — Contexte projet pour Claude Code

> Ce document est le point d'entrée de contexte. Lis-le entièrement avant toute tâche de code, de CDC ou de diagramme. Il remplace toute réexplication orale du projet.

---

## 1. Identité du projet

**Nom :** LaafiCare *(anciennement SantéBF, renommé)* — « Laafi » signifie santé/bien-être en mooré.

**Nature :** Plateforme numérique de santé intégrée pour le Burkina Faso, connectant patients, médecins, sages-femmes, pharmaciens, biologistes et assureurs autour d'un identifiant patient unique (NIP).

**Porteur du projet :** Calliste Elie KABORE — ingénieur en Électronique et Informatique Industrielle. Il est **seul développeur** au quotidien (MOA et MOE réunis).

**Aucun autre nom que Calliste Elie KABORE ne doit apparaître dans le projet ou le CDC.** Il porte seul le rôle de MOA et de MOE — pas de consultants nommés, pas de co-auteurs.

**Contrainte projet à retenir :** charge de développement portée par une seule personne → priorisation stricte du périmètre Must Have avant tout Should Have. C'est un risque identifié explicitement dans la matrice des risques du CDC.

---

## 2. Le problème résolu (à ne jamais édulcorer)

Le système de santé burkinabè fonctionne presque exclusivement sur support papier/Excel/Word, y compris pour le dossier médical, le suivi de grossesse et l'assurance maladie. Conséquences documentées par entretiens terrain réels (pas des hypothèses) :

- Un médecin perd **15 minutes par consultation** à reconstituer l'historique d'un patient (entretien médecin, une clinique privée de Ouagadougou)
- **1 patient sur 5** interrogés a dû refaire des analyses biologiques déjà payées, résultats perdus entre labo et patient
- **2 patients sur 5** ont eu une réaction médicamenteuse évitable (allergie non vérifiée)
- Un pharmacien confirme que **l'enregistrement manuel du patient** est le principal facteur d'erreur, devant l'analyse elle-même
- Un patient a attendu **plus d'un an** un remboursement d'assurance sans réponse
- Le carnet national de santé mère-enfant (4 CPN obligatoires) reste **100% papier**, sans alerte automatique de rendez-vous manqué ou de tension anormale
- Pertes pharmacie estimées à **4 millions FCFA/an** en médicaments périmés non écoulés (entretien assistant pharmacien)

**Objectif chiffré de référence :** réduire le délai de remboursement assurance de 1 mois à moins de 72h (validé par le précédent documenté de la plateforme MUNASEB, mutuelle étudiante burkinabè).

---

## 3. Fondements documentaires (sources réelles, pas des suppositions)

Toute fonctionnalité du CDC doit être traçable à une de ces sources :

| Source | Ce qu'elle a apporté |
|---|---|
| Entretien médecin généraliste (une clinique privée de Ouagadougou — jamais nommée, pour que la personne interrogée ne soit pas reconnaissable) | Flux consultation, ordonnances, référencements, contraintes réseau/électricité |
| Entretien biologiste/technologiste biomédical | Circuit analyse, double validation technique+biologique, protocole résultats sensibles |
| Entretien assistant pharmacien | Gestion stock, vérification allergie, **substitution par DCI** (suggestion spontanée non anticipée), écarts stock virtuel/physique |
| 5 entretiens patients (Ouagadougou) | Aucune assurance, tous équipés smartphone+Mobile Money, **géolocalisation médicaments disponibles** (besoin exprimé par 4/5), erreur chirurgicale + dépendance antidouleurs (1 cas) |
| Carnet de Santé Mère-Enfant (Ministère de la Santé BF) | Calendrier officiel 4 CPN, champs cliniques exacts (HU, MAF, BDC, TV...), 12 signes de danger, score APGAR, seuil tension 140 mmHg |
| Mémoire MUNASEB (mutuelle étudiante) | Modèle de remboursement automatisé, workflow acteurs assurance, référence "1 mois → 72h" |

**Profils identifiés mais pas encore interviewés en entretien direct** : Gynécologue/Sage-femme (couvert indirectement par le carnet CPN), Agent d'assurance (couvert indirectement par MUNASEB), Directeur de structure, Infirmier, Agent d'accueil/administratif, Major de laboratoire. Ne pas prétendre qu'ils ont été interviewés s'ils ne l'ont pas été — dire "besoin déduit du carnet national" ou "rôle mentionné en filigrane par le pharmacien", jamais "confirmé en entretien" pour ces profils.

**Décision actée (2026) : chaque profil dispose de son propre compte d'inscription et de connexion — aucune délégation d'usage entre rôles.** Cela inclut Infirmier, Agent d'accueil et Directeur, qui rejoignent donc la hiérarchie `Utilisateur` comme sous-classes à part entière (voir section 6). Le cas de Major de Laboratoire reste ouvert : compte séparé (cohérent avec la règle) vs simple attribut `responsableStock` sur `Biologiste` — à trancher avant de coder ce rôle.

---

## 4. Architecture technique — ⚠️ PIVOT EN COURS

**L'architecture décrite dans les versions précédentes du CDC (Node.js/FastAPI + Flutter Web partout) est OBSOLÈTE. Nouvelle direction validée :**

| Couche | Choix technique | Justification |
|---|---|---|
| **Backend API** | **Rust + Axum** | Performance et sécurité mémoire — contexte critique santé, aucune tolérance pour un crash ou une fuite mémoire sur des données patient |
| ORM / accès DB | SQLx (requêtes SQL paramétrées, écrites avec `sqlx::query` / `query_as` / `query_scalar`) | Bibliothèque PostgreSQL native de l'écosystème Rust asynchrone, qui fournit aussi les migrations et `#[sqlx::test]`. **Les requêtes ne sont PAS encore vérifiées à la compilation** (aucune macro `query!` dans le code au 2026-10-06) : leur adoption est un point ouvert (section 14), à traiter comme une étape à part après la stabilisation du module MUNASEB |
| Base de données | PostgreSQL | Inchangé — déjà acté |
| **Frontend web** (Hôpital, Pharmacie, Laboratoire, Assurance) | **Angular** | Remplace Flutter Web |
| **Frontend mobile** (Patient) | **Flutter** | Inchangé — reste la bonne techno pour le mobile offline-first |
| Authentification | JWT | Inchangé (BF-01) |
| Paiement | API Orange Money, API Moov Money | Inchangé |
| Notification de secours | API Orange BF, API Telecel (SMS) | Inchangé |
| Synchronisation offline | À redéfinir côté Rust (l'ancien choix WatermelonDB/SQLite était pensé pour Flutter Web ; le mobile Flutter garde SQLite local, mais le protocole de sync avec un backend Rust doit être reconçu) | ⚠️ Point ouvert à trancher |

**Ce qui ne change PAS :** les 20 entités "métier" du modèle de données, les 15 BF et 5 BNF du périmètre V1, le calendrier des jalons Go/No-Go. **Ce qui a changé depuis la dernière version de ce paragraphe :** la hiérarchie `Utilisateur` n'est plus (Patient, Medecin, SageFemme, Pharmacien, Biologiste, AgentAssurance, Administrateur) — voir section 6 pour la liste à jour à 12 rôles, SageFemme n'étant plus une classe séparée. **Le format du NIP a également changé (2026-09-23, voir section 11)** : ce n'est plus `BF-AAAA-XXXXXX`, mais 16 chiffres purement numériques (15 chiffres de séquence globale + 1 chiffre de contrôle Luhn) — un identifiant de santé ne devrait rien encoder de significatif comme une année.

**Tâche immédiate demandée par le porteur :** mettre à jour le CDC (toutes sections concernées, notamment III.2.3 « Architecture technique retenue » et toute mention de Node.js/FastAPI/Flutter Web) pour refléter ce pivot, **avant** de retoucher aux diagrammes ou au code.

---

## 5. Les 5 applications du système

| Application | Plateforme (nouvelle) | Utilisateurs | Statut périmètre |
|---|---|---|---|
| App Mobile Patient | Flutter (Android/iOS) | Patients | V1 |
| App Web Hôpital | Angular | Médecins, sages-femmes, gynécologues | V1 |
| App Web Pharmacie | Angular | Pharmaciens, préparateurs | V1 |
| App Web Laboratoire | Angular | Biologistes, technologistes | V1 |
| App Web Assurance | Angular | Agents d'assurance maladie | V1 |
| App Web Mairie | Angular | Agents état civil | V2 — hors périmètre actuel |

Toutes consomment la même API Rust/Axum. Le NIP (QR Code) est la clé d'interopérabilité entre les 5 apps.

---

## 6. Modèle de données — résumé (détail complet dans les diagrammes .drawio du dépôt)

**Hiérarchie `Utilisateur`** (héritage réel, pas un champ `role` générique) — **11 sous-classes, chacune avec son propre compte d'inscription/connexion (aucune délégation entre rôles) :**
`Utilisateur` (id, nom, prénom, email, téléphone, motDePasseHash, dateCreation)
→ `Patient` (nip, dateNaissance, sexe, groupeSanguin, adresse)
→ `Medecin` (numeroOrdre, spécialité, structure)
→ `Medecin` porte le champ `specialite` (généraliste, gynécologue, etc.) — les spécialités ne sont jamais des classes séparées
→ `Pharmacien` (numeroOrdre, structure)
→ `Biologiste` (numeroOrdre, structure)
→ `AgentAssurance` (assureur)
→ `Administrateur` (niveauAcces)
→ `Infirmier` (numeroOrdre, structure, specialite) — actions propres : **exécute des prescriptions** (prélèvement, administration de médicaments, pansements). **La sage-femme est une spécialité d'Infirmier, pas une classe séparée** (décision actée — mêmes fonctions qu'un infirmier, appliquées aux femmes enceintes ; détail et impact sur BF-08 dans l'encadré ci-dessous).
→ `AgentAccueil` (structure) — inscription patient, vérification identité, gestion administrative de l'assurance
→ `Major` (structure) — délégué des infirmiers ET des salles : embauche/licenciement d'un infirmier, gestion des salles. Rôle hospitalier, **distinct** de `MajorLaboratoire` ci-dessous.
→ `ChefDeService` (structure) — délégué des médecins, les gère.
→ `Directeur` (structure) — pilote l'hôpital dans son ensemble, rapports, pas d'acte médical.
→ `MajorLaboratoire` (structure) — **statut à confirmer**, et **distinct du `Major` hospitalier** malgré le même mot dans les entretiens (celui-ci gère le stock de réactifs, pas des infirmiers/salles). Ses actions recoupent presque entièrement celles de `Biologiste` + autorité sur le stock. Vérifier avant implémentation si une sous-classe séparée est justifiée ou si un attribut `responsableStock: bool` sur `Biologiste` suffirait.

**`AideSoignant`** (nettoyer le patient, préparer le matériel, nourrir le patient) **existe dans l'organisation hospitalière mais n'a PAS de compte LaafiCare en V1** — décision actée, hors périmètre applicatif, aucune sous-classe à créer.

**Hiérarchie clinique confirmée par recherche terrain du porteur (pas une hypothèse) :** trois niveaux fonctionnels dans un hôpital — Médecin (examine, diagnostique, dossier médical, prescrit examens et traitements), Infirmier (exécute les prescriptions), AideSoignant (soins de confort, hors périmètre app). Les spécialités (Gynécologue, Sage-femme, etc.) sont des valeurs du champ `specialite` à l'intérieur de ces niveaux, jamais des classes séparées. L'encadrement (Major, ChefDeService, Directeur) est un axe orthogonal — pouvoir hiérarchique/administratif, pas un niveau clinique.

**Principe multi-rôles (décision actée, jamais encore reportée nulle part avant cette édition) :** une même personne peut cumuler plusieurs rôles sur des comptes distincts — un médecin peut aussi être patient de LaafiCare (il tombe malade comme n'importe qui). Conséquence sur le modèle : le téléphone reste **unique globalement** au niveau de l'identité (`utilisateur`), mais `utilisateur` n'est plus "une ligne = un rôle exclusif". Chaque table de rôle (`patient`, `medecin`, `infirmier`, etc.) référence `utilisateur_id` en clé étrangère, sans empêcher qu'un même `utilisateur_id` apparaisse dans plusieurs tables de rôle simultanément. Ce n'est donc plus un héritage strict à clé primaire/étrangère partagée entre toutes les sous-classes — c'est une identité centrale (`utilisateur`) avec des rôles rattachés, potentiellement plusieurs par identité. **Conséquence directe sur BF-01 (Section II du CDC) : la formulation "chaque compte doit être rattaché à un rôle unique" est fausse et doit être corrigée** — un compte (une identité) peut porter plusieurs rôles, chacun avec ses propres droits. La mécanique de connexion (quel rôle "actif" après authentification si plusieurs existent) reste à définir plus tard, pas bloquant pour le sprint en cours (Patient/AgentAssurance). **⚠️ Révisé le 2026-09-27/29, voir section 14** : une identité porte des comptes séparés (patient, professionnel, administrateur), chacun avec son mot de passe et son verrouillage ; les rôles professionnels passent sur des *affectations* (compte professionnel + structure + rôle) ; le « rôle actif » est résolu par le choix de l'affectation après connexion.

**⚠️ Correction encore à reporter dans le CDC (Section II, BF-08) — pas encore faite dans le document Word :** le texte actuel attribue à la sage-femme le pouvoir de "déclarer une grossesse à risque" en autonomie. C'est incohérent avec sa reclassification en spécialité d'Infirmier (exécution, pas décision diagnostique). Correction à faire : la sage-femme enregistre les mesures de la visite CPN et exécute les traitements prescrits (fer/acide folique, TPI, VAT) ; l'alerte tension ≥140mmHg reste un déclenchement automatique du système sur la valeur saisie (inchangé, ce n'est pas un jugement humain) ; la déclaration formelle d'une grossesse à risque redevient un acte du Médecin/Gynécologue.

**20 entités "métier" au total** (hors les 4 nouvelles sous-classes de rôle qui s'ajoutent à la hiérarchie Utilisateur), dont : `DossierMedical`, `Allergie`, `Antecedent`, `Structure`, `Consultation`, `Ordonnance`, `LignePrescription`, `Medicament`, `StockMedicament`, `Delivrance`, `DemandeAnalyse`, `ResultatAnalyse`, `Grossesse`, `VisiteCPN`, `Naissance`, `Referencement`, `ContratAssurance`, `DemandeRemboursement`.

**Règles non négociables du modèle :**
- Les allergies s'affichent toujours en priorité, en rouge, dès l'ouverture du dossier
- Un résultat biologique n'est jamais visible avant validation biologique finale (double validation : technicien puis biologiste)
- Les résultats d'examens sensibles sont **bloqués automatiquement** — libération manuelle obligatoire par le biologiste après entretien avec le patient (jamais de notification push/SMS automatique pour ces résultats)
- Une tension artérielle systolique ≥ 140 mmHg lors d'une CPN déclenche une alerte automatique
- Aucun export PDF ni impression de résultat biologique depuis la plateforme

---

## 7. Périmètre fonctionnel V1 (15 BF + 5 BNF)

Voir le CDC complet pour le détail QQOQCCP de chaque besoin. Résumé des BF Must Have :
BF-01 Authentification multi-rôles · BF-02 Dossier patient + NIP · BF-03 Ordonnances/prescriptions · BF-04 Stock pharmacie · BF-05 Substitution par DCI · BF-07 Analyses/résultats biologiques · BF-08 Suivi prénatal (4 CPN) · BF-09 Naissance + NIP nouveau-né · BF-10 Assurance/remboursements automatiques.

Should Have : BF-06 Géolocalisation médicaments · BF-11 Notifications · BF-12 Référencements · BF-13 Offline-first · BF-14 Statistiques · BF-15 Suivi post-opératoire.

Hors périmètre V1 : IA traduction médecin-patient, partogramme numérique, suivi vaccinal enfant 0-18 ans, module Mairie, télémédecine.

---

## 8. Ce qui existe déjà (état d'avancement)

- ✅ CDC v2.0 complet — Sections I à V + Annexes (Word, ~30 pages) — **à mettre à jour pour le pivot Rust/Angular**
- ✅ Diagramme de cas d'utilisation (6 pages .drawio) — acteurs, `«include»`/`«extend»`/`«exclude»`
- ⚠️ Diagramme de classes UML — existe mais **basé sur les 7 rôles obsolètes**, à régénérer avec les 11 rôles (voir section 6)
- ⚠️ Modèle relationnel (MLD) — même remarque, à régénérer
- ✅ 4 diagrammes de séquence — parcours complet NIP, analyse biologique, suivi CPN, référencement (impact des nouveaux rôles à vérifier au cas par cas, pas de refonte totale nécessaire a priori)
- 🔶 Wireframe démarré — 2 écrans mobiles (accueil patient, ordonnances), direction visuelle actée (bleu `#1A56DB` primaire, rouge strictement réservé aux alertes)
- 🔶 Backend Rust, scope MUNASEB (section 12) — en cours, état au 2026-09-27 :
  - Structure : bibliothèque partagée (`src/lib.rs`) + serveur (`src/main.rs`) + outil local d'administration `src/bin/creer_agent_assurance_munaseb.rs` (création de compte agent, jamais une route HTTP).
  - Migrations 0001 à 0010 : utilisateur, OTP, patient, agent MUNASEB, contrat, partenaires/tarifs/demandes, statut du contrat et notifications, périodes d'adhésion, historique des statuts.
  - Routes : patient (OTP, inscription, mot de passe oublié) ; agent (connexion) ; remboursements (`simuler-acte`, liste, détail, prise en charge, validation, rejet, paiement).
  - Pas encore exposé : connexion patient, routes du contrat, routes côté patient, récupération du mot de passe agent (voir « Suite de l'itération », section 12).
- ❌ Frontend Angular et Flutter : rien d'écrit

---

## 9. Consigne de narration — importante

Ne jamais présenter le projet comme ayant déjà eu un prototype ou une version antérieure codée. Le CDC et toute présentation doivent se lire comme une phase de cadrage rigoureuse suivie d'un développement qui démarre maintenant — pas comme la formalisation après coup d'un outil déjà existant.

---

## 10. Ordre de travail demandé par le porteur

1. **Mise à jour du CDC** pour intégrer (a) le pivot Rust/Axum + Angular, (b) la hiérarchie de rôles réactualisée à 12 sous-classes — Infirmier/Sage-femme fusionnés, Major/ChefDeService/Directeur distincts, AgentAccueil, MajorLaboratoire (voir section 6), (c) la correction de BF-08 sur le périmètre d'action de la sage-femme (signalée section 6), et (d) l'authentification à deux stratégies (voir section 11) — Section I.5 (acteurs), Section II (BF-01, BF-08, matrice de traçabilité), annexe glossaire
2. Mise à jour des diagrammes techniques impactés — le diagramme de classes et le modèle relationnel doivent être régénérés avec les 11 rôles ; les diagrammes de séquence à vérifier au cas par cas
3. Maquettes/wireframes (suite de ce qui a été commencé)
4. **Backend Rust — démarrer par le scope MUNASEB isolé (voir section 11 pour le détail exact du périmètre)**, une table/entité à la fois, validée étape par étape (le porteur a explicitement demandé d'aller lentement, un fichier à la fois, pas de génération massive de code sans validation)
5. Frontend Angular (web) et Flutter (mobile)

**Méthode de travail à respecter absolument :** avancer étape par étape, un fichier ou une décision à la fois, expliquer chaque choix, attendre validation avant de continuer. Le porteur a été explicite là-dessus à plusieurs reprises.

**Point de décision encore ouvert à poser au porteur avant d'implémenter `MajorLaboratoire` :** sous-classe séparée de `Utilisateur`, ou attribut `responsableStock` sur `Biologiste` ? Ne pas trancher seul, redemander.

---

## 13. Contrôle du porteur sur toute décision technique

**Aucune décision technique ne se prend sans validation explicite du porteur** — pas seulement les dépendances, mais tout choix ayant plusieurs options raisonnables : nom de champ, structure d'erreur, format de réponse API, stratégie de test, organisation des modules, etc. Avant d'écrire un fichier impliquant un tel choix, présenter les options (1 à 3, avec une recommandation si pertinent) et attendre une réponse. Ne jamais enchaîner deux fichiers sans validation entre les deux.

**Toute modification de fichier passe par l'outil d'édition, jamais par un script Python, `sed` ou une redirection shell** (règle ajoutée le 2026-09-27). Sinon, le porteur ne voit pas les changements ligne par ligne et ne peut pas les refuser. Un gros changement se découpe en plusieurs éditions plutôt que d'être écrit par un script. Les scripts restent permis pour lire ou analyser (extraire un `.docx`, calculer un NIP de test), jamais pour écrire dans le projet.

**README et `docs/` (règle ajoutée le 2026-10-06)** : toute décision qui change le fonctionnement met aussi à jour, dans la même étape, le `README.md` et les fichiers de `docs/` concernés (`securite.md`, `modele-donnees.md`, `api.md`, `points-ouverts.md`, `glossaire.md`). Ces documents s'adressent à quelqu'un qui n'a jamais codé, et ne contiennent ni secret, ni donnée personnelle, ni contenu des documents officiels ; seul Calliste Elie KABORE y figure comme auteur.

**Caractères non ASCII dans un fichier SQL (règle ajoutée le 2026-10-06)** : toujours la forme longue `\UXXXXXXXX`, **jamais `\uXXXX`** — l'outil d'édition remplace d'office `\uXXXX` par le caractère lui-même, ce qui a glissé des espaces invisibles et des tirets trompeurs dans les migrations 0012 et 0013. **Après chaque écriture qui contient des échappements, vérifier les octets** du fichier (lecture par script, permise). Le test sans base `cargo test --test caracteres_migrations` refuse, dans `migrations/*.sql`, tout espace autre que l'espace, la tabulation et la fin de ligne LF, et tout tiret autre que le tiret simple, avec le fichier, la ligne et le code du caractère. **Aucun `\r` dans une migration, même en fin de ligne** : SQLx garde l'empreinte de chaque migration appliquée, fins de ligne comprises, et une migration extraite en CRLF (`core.autocrlf = true`) aurait une autre empreinte — le serveur refuserait de démarrer. Le `.gitattributes` à la racine impose `text eol=lf` à `backend/migrations/*.sql`. En Rust, la forme `\u{…}` avec accolades n'est pas touchée par l'outil.

Après chaque fichier livré, le porteur doit pouvoir **tester lui-même** avant de continuer. Chaque étape doit donc se terminer sur quelque chose d'observable — une route qui répond, une commande `curl`/Postman précise à lancer, un `cargo test` qui passe — jamais sur du code qui ne peut être vérifié qu'en le relisant. Fournir systématiquement : (1) comment lancer/tester ce qui vient d'être fait, (2) ce qu'on doit observer si ça marche.

**Rien ne s'invente — tout doit se référer à la documentation officielle, systématiquement, y compris pour la sécurité.** Que ce soit la conception d'une route API, l'implémentation de l'authentification, le hachage de mot de passe, la génération/validation JWT, la configuration SQLx, ou tout autre point technique : consulter la documentation officielle du crate ou de la technologie concernée avant d'écrire le code, et pouvoir citer d'où vient un choix (nom du crate, section de la doc, exemple officiel suivi). Ceci s'applique avec une rigueur particulière à tout ce qui touche la sécurité et l'authentification (BF-01, BNF-01, section 11 de ce document) — un système de santé ne tolère aucune improvisation sur ces points : pas de pattern "recopié de mémoire" ou "qui devrait marcher", uniquement ce que la documentation officielle recommande explicitement pour l'usage visé.

**Chaque choix technique non trivial doit être commenté directement dans le code**, pas seulement expliqué dans la conversation. Le commentaire doit dire *pourquoi* ce choix a été fait, pas répéter ce que le code fait déjà lisiblement. Pour tout ce qui touche à la sécurité/auth en particulier, citer la source (crate + doc officielle suivie). Exemples du niveau attendu :

```rust
// Argon2id recommandé par la doc officielle du crate `argon2` pour le hachage
// de mot de passe (résistant aux attaques GPU, contrairement à bcrypt/SHA).
// Voir https://docs.rs/argon2
let hash = argon2.hash_password(password.as_bytes(), &salt)?;

// Verrouillage 5 tentatives / 15 min — exigence BF-01 du CDC, pas un choix
// arbitraire de la librairie.
const MAX_TENTATIVES: i32 = 5;
```

Un commentaire du type `// hash le mot de passe` au-dessus d'une ligne qui hache visiblement un mot de passe n'apporte rien et n'est pas ce qui est demandé ici — le but est de tracer la *justification*, pas de paraphraser le code.

---

## 11. Authentification à deux stratégies (décision actée — révisée 2026-09-23 : l'OTP quitte la connexion courante)

> **⚠️ Section révisée par la section 14 (2026-09-27/29).** Ce qui change : le mot de passe et le verrouillage quittent `utilisateur` pour aller sur chaque compte ; les professionnels utilisent l'email pour tout (connexion et, plus tard, récupération par email) et leur téléphone n'est plus qu'une coordonnée ; **l'OTP ne sert plus qu'au compte patient** (inscription et récupération) ; il n'y a plus de récupération professionnelle par OTP (fonctions supprimées) — en attendant l'email, un outil local réinitialise le mot de passe d'un compte professionnel. Les passages ci-dessous sur la récupération professionnelle par OTP et sur l'asymétrie `AuthProfessionnelService` sont donc obsolètes ; le reste (Argon2id, verrouillage BF-01, flux patient, aucune information révélée sur un compte avant authentification) reste valable.

`Utilisateur` (base) porte deux façons de s'authentifier selon le rôle — **ne jamais implémenter une seule stratégie unifiée** :

| | `Patient` | Tous les autres rôles (Medecin, Pharmacien, AgentAssurance, etc.) |
|---|---|---|
| Identifiant de connexion | `telephone` | `email` |
| Secret en connexion normale | `motDePasseHash` (Argon2id) — **depuis cette révision, identique aux autres rôles** ; l'OTP n'intervient plus ici | `motDePasseHash` (Argon2id — décision actée, remplace BCrypt initialement prévu en BNF-01 ; voir doc OWASP/crate `argon2`, paramètres explicites m=19456, t=2, p=1, crate 0.6.0) |
| Verrouillage 5 tentatives / 15 min (BF-01) | **S'applique désormais au Patient aussi** — ce n'était pas le cas dans la version précédente de cette section | S'applique |
| Champs `email` / `motDePasseHash` sur le compte | `motDePasseHash` obligatoire dès la création du compte (voir flux ci-dessous) ; `email` reste optionnel | obligatoires |

**Rôle de l'OTP, redéfini.** L'OTP n'est plus utilisé à chaque connexion patient. Il sert uniquement à deux moments : **(a)** la création de compte — vérifier que le numéro appartient bien au demandeur avant que le compte existe — et **(b)** la récupération de mot de passe oublié. `otp.rs` (déjà codé) fonctionne sans changement — ce qui change, c'est *quand* il est appelé, pas son fonctionnement interne. Il n'est **jamais exclusif au Patient** : les autres rôles s'en servent aussi pour leur propre récupération de mot de passe (voir plus bas).

**Décision actée : aucun affichage du nom associé à un numéro avant authentification, dans aucun flux.** Une version antérieure de ce document prévoyait d'afficher le nom du titulaire dès la saisie du téléphone, pour confirmation visuelle avant l'envoi de l'OTP. Retiré : le bénéfice UX (attraper une faute de frappe) ne justifie pas de révéler à quiconque tape un numéro si une personne est inscrite et son nom. Le numéro seul déclenche l'OTP ou une tentative de connexion par mot de passe — aucune information sur le compte n'est renvoyée avant authentification réussie.

**Flux de création de compte patient :**
1. Le formulaire (`telephone`, `nom`, `prenom`, `dateNaissance`, `lieuNaissance`) reste **côté client** — rien n'est stocké côté serveur avant validation de l'OTP.
2. OTP envoyé au numéro saisi, puis validé.
3. **Un seul appel serveur, atomique** : crée le compte, génère le NIP (BF-02 — nouveau format 2026-09-23 : 16 chiffres purement numériques, 15 de séquence globale + 1 chiffre de contrôle Luhn, plus de `BF-AAAA-XXXXXX`), enregistre les informations du formulaire.
4. La personne définit immédiatement son mot de passe.
5. Session ouverte (JWT émis).

**Flux mot de passe oublié — Patient :** demande d'OTP → validation du code → saisie d'un nouveau mot de passe.

**Flux mot de passe oublié — rôles non-patients :** le choix entre les deux voies est laissé à la personne **au moment de la demande**, jamais imposé :
- **Voie OTP** : réutilise `otp.rs` tel quel.
- **Voie email (lien de réinitialisation)** : brique neuve, **aucune dépendance d'envoi d'email choisie à ce jour**. Comme tout ajout au `Cargo.toml` (section 13) : présenter les options avec sources officielles avant d'ajouter quoi que ce soit. **Non urgent — l'implémentation reste concentrée sur le Patient d'abord.**

**⚠️ Point ouvert, à trancher au moment d'implémenter le token de réinitialisation par email (ne pas copier le fonctionnement de l'OTP sans vérifier) :** un token de réinitialisation n'a pas le même espace de valeurs qu'un code à 6 chiffres (10⁶) — les choix faits pour l'OTP (hachage SHA-256 suffisant, pas de comparaison en temps constant, sel = `utilisateur_id`) reposaient sur cette faible entropie et cette courte durée de vie. Vérifier la doc/les bonnes pratiques pour ce nouveau cas plutôt que de supposer que « même mécanisme, juste un autre canal ».

**Implémentation actée (code déjà écrit, 2026-09-23) — asymétrie patient/professionnel, à ne pas reperdre une troisième fois :**
- `AuthPatientService` (les trois flux patient ci-dessus) **émet directement le JWT** (`role: "patient"` fixe) — cohérent avec le fait qu'aucune table de rôle séparée n'existe pour Patient, le rôle est connu d'avance.
- `AuthProfessionnelService::se_connecter` (email + mot de passe + verrouillage) **ne connaît aucun rôle et n'émet pas de JWT** — il renvoie l'`utilisateur_id` vérifié. Raison : ce service est construit *avant* qu'aucune table de rôle non-Patient (`AgentAssurance`, prochaine étape du plan, section 12) n'existe — il ne peut donc pas savoir quel `role` mettre dans le jeton. C'est au futur service spécifique au rôle (ex. un service `AgentAssurance` à venir) de confirmer l'appartenance à ce rôle puis d'appeler `JwtService::emettre(id, role)` — le même `JwtService` pour tous les rôles, mais jamais appelé directement par `AuthProfessionnelService`.
- Même asymétrie sur la récupération : `AuthProfessionnelService::demander_otp_recuperation` / `reinitialiser_via_otp` restent au niveau `utilisateur` seul, sans rôle, pour la même raison.

**⚠️ Point à respecter à l'implémentation de la route HTTP de récupération professionnelle (pas encore codée) :** la réponse doit être identique **en forme et en temps de traitement**, que l'email existe ou non — un temps de réponse différent entre les deux cas (ex. écriture d'un OTP en base seulement si l'email existe) trahirait par le *timing* l'information que l'uniformité de la réponse cherche justement à cacher. `demander_otp_recuperation` renvoie déjà `Ok(None)` de façon uniforme côté type, mais ça ne suffit pas à soi seul : la route devra égaliser le temps d'exécution des deux branches (ex. exécuter un travail équivalent — un faux hachage, une requête factice — dans le cas « email inconnu », plutôt que de renvoyer immédiatement).

Champ du modèle `Patient` : `lieuNaissance` (obligatoire à l'inscription, avec `telephone`, `nom`, `prenom`, `dateNaissance`) — inchangé.

---

## 12. Scope de démarrage backend — MUNASEB isolé

**Décision actée :** on ne code pas "un module Assurance générique" — MUNASEB est une assurance spécifique, câblée en dur pour l'instant. Le module est un service greffé sur LaafiCare, jamais une dépendance dure : un `Patient` sans `ContratAssurance` doit pouvoir utiliser tout le reste de la plateforme sans blocage. Pas d'entité "Assuré" séparée — un assuré MUNASEB est un `Patient` comme un autre, avec en plus un `ContratAssurance` accroché à son NIP.

**Ordre de code exact pour cette itération, rien d'autre :**

1. Squelette Axum + connexion PostgreSQL (SQLx)
2. `Utilisateur` (base, avec les deux stratégies d'auth de la section 11)
3. `Patient` — version minimale : NIP, nom, prénom, téléphone, dateNaissance, lieuNaissance. Pas de dossier médical, pas d'allergies, pas d'antécédents à ce stade — ils appartiennent à un autre module.
4. ✅ Agent MUNASEB — **⚠️ à reprendre selon la section 14** (la table `agent_assurance_munaseb` et l'outil `creer_agent_assurance_munaseb` disparaissent au profit des affectations et des invitations). État actuel : table `agent_assurance_munaseb` (nom décidé le 2026-09-23 : MUNASEB est câblée en dur, le nom générique `agent_assurance` est réservé au futur module Assurance), auth email + mot de passe standard, rôle JWT `agent_assurance_munaseb`. **V1 : un seul rôle, pas de sous-rôles** (Régie des recettes / Liquidation / Médecin Conseillé / Finance / Directeur du mémoire MUNASEB ne sont PAS répliqués en comptes séparés pour l'instant — un seul agent couvre tout le pipeline). Comptes créés par l'outil local `creer_agent_assurance_munaseb` : dans le mémoire, la création des comptes agents est une fonction d'Administrateur, jamais une auto-inscription (l. 843-847).
5. 🔶 Contrat MUNASEB — table `contrat_assurance_munaseb` et ses périodes. Fait : création avec carence, renouvellement, vérification des droits (plafond restant par période ; le taux dépend du type d'acte, il est dans la grille tarifaire). **Suspension : à concevoir** (colonne `statut` présente, ni fonction, ni route, ni règle de notification). **Aucune route encore.**
6. ✅ Demande de remboursement — `creer_depuis_acte`, transitions, lectures, historique (voir ci-dessous).

**Point d'entrée critique à respecter — frontière propre avec les modules futurs :**
`demande_remboursement_munaseb::creer_depuis_acte(pool, acte)` (écrit `creerDepuisActe` dans les premières versions de ce document) est l'unique point d'entrée métier, complet et définitif, pour la génération automatique d'une demande (BF-10). Les modules Hôpital/Pharmacie/Laboratoire n'existent pas encore et ne peuvent donc rien déclencher automatiquement. En attendant, la route de test manuel `POST /api/assurance-munaseb/remboursements/simuler-acte`, réservée aux agents (JWT agent), appelle ce même service. Quand les autres modules seront codés, ils appelleront `creer_depuis_acte` directement — **zéro réécriture de la logique Assurance à ce moment-là.**

**Décisions actées — `DemandeRemboursement` (2026-09-24).** Sources : `docs/Memoire_Soutenance1.docx` et `docs/diagrammes/diagramme_classes_munaseb oki11.drawio` uniquement. Ces décisions tranchent les ambiguïtés et contradictions relevées dans le mémoire.
- **Statuts** : 5 statuts, cycle `En attente → En cours → Validé / Rejeté → Payé` (mémoire l. 815). **Statut initial : En attente.** Cela tranche la contradiction avec le Tableau 9, qui crée la demande directement « en cours de traitement ».
- **Taux** : fixes, **aucun ajustement manuel** par un médecin conseil. Cela écarte la lecture « 80 % max » du mémoire (l. 758, dictionnaire). Ils **varient selon le type d'acte** : médicaments, pharmacie et labo à 80 % ; les soins à leur propre taux (hospitalisation 100 %, consultation gratuite, mémoire l. 489). **Une table par type d'acte** porte **soit un taux, soit un forfait en montant fixe** (ex. lunetterie 15 000 FCFA, l. 502). **Le type d'acte reste une énumération fermée.**
- **Dépassement du plafond** : le montant est **plafonné au solde restant**. Une demande n'est **jamais rejetée pour ce seul motif**.
- **Année du plafond** : **année d'adhésion** (période de validité du contrat), pas l'année civile. **Seuls les montants Validé et Payé consomment le plafond.**
- **Arrondi** : à l'entier FCFA le plus proche (pas de sous-unité en circulation).
- **Périodes d'adhésion et renouvellement (décidé par le porteur, 2026-09-24)** :
  - Chaque période d'adhésion est **conservée**. Un renouvellement **crée une nouvelle période** et n'écrase jamais les dates de l'ancienne.
  - **Première adhésion** : **carence d'un mois**. La période commence un mois après la date de paiement.
  - **Renouvellement dans les temps** (payé au plus tard le dernier jour de la période en cours) : la nouvelle période commence **le lendemain de la fin**, **sans carence**. Ni chevauchement, ni jour perdu. Pendant un temps, le patient a donc deux périodes : celle en cours et la suivante, déjà payée.
  - **Renouvellement en retard** (payé après la fin de la dernière période) : **carence d'un mois** aussi. La période commence un mois après le paiement. Les jours entre l'expiration et le début de la nouvelle période restent **non couverts, sans effet rétroactif**. Sinon, un adhérent pourrait laisser expirer son contrat et payer seulement le jour où il tombe malade.
  - **Durée** : dans tous les cas, la période dure **12 mois à partir de son début**. La carence ne réduit pas la durée de couverture.
  - **Date de paiement (V1)** : **jamais saisie par l'appelant**. C'est la **date du jour, fixée par le système** au moment de l'enregistrement. `creer` et `renouveler` ne prennent pas de paramètre `date_paiement`. Raison : toute la règle « dans les temps / en retard » dépend de cette date. Un paiement en retard antidaté à la veille de l'expiration, par erreur ou volontairement, ferait disparaître la carence. À revoir quand l'API Mobile Money fournira la vraie date de transaction.
  - **Pendant la carence**, le patient n'a qu'une période future et aucune en cours. Il ne doit pas recevoir « contrat expiré » mais une information distincte (en carence), avec la date de début de couverture.
  - **Rattachement** : chaque demande est rattachée **dès sa création** à la période qui contient la date de l'acte. Le plafond de 100 000 FCFA se calcule **par période**.
  - **Deux périodes d'un même patient ne se chevauchent jamais.** La garantie doit venir de la base elle-même, pas seulement du code.
- **Statut du contrat (décision LaafiCare, hors sources MUNASEB)** : `contrat_assurance_munaseb.statut` vaut `actif` ou `suspendu` (migration 0008). La carte d'adhésion du mémoire n'a qu'une date d'expiration : ce champ est donc un ajout du porteur (2026-09-24), pas une reprise du système source. `creer_depuis_acte` exige un contrat **actif** et une date d'acte comprise dans sa période ; sinon il renvoie `NonCouvert`. Un **partenaire suspendu** renvoie une erreur distincte, `PartenaireSuspendu`, pour que le patient ne croie pas qu'il n'est plus assuré. Aucune route de suspension n'existe encore.
- **Notifications de statut (exception assumée à BF-11)** : chaque changement de statut d'une demande, création comprise, crée une **notification enregistrée en base**, que l'app patient affichera. **Pas de SMS, ni en secours** : ces notifications sont informatives et non urgentes. **Le SMS reste réservé à l'OTP.** Pas de push pour l'instant, faute d'infrastructure. Il viendra plus tard par-dessus, sans réécrire les appelants. Trois conditions : la notification est créée **après le commit** de la demande ; un échec de notification est journalisé et **n'annule jamais** la demande ; le texte reste **minimal**, sans type d'acte ni nom du partenaire, car un push futur s'afficherait sur l'écran verrouillé.
- **Circuit de `creer_depuis_acte`** : c'est le **tiers payant**. L'acte est enregistré par une structure partenaire, et la demande est générée pour son compte (appui : mémoire l. 1077, un partenaire peut formuler une demande de remboursement). Le **paiement direct** (le mutualiste dépose lui-même sa demande, l. 499/563) est un **autre flux, hors périmètre pour l'instant**.

**Précisions de périmètre (2026-09-24), pour plus tard :**
- **Partenaires** : les agents les ajouteront eux-mêmes quand on fera les modules Pharmacie et Laboratoire. D'ici là, `partenaire_sante_munaseb` reste minimale, avec seulement des données de test.
- **Tarifs** : la **liste** des types d'actes reste fermée et **définie par le code** (CHECK SQL + enum Rust). Elle sera étendue par migration quand l'énumération des soins sera complète. Les **valeurs** (taux ou forfait) seront **modifiables par les agents**. Elles ne doivent donc exister **qu'en base, jamais dans une constante Rust**. Le montant calculé à la création d'une demande est **figé** : une modification de tarif ne touche jamais les demandes existantes. Quand les routes de gestion des tarifs arriveront, **chaque modification devra être tracée** (auteur, date, ancienne et nouvelle valeur).

**Décisions actées — transitions et lectures des demandes (2026-09-25/26)** :
- **Transitions** (un seul rôle agent, V1) : En attente → En cours (prise en charge) ; En cours → Validé ; En attente **ou** En cours → Rejeté (T2 : la liquidation peut refuser dès la vérification des pièces) ; Validé → Payé (marquage manuel, sans Mobile Money pour l'instant). Tout autre passage renvoie `transition_invalide` (409) avec le `statut_actuel` de la demande.
- **Validation** : montant remboursé = montant demandé plafonné au solde de la période de la demande, **0 FCFA si le solde est épuisé** (T1 a — jamais de rejet pour ce seul motif, et un refus laisserait la demande bloquée pour toujours). Une demande validée à 0 FCFA passe quand même par Payé (T3 : Payé = dossier clos).
- **Notifications** : validation à 0 FCFA → texte distinct « Votre demande a été traitée : le plafond de votre période est atteint, aucun montant ne sera versé. », pour que le patient ne croie pas être remboursé ; **aucune notification** au passage à Payé d'une demande à 0 FCFA.
- **Rejet** : motif obligatoire, non vide, **1 000 caractères au plus** (garde-fou technique). Il est enregistré sur la demande, jamais repris dans la notification.
- **Historique** (`historique_statut_demande`, migration 0010) : une ligne par changement de statut, création comprise, avec l'agent qui l'a fait ; lignes **jamais modifiées ni supprimées** (triggers). Une demande qui a un historique ne peut plus être supprimée.
- **Liste pour l'agent** : filtrable par statut, **pagination par curseur** sur l'identifiant (`uuidv7`, time-ordered selon la doc PostgreSQL 18), 50 par défaut, 100 au maximum. Raison : avec un décalage, une demande prise en charge décale les suivantes et une demande peut glisser entre deux pages sans être vue, ce qui compromet l'objectif de 72 h. Limite acceptée : la doc ne garantit pas un ordre strict entre deux connexions à la même milliseconde, une demande créée pendant le parcours peut n'apparaître qu'au rechargement.
- **Ce que voit l'agent** : nom, prénom et numéro de carte MUNASEB du patient, **jamais son NIP ni son téléphone** (inutiles à l'instruction du dossier). Le détail d'une demande ajoute le motif, la période, le solde restant (indicatif, recalculé sous verrou à la validation) et l'historique avec le nom et le prénom de l'agent.

**Format d'erreur unique de toute l'API (décidé le 2026-09-24)** : chaque réponse d'erreur a la forme `{"erreur": "<message lisible>", "code": "<code_machine>"}`, avec des champs complémentaires si le cas l'exige (ex. `"debut_couverture"` pour `en_carence`). Le `code` est ce que les apps Angular et Flutter testent, jamais le message. Introduit avec la route `simuler-acte`. **Alignement fait (2026-09-25)** sur toutes les routes existantes (patient, connexion agent, remboursements) et sur les rejets d'axum eux-mêmes, via `src/erreur_api.rs` : `JsonApi` (corps JSON), `CheminApi` (paramètre d'URL), `RequeteApi` (paramètres de requête). Le texte d'un rejet d'axum va dans la réponse, jamais dans les logs (il peut reprendre une donnée de santé saisie par erreur). La fonctionnalité `serde` de la crate `uuid` est activée : les identifiants se lisent directement en JSON et dans les URL.

**Ordre des verrous (règle permanente, 2026-09-25)** : tout code qui verrouille à la fois une demande de remboursement et sa période d'adhésion (`SELECT … FOR UPDATE`) doit **toujours verrouiller la demande d'abord, puis la période**. Sinon, deux transactions qui prennent ces verrous dans l'ordre inverse peuvent se bloquer mutuellement (interblocage).

**Exigence pour la production (2026-09-25)** : l'application doit se connecter avec un **rôle PostgreSQL sans droits d'administration** (ni superuser, ni propriétaire des tables). Sinon, le trigger qui rend `historique_statut_demande` non modifiable (migration 0010) peut être désactivé, et l'historique perd sa garantie. En développement, l'utilisateur `laaficare` du docker-compose est superuser : la protection n'y vaut que contre les erreurs, pas contre un administrateur.

**Tests et environnement (2026-09-25)** :
- Les tests en base utilisent **`#[sqlx::test]`** : une base temporaire par test, migrations appliquées automatiquement, supprimée si le test réussit. La base de développement n'est jamais touchée. Ils gardent `#[ignore]` (il faut un PostgreSQL démarré) et se lancent avec `cargo test -- --include-ignored`.
- `DATABASE_URL` pointe sur **`127.0.0.1`, jamais `localhost`** : sous Windows, `localhost` se résout d'abord en `::1`, or le docker-compose ne publie PostgreSQL que sur `127.0.0.1:5433` ; la tentative IPv6 est refusée au bout d'environ 2 s (2 087 ms mesurés) à chaque nouvelle connexion.
- Depuis la migration 0010, les données de test d'une base de développement ne peuvent plus être supprimées dès qu'une demande a été créée (historique non modifiable, clés étrangères en chaîne). Seule une réinitialisation du volume Docker (`docker compose down -v`) repart d'une base vide, en effaçant tout.

**Suite de l'itération (ordre validé le 2026-09-27, révisé le 2026-09-29 : backoffice d'abord)** — chaque étape présentée avant d'être écrite :
1. ✅ Mise à jour de ce document.
2. ✅ Conversion des tests en base restants en `#[sqlx::test]`.
3. **Backoffice (section 14)**, ordonné pour que les parcours redeviennent testables avec Postman au plus tôt (révisé le 2026-09-29) :
   1. ✅ Migration 0011 (identité et comptes).
   2. ✅ `verrouillage.rs` ; `mot_de_passe.rs` (constantes 8 / 128, normalisation NFC, longueur en caractères, règle de composition `controler`).
   3. ✅ Authentification patient sur les comptes, **connexion patient comprise** (décision P1) → inscription, connexion, verrouillage et mot de passe oublié testables.
   4. ✅ Migration 0012 (structures, pièces, registre des autorisations, décisions, affectations, invitations).
   5. **Second facteur TOTP** : conception présentée et validée (RFC 6238, crate, chiffrement du secret, codes de secours…), puis écriture. Obligatoirement **avant** l'outil du premier administrateur, qui doit l'activer dès la création.
   6. Authentification professionnelle sur les comptes, forme du jeton, outils locaux (premier administrateur avec TOTP, réinitialisation) → connexion administrateur testable.
   7. Création de structure, brouillon, téléversement des pièces, soumission → parcours du responsable testable.
   8. Décisions de l'administrateur (valider, refuser, suspendre) → licence testable.
   9. Choix de l'affectation après connexion et extracteurs (gérer / exercer), routes MUNASEB rebranchées sur l'affectation → remboursements de nouveau testables.
   10. Invitations (création, SMS, acceptation, refus, annulation, expiration) → rattachement d'un agent testable.
   11. Désactivation d'une affectation ; bascule.
4. *(fusionnée dans l'étape 3.3 : connexion patient)*
5. Routes du contrat côté agent (adhésion avec carence, renouvellement, consultation des droits), en commençant par la conception de la suspension. **Au même moment : regrouper le calcul du solde, aujourd'hui en double dans `solde_periode` (demandes) et `verifier_droits` (contrat).**
6. Routes côté patient : ses droits (plafond restant, carence), ses demandes et leur statut, ses notifications (lecture, marquer comme lue).
7. Bilan de fin d'itération : types d'actes sans valeur dans le mémoire (dentaire et prothèses, accouchement et CPN, chirurgie, frais funéraires, radiologie), purge des inscriptions patient abandonnées au-delà de 24 h, et ce qui part dans l'itération suivante.

(L'ancienne étape « récupération du mot de passe agent par OTP » est supprimée : les professionnels n'utilisent plus l'OTP, voir section 14.)

**Hors scope de cette itération, à ne pas toucher :** Medecin, Pharmacien, Biologiste, Infirmier (dont la spécialité sage-femme), AgentAccueil, Major, ChefDeService, Directeur, MajorLaboratoire, Ordonnance, DossierMedical, Allergie, tout le module CPN. Ils reviendront dans une itération ultérieure, une fois MUNASEB fonctionnel de bout en bout.

---

## 14. Backoffice, comptes et structures (décisions du 2026-09-27 au 2026-09-29)

**Identité et comptes**
- `utilisateur` porte l'identité : nom, prénom, **téléphone unique vérifié par OTP** (via le compte patient), email.
- Une identité porte des **comptes séparés** : `patient`, `professionnel`, `administrateur_laaficare`. Chaque compte a **son propre mot de passe, son verrouillage (5 tentatives / 15 min, BF-01), son statut et sa `version_jeton`**. Un blocage du compte patient ne touche jamais le compte professionnel, et inversement.
- **Tout professionnel a d'abord un compte patient** : son identité et son téléphone vérifié sont ceux de ce compte.
- **Les professionnels utilisent l'email pour tout** (connexion, et plus tard récupération par email). Leur téléphone n'est qu'une coordonnée. **L'OTP ne sert plus qu'au compte patient** (inscription, récupération). En attendant l'envoi d'email, un **outil local** réinitialise le mot de passe d'un compte professionnel.
- **Normalisation du numéro, côté serveur (décisions C3, T1 à T4, 2026-10-03)** — module `telephone.rs`, appliqué à la demande d'OTP, à l'inscription, à la connexion et à la réinitialisation (puis aux invitations) ; le SMS part toujours vers le numéro normalisé. Règle : retirer les espaces (au sens Unicode), les points et tous les tirets (propriété Unicode *Dash*, 31 caractères) — jamais les parenthèses ; 8 chiffres → `+226` devant ; `00226` + 8 chiffres ou `226` + 8 chiffres → `+226` + 8 chiffres ; `+226` exige exactement 8 chiffres après ; un autre `+` (numéro étranger) exige 7 à 15 chiffres ; tout le reste → **422 `telephone_invalide`** (une erreur de format ne dit rien d'un compte). Source : plan de numérotage du Burkina Faso publié par l'UIT (communication de l'ARCEP du 4.V.2023), numéro national de 8 chiffres ; le 0 initial de certains numéros nationaux (03, 05, 06…) fait partie du numéro et n'est jamais retiré.
- **Connexion patient (P1)** : une seule réponse `identifiants_invalides` pour un numéro inconnu, une inscription inachevée ou un mauvais mot de passe, avec le même message court (voir ci-dessous) ; un faux hachage Argon2id égalise le temps de réponse quand aucun compte n'existe. Aucune information sur l'existence d'un compte (section 11).
- **Message de connexion refusée (2026-10-03)**, court et identique dans tous les cas : **« Numéro ou mot de passe incorrect. »** pour le patient, **« Email ou mot de passe incorrect. »** pour les professionnels. **Pour le frontend** : les actions passent par des boutons, **« Mot de passe oublié ? »** et **« Créer un compte »** (patient). « Mot de passe oublié ? » couvre aussi le compte verrouillé, puisque la réinitialisation lève le verrouillage.
- **Compte verrouillé (L1, 2026-10-02), patients et professionnels** : il répond lui aussi `identifiants_invalides` (401), après un faux hachage, avec le même message. **Plus de réponse 423** : elle révélait l'existence d'un compte, puisqu'un numéro ou un email inconnu ne se verrouille jamais. Cela remplace le 423 validé plus tôt pour la connexion professionnelle.
- **Verrouillage (`verrouillage.rs`, seule implémentation de BF-01 pour tous les comptes)** : la tentative est comptée par la base **avant** la vérification du mot de passe, en une seule requête qui pose aussi le verrouillage au 5ᵉ échec ; au plus 5 tentatives passent, même simultanées. Un compte désactivé répond `identifiants_invalides` après vérification du mot de passe (même temps de réponse).

**Mots de passe (décisions du 2026-10-02), tous les comptes** — référence NIST SP 800-63B **révision 4** (26 août 2025, §3.1.1.2), qui remplace la révision 3 citée auparavant :
- **Normalisation NFC avant hachage**, à la création comme à la vérification (crate `unicode-normalization` 0.1.25). Un même mot de passe accentué peut être codé différemment selon le clavier ; sans normalisation, le patient ne pourrait plus se connecter depuis un autre appareil.
- **Longueur comptée en caractères** (points de code), **après normalisation** — jamais en octets.
- **8 caractères minimum, 128 maximum** (la révision 4 demande d'accepter au moins 64).
- **Au moins une lettre, un chiffre et un caractère spécial** (définition validée le 2026-10-03) : **lettre** = propriété Unicode *Alphabetic* (`is_alphabetic`), lettres accentuées et non latines comprises ; **chiffre** = 0 à 9 seulement ; **caractère spécial** = tout le reste, sauf les espaces (**permises mais non comptées**) et les caractères de contrôle (**refusés** : tabulation, retour à la ligne…). **Aucune espace n'est retirée** : le mot de passe est pris tel que saisi. Contrôlé seulement à la création ou au changement d'un mot de passe, jamais à la connexion. Une seule erreur, `mot_de_passe_non_conforme`, avec **toutes** les règles non respectées dans `regles_non_respectees` (`longueur_min`, `longueur_max`, `caractere_de_controle`, `lettre`, `chiffre`, `caractere_special`), pour que l'app affiche une liste à cocher.
- **Pour l'interface (plus tard)** : l'app conseille des symboles comme `!`, `@` ou `#` plutôt que des emojis, qui peuvent s'écrire différemment d'un téléphone à l'autre.
- **Deux écarts assumés à la révision 4, décidés par le porteur** : (1) elle interdit d'imposer un mélange de types de caractères (« SHALL NOT impose other composition rules ») ; (2) elle exige 15 caractères pour un mot de passe utilisé comme seul facteur, et n'autorise 8 qu'avec un second facteur — or le TOTP reste facultatif pour les patients et les professionnels. Ces écarts sont aussi documentés dans le code (`mot_de_passe.rs`).
- **Liste de mots de passe interdits** (exigée par la révision 4, « SHALL ») : **non faite, point ouvert à traiter avant la production**, avec présentation des options documentées.

**Second facteur (TOTP, compatible Google Authenticator), décidé le 2026-10-02** : **facultatif** pour les patients et les professionnels, **obligatoire pour les administrateurs LaafiCare** (ils valident les structures et accordent les licences). Activé dès la création par l'outil du premier administrateur.

**Conception du TOTP (décisions Z1 à Z10, 2026-10-05)** — sources : RFC 6238 ; NIST SP 800-63B rév. 4, §3.1.2 (codes de secours), §3.1.4 (OTP), §3.2.2 (limitation des essais) ; doc des crates.
- **Crates (Z1, Z4)** : `totp-rs` 6.0.0 (fonctionnalités `otpauth`, `gen_secret`, `zeroize`) et `aes-gcm` 0.11.1. **Versions stables vérifiées sur crates.io le 2026-10-05** : non retirées, sans suffixe de pré-version, et aucune de leurs dépendances n'exige de pré-version. `totp-rs` 6.0.0 demande Rust 1.88 au moins, `aes-gcm` 1.85 (installé : 1.98.1). Jamais de HOTP/TOTP codé à la main.
- **Paramètres** : SHA-1, 6 chiffres, pas de 30 s (défauts de la RFC et de la crate ; certaines applications reviennent silencieusement à SHA-1). Secret de 160 bits par `Secret::generate()` (CSPRNG, taille recommandée par la RFC 4226 ; NIST exige 112 bits au moins).
- **Tolérance sur l'heure (Z3)** : `skew = 1`, soit le pas précédent, l'actuel et le suivant (environ 90 s). NIST demande de tenir compte de la dérive d'horloge dans les deux sens.
- **Code déjà utilisé refusé** (RFC 6238 « MUST NOT accept the second attempt », NIST « SHALL accept a given OTP only once ») : `Totp::check` renvoie le pas qui a validé le code ; il est enregistré, et un code n'est accepté que si son pas est strictement plus grand que le dernier utilisé, en une seule requête atomique.
- **Essais limités** : chaque essai de code, ou de code de secours, compte dans le **même compteur que le mot de passe** du compte (5 essais / 15 min, BF-01), compté avant la vérification.
- **Chiffrement du secret (Z4)** : AES-256-GCM ; clé de 32 octets dans une **variable d'environnement dédiée**, distincte du secret JWT ; nonce aléatoire de 96 bits par chiffrement, enregistré avec le texte chiffré ; **identifiant du compte en donnée associée** (un secret recopié sur un autre compte ne se déchiffre pas) ; colonne `version_cle` pour un futur changement de clé.
- **QR code (Z2)** : le serveur renvoie l'URL `otpauth://` et la clé en base 32 ; Angular et Flutter dessinent le QR. Les outils locaux affichent la clé à saisir à la main, jamais écrite dans un fichier.
- **Stockage (Z5)** : tables séparées pour le second facteur (une ligne par compte) et les codes de secours.
- **Codes de secours (Z6)** : 10 codes de 10 caractères en base 32 (environ 50 bits), affichés `XXXXX-XXXXX`, montrés une seule fois, hachés en Argon2id (sous 112 bits, NIST exige un hachage de mot de passe salé), à usage unique.
- **Activation et désactivation par la personne (Z7)** : l'activation demande le mot de passe et n'est effective qu'après un premier code correct. Un administrateur ne peut jamais désactiver son TOTP, seulement le remplacer. **En V1, la personne ne désactive pas elle-même son TOTP** (voir Z9).
- **Nom dans l'application d'authentification (Z8)** : émetteur `LaafiCare`, compte « Patient », « Professionnel » ou « Administrateur » ; aucune donnée personnelle dans l'URL, qui peut finir dans une sauvegarde en ligne du téléphone.
- **Connexion en deux temps (Z10)** : quand le TOTP est actif, un mot de passe correct donne un jeton intermédiaire de courte durée, accepté seulement par la route qui reçoit le code. Sa forme est définie à l'étape 3.6.
- **Téléphone perdu, plus aucun code de secours (Z9)** : en V1, **seule l'équipe LaafiCare désactive un TOTP**, pour tous les types de comptes, avec l'outil local `desactiver_totp`.
  - La **réinitialisation du mot de passe par OTP ne touche jamais au TOTP**. Sinon, un vol de carte SIM suffirait à retirer le second facteur.
  - L'outil exige que le membre de l'équipe **s'authentifie avec son propre compte administrateur** (email, mot de passe et code TOTP), pour que la trace désigne la bonne personne.
  - **Chaque désactivation est tracée**, sans jamais être modifiée ni supprimée : compte concerné, administrateur, date, motif, et mention que la **pièce d'identité (CNIB) a été vérifiée**.
  - **À prévoir à l'étape 3.6 (2026-10-06)** : un administrateur dont le TOTP a été désactivé ne peut plus se connecter (TOTP obligatoire), donc ne peut pas en activer un nouveau. Il faut un **jeton de courte durée, sur le modèle de Z10**, délivré après le mot de passe, qui ne permet **que l'activation d'un nouveau TOTP**.
- **Migration 0013 (décisions Y1, Y3, Y4, 2026-10-06)** :
  - **Y1** : un TOTP désactivé ou remplacé voit **sa ligne supprimée** (secret chiffré détruit, codes de secours emportés en cascade) ; la trace reste dans `desactivation_totp`. Garder un secret dont plus personne ne se sert n'apporte rien et reste un risque.
  - **Y3, complété par Y3-bis (2026-10-06)** : un administrateur ne peut désactiver le TOTP d'**aucun de ses propres comptes** (administrateur, patient ou professionnel) — **refusé par la base**, par un trigger avant insertion qui compare l'`utilisateur_id` du compte visé et celui de l'administrateur (un CHECK ne peut pas lire la table `compte`). Cohérent avec Z7 : il faut un autre membre de l'équipe.
  - **Y4** : la règle « tout administrateur a un TOTP actif » est garantie **par le code**, pas par la base : l'outil crée le compte et le TOTP dans la même transaction, et l'extracteur administrateur (étape 3.6) refuse un compte sans TOTP actif. Un trigger en base bloquerait l'administrateur dont le TOTP vient d'être désactivé, qui doit pouvoir exister sans TOTP le temps de se réenrôler (voir la note ci-dessus).

**Profil administrateur et numéro de CNIB (décisions K1 à K3, 2026-10-06, migration 0013)**
- Chaque administrateur LaafiCare a un **numéro de CNIB**, enregistré **une seule fois** sur son profil : demandé à la création du premier administrateur par l'outil local, et à l'acceptation d'une invitation d'administrateur. (À ne pas confondre avec la personne qui demande une désactivation de TOTP : pour elle, seule l'attestation `cnib_verifiee` est enregistrée, jamais le numéro — décision Y2.)
- **K1** : table `profil_administrateur` (une ligne par compte, clé étrangère composite qui n'accepte qu'un compte `administrateur_laaficare`), et non une colonne sur `compte` : le numéro n'apparaît dans aucune requête de connexion.
- **K2** : dans la migration 0013, avec le TOTP.
- **K3** : **obligatoire** pour tout compte administrateur, garanti **par le code** (compte et profil créés dans la même transaction) ; **unique**, garanti **par la base**, sur le numéro normalisé.
- **Format (décision du porteur, 2026-10-06)** : la lettre **B suivie de 8 chiffres** (`B12345678`), contrôlé sur le numéro **normalisé** (majuscules ASCII avec `COLLATE "C"`, sans les 25 espaces Unicode, aucune autre transformation) : `b 1234 5678` est accepté et devient `B12345678` ; `B1234567` ou `A12345678` sont refusés. Garanti par la base (CHECK) ; le code contrôle avant l'écriture pour renvoyer un message clair. Le numéro reste aussi conservé tel que saisi. **Ce format vient du porteur, pas d'une source officielle** (aucune trouvée le 2026-10-06 : sites de l'ONI et de la Police nationale ; texte du décret n° 2003-668 inaccessible en ligne), et il ne décrit que la **CNIB actuelle**. Quand la carte d'identité biométrique de l'AES arrivera, la règle devra **accepter les deux formats** : la CNIB actuelle restera valable pour les administrateurs déjà enregistrés.
- **Visible seulement par l'équipe LaafiCare** : lu uniquement par les routes réservées aux administrateurs, jamais mis dans un jeton, jamais écrit dans les journaux. La base ne peut pas l'imposer (un seul rôle PostgreSQL pour l'application).

**Affectations**
- Un **compte professionnel unique** par personne, avec **plusieurs affectations** : une affectation relie le compte à une structure avec un rôle. Une personne peut avoir des rôles différents selon la structure, et plusieurs rôles dans la même structure (ex. responsable et médecin de sa clinique).
- Rôles : liste fermée (responsable, directeur, chef de service, major, médecin, infirmier dont sage-femme, agent d'accueil, pharmacien, biologiste, agent MUNASEB). Les rôles permis dépendent du type de structure (liste fermée dans le code) ; dans cette itération, seule la ligne `munaseb` → `responsable`, `agent_assurance_munaseb` est utilisable.
- Connexion professionnelle : email + mot de passe, puis **choix de l'affectation** ; le jeton porte l'affectation active. Changer de structure donne un nouveau jeton, sans mot de passe.
- Deux familles d'extracteurs : *gérer sa demande* (responsable d'une structure non validée : brouillon, pièces, soumission, suivi) et *exercer* (à chaque requête : compte actif et bonne `version_jeton`, affectation active, structure validée, licence en cours).
- **Désactivation** : elle porte sur l'affectation, jamais sur le compte, et jamais de suppression. Celui qui peut inviter à un rôle peut le désactiver. Le dernier responsable actif d'une structure ne peut pas être désactivé.

**Structures et licence**
- **N'importe quel patient peut créer sa structure** et en devient responsable. S'il n'a pas encore de compte professionnel, il le crée à ce moment (email + mot de passe professionnel) ; s'il en a un, son mot de passe professionnel est demandé.
- Informations : nom, type, téléphone ; adresse cadastrale (commune et province obligatoires ; parcelle, lot, section facultatifs, pour les zones non loties ; liste des provinces non fermée faute de source officielle) ; latitude et longitude (bornes −90/90 et −180/180 ; servira aussi à BF-06) ; références des autorisations.
- Cycle : `brouillon` → `en_attente` (soumission, pièces exigées vérifiées) → `validee` (**licence attribuée**) ou `refusee` (motif) ; `validee` → `suspendue` → `validee` (**réactivation** par l'équipe LaafiCare, décision M8 du 2026-10-04 ; la date d'expiration de la licence peut être redéfinie à ce moment) ; `brouillon` ou `refusee` → `abandonnee` (décision M5 du 2026-10-04 : le **créateur** renonce à sa demande ; elle ne compte plus dans la limite anti-abus, **rien n'est effacé**, pièces comprises ; état final). Une demande refusée peut être corrigée et soumise à nouveau (retour à `brouillon`), anciennes pièces et décisions conservées. Chaque décision de l'administrateur (validation, refus, suspension, réactivation) est tracée dans `decision_structure` (administrateur, date, motif, expiration de licence), jamais modifiée ni supprimée. Les soumissions, retours en brouillon et abandons ne sont pas historisés en V1 (M9).
- **Avant validation, aucun accès aux données patient, quelle que soit l'action.**
- **Limite anti-abus** : une seule structure **non validée** par personne (`brouillon`, `en_attente` ou `refusee` ; une structure `abandonnee` ne compte plus), garantie par la base ; plafond d'invitations par structure et par jour (constante).
- **La MUNASEB** : son chef crée la structure (type `munaseb`), l'équipe LaafiCare la valide, puis il invite ses agents. **Une seule MUNASEB `validee` ou `suspendue`** (décision M6 du 2026-10-04), garantie par la base : plusieurs demandes MUNASEB peuvent exister en brouillon ou en attente — sinon n'importe qui bloquerait le vrai chef de la mutuelle avec un brouillon, comme pour les numéros d'autorisation — et l'administrateur voit un avertissement quand plusieurs sont en cours. L'outil `creer_agent_assurance_munaseb` et la table `agent_assurance_munaseb` disparaissent ; les comptes agents de développement existants ne sont pas repris (à recréer par invitation).

**Pièces justificatives**
- Liste fermée **par type de structure**, dans le code. Clinique : `autorisation_creation` et `autorisation_ouverture_exploitation` (Ministère de la Santé ; seule la seconde autorise à exercer), chacune avec numéro, date et fichier. MUNASEB : liste inconnue, soumission permise sans pièce exigée, validation sur les seules informations (U6 b). Hôpital, pharmacie, laboratoire : soumission impossible tant que leur liste n'est pas définie (U6 a).
- **Numéro d'autorisation** : saisi par le responsable et **conservé tel quel** (l'équipe le compare au document) ; une version **normalisée** sert aux comparaisons (colonne générée `numero_normalise`, migration 0012 ; règle complétée le 2026-10-05) : majuscules sur les **seules lettres ASCII** (`upper` avec `COLLATE "C"`) ; **espaces retirées** (les 25 caractères Unicode *White_Space*, liste explicite et non `\s`, dont le sens dépend de la locale du serveur — décision N1) ; **tirets gardés** (ils font partie du numéro officiel, ex. `2018-628/MS/CAB`) mais **toutes les sortes de tirets converties en tiret simple** (propriété Unicode *Dash*, même liste que `telephone.rs`) ; **préfixe retiré au début** : « N » suivi de « ° », « º », « o » ou « . », avec ou sans espaces (sans risque : un numéro d'arrêté commence par l'année). Résultat identique sur tous les serveurs. Un numéro normalisé vide (ex. « N° » seul) est refusé par la base (décision N2). **Unique parmi les structures validées seulement, par type de pièce** — sinon un brouillon pourrait bloquer l'inscription d'une vraie structure. La base garantit qu'une seule structure peut être validée avec un même numéro (registre des autorisations validées, rempli dans la transaction de validation). Quand une demande en attente porte un numéro déjà présent dans une autre demande, l'administrateur voit un avertissement avec la demande concernée.
- **Téléversement** : stockage dans PostgreSQL (`bytea`), jamais d'adresse publique ; formats PDF, JPEG, PNG uniquement (ni SVG, ni HEIC) ; 5 Mo par fichier ; **type vérifié par les premiers octets** du fichier, jamais par l'extension, le `Content-Type` ou le nom envoyés ; nom remplacé par un identifiant ; empreinte SHA-256 enregistrée ; une pièce ne se modifie jamais (remplacée pendant le brouillon) ; conservées après validation. **Seuls les administrateurs LaafiCare les consultent**, par une route qui renvoie `Content-Disposition: attachment` et `X-Content-Type-Options: nosniff`. **Le contenu d'un fichier n'est lu que dans cette route de téléchargement** : les listes et détails de pièces ne sélectionnent jamais la colonne du fichier. Une pièce est envoyée à part (réseau faible), dans une structure en brouillon.

**Invitations**
- Seulement depuis une **structure validée**. Qui invite qui : le responsable propose tout rôle permis pour son type de structure (y compris un autre responsable) ; le chef de service, un médecin ; le major, un infirmier ; un administrateur, un autre administrateur.
- L'invitation est enregistrée **au numéro, sans rien chercher** : réponse toujours « invitation envoyée », SMS identique dans tous les cas (section 11). Si le numéro n'a pas de compte patient, le SMS invite à en créer un d'abord.
- Validité **7 jours** (constante) ; annulable par celui qui a invité ou par le responsable tant qu'elle est en attente. Statuts : en attente, acceptée, refusée, annulée, expirée.
- La personne répond depuis son compte patient. **La première acceptation crée le compte professionnel** (email et mot de passe choisis par la personne, jamais connus d'un tiers) ; **les suivantes ajoutent seulement une affectation**, avec le mot de passe professionnel demandé.

**Bascule (app web professionnelle)**
- Du professionnel vers le patient : **sans mot de passe**, même si le compte patient est verrouillé (le verrouillage reste en place pour la connexion par téléphone). La bascule incrémente la `version_jeton` du compte professionnel : l'ancien jeton professionnel est aussitôt refusé.
- Du patient vers le professionnel : **toujours avec le mot de passe professionnel**.

**Administration LaafiCare**
- Le **premier administrateur** naît d'un outil local (il doit déjà avoir un compte patient ; l'outil refuse si un administrateur existe déjà). Les suivants sont invités par un administrateur.
- L'équipe consulte les informations et les pièces, puis valide (licence) ou refuse (motif), et peut suspendre.

**À reporter dans le CDC** : un **espace patient sur le web** entre en V1 (le CDC l'excluait), pour la bascule depuis l'app professionnelle ; création libre des structures sous licence ; invitations ; comptes séparés ; sections 6 et 11 révisées.

**Points ouverts**
- Durée d'une licence : illimitée ou à renouveler (le modèle accepte les deux : expiration facultative).
- Pièces exigées pour la MUNASEB (le porteur se renseigne), et pour les hôpitaux, pharmacies et laboratoires.
- Liste officielle des provinces du Burkina Faso (pour fermer la liste) ; bornes géographiques du territoire (pour un contrôle GPS plus strict).
- Choix de la dépendance d'envoi d'email (récupération professionnelle, vérification de l'email) ; l'email professionnel n'est pas vérifié en attendant.
- Liste de mots de passe interdits (NIST rév. 4, « SHALL ») : source à choisir (liste intégrée, service externe), **avant la production**.
- **Plafond sur `POST /api/patients/otp`**, par numéro et par période, **avant la production** : aujourd'hui, n'importe qui peut faire envoyer un SMS à n'importe quel numéro, sans limite (coût SMS, harcèlement).
- `version_jeton` est augmentée à chaque réinitialisation du mot de passe patient (décision C2), mais elle ne sera vérifiée qu'une fois portée par le jeton (étape 3.6).- Production : réécriture des images, antivirus, désactivation du contenu actif des PDF (OWASP) ; chiffrement du disque du serveur de base de données ; rôle PostgreSQL sans droits d'administration (section 12) ; fournisseur SMS réel (OTP, invitations).
- `partenaire_sante_munaseb` recoupe `structure` : un partenaire deviendra un lien entre la MUNASEB et une structure, avec les modules Pharmacie et Laboratoire.
- `MajorLaboratoire` : rôle séparé ou non (section 6).
- **Changement du numéro de CNIB d'un administrateur** (renouvellement de carte, passage à la carte AES) : par l'équipe LaafiCare, avec trace ; à concevoir.
- **Requêtes SQLx vérifiées à la compilation** (macros `query!`, `query_as!`, mode hors ligne `.sqlx` pour compiler sans base) : étape à part, **après la stabilisation du module MUNASEB**. Aujourd'hui, une faute dans une requête n'apparaît qu'à l'exécution (tests en base compris).
- **Sauvegarde de la clé `TOTP_CLE_CHIFFREMENT` en production** : à conserver à part, de façon sûre (jamais avec la base ni ses sauvegardes) ; sa perte rendrait tous les TOTP inutilisables, et chaque compte devrait être réenrôlé.
- **Désactivation du TOTP par les structures validées** (vérification de la CNIB à l'accueil d'une structure plutôt que par l'équipe LaafiCare) : à rediscuter plus tard ; en V1, seule l'équipe LaafiCare désactive (Z9).
