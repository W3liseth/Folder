# CLAUDE.md

Guide de travail pour Claude Code dans ce dépôt.

## Le projet

**folder** (anciennement « fx ») est un gestionnaire de fichiers pour Windows écrit en Rust,
inspiré fonctionnellement de l'application *Files* (Files Community). Les crates internes gardent
le préfixe `fx-`. Priorités, dans cet ordre :

1. **Rapide** : rien ne bloque le thread UI ; navigation instantanée, même sur 100 000 fichiers.
2. **Fluide** : ≤ 8 ms par image, aucun saccadement au défilement ou au redimensionnement.
3. **Sobre** : ≤ 30 Mo de mémoire privée au repos, ≤ 60 Mo pour un dossier de 100 000 fichiers.

L'architecture complète, les budgets chiffrés et le plan par phases sont dans
[ARCHITECTURE.md](ARCHITECTURE.md). **Lis-le avant toute modification non triviale.**
Si un changement contredit ce document, mets à jour le document dans le même commit
(ou demande avant si c'est une décision structurante).

## Interdits absolus

- Pas d'Electron, Tauri, WebView/WebView2, moteur JavaScript ni HTML pour l'UI ou les aperçus.
- Pas de `tokio`, `async-std` ni `rayon` (threads et mémoire supplémentaires). Concurrence =
  `std::thread` + `crossbeam-channel` + les pools définis dans `fx-app`.
- Pas d'I/O, d'appel COM ou d'appel système bloquant sur le thread UI. Exceptions
  documentées (ARCHITECTURE.md §11.2), qui l'exigent : options du menu contextuel shell
  (`IContextMenu` : construction, lecture, exécution ; les extensions comptent sur la boucle
  de messages de la fenêtre), glisser vers l'extérieur (`SHDoDragDrop`) et attributs DWM du
  cadre.
- Pas d'objet alloué par fichier listé : utiliser `DirSnapshot` (structure de tableaux).
- Pas de `unsafe` hors de `crates/fx-win` (seule exception : le code tiers vendu dans
  `vendor/`, qu'on ne modifie que pour le porter ; cf. `vendor/*/VENDOR.md`).
- Pas de télémétrie réseau, pas de requête réseau implicite. Seule exception : la
  vérification de mise à jour (`fx-app::upgrade`, ARCHITECTURE.md §7.12) — réglage
  `auto_update` désactivable, une requête par heure au plus vers les releases GitHub (et à
  l'ouverture de Paramètres › Mises à jour si le résultat a plus de 5 min), rien
  d'autre envoyé, faite dans le processus éphémère `folder.exe --update-check` (WinHTTP via
  `fx_win::http`, jamais chargé dans une fenêtre) ; installation seulement à la demande.
- Pas de copie de code ni d'assets de l'application Files : elle n'a servi qu'à établir
  l'inventaire fonctionnel. On réimplémente.

## Commandes

```bash
cargo build --release
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

```bash
cargo fmt --all -- --check
```

```bash
cargo bench -p fx-core
```

Lancer folder (rendu CPU par défaut ; `--renderer glow` pour OpenGL) :

```bash
cargo run --release -p folder
```

Mesures face aux budgets (mémoire, threads, première image, navigation) :

```bash
cargo run --release -p memprobe -- --renderer cpu --tree bench-data/100k
```

Fluidité en fenêtre (temps d'image en défilement continu) :

```bash
cargo run --release -p memprobe -- --scroll --renderer cpu --size 1920x1080 --tree bench-data/100k
```

Coût de rendu hors écran, 1080p à 4K :

```bash
cargo run --release -p renderbench -- --tree bench-data/100k
```

Vue « Analyser l'espace » sur un vrai dossier (coût de rendu, capture PNG facultative) :

```bash
cargo run --release -p renderbench -- --disk C:\Users --snapshot capture.png
```

Rendu partiel comparé image par image au rendu complet (défilement, survol, menus) :

```bash
cargo run --release -p renderbench -- --tree bench-data/100k --verify
```

Installeur Windows (Inno Setup 6 requis). En CI, `.github/workflows/installer.yml` publie
la release `v<version>` dès que la version de `[workspace.package]` (Cargo.toml) change sur `main`,
avec pour description sa section de `CHANGELOG.md` : tout changement de version ajoute
`## <version> — <date>` en tête de ce fichier (notes affichées dans Paramètres › Mises à jour,
vérifié par `cargo test`) :

```bash
iscc /DAppVersion=0.9.0 installer/folder.iss
```

Générer les jeux de test (une seule fois) :

```bash
cargo run --release -p gen-tree -- bench-data
```

Avant de dire qu'une tâche est terminée : `fmt`, `clippy -D warnings` et `test` passent.
Si la tâche touche `fx-fs`, `fx-app` ou `fx-ui`, lance aussi `memprobe` et rapporte les
chiffres (mémoire privée, threads, temps jusqu'à la première image) face aux budgets ;
si elle touche le rendu ou la vue, ajoute `memprobe --scroll` (1080p et 1440p).
Les deux moteurs coexistent : **rendu CPU par défaut**, `--renderer glow` en option
(ARCHITECTURE.md §3.1) ; mesure celui qui est concerné.

## Carte du workspace

| Crate | Rôle | Règle clé |
|---|---|---|
| `fx-core` | types de domaine (`EntryMeta`, `DirSnapshot`, `DiskTree`, tri, sélection, formats) | aucune dépendance `windows`/`egui` |
| `fx-win` | wrappers sûrs Win32/COM (énumération, `IFileOperation`, `IContextMenu`, presse-papiers, images, watch, DWM, HTTPS WinHTTP et signatures pour la mise à jour) | seul endroit avec `unsafe`, chaque bloc a `// SAFETY:` |
| `fx-fs` | énumération en flux, service de surveillance (IOCP), recherche récursive, zip, analyse de l'espace (`diskscan`) et suggestions de nettoyage (`cleanup`) | jamais de dépendance à l'UI |
| `fx-shell` | worker STA : icônes, miniatures, ouvrir / ouvrir avec / propriétés | COM en STA uniquement |
| `fx-commands` | métadonnées des commandes (macro `commands!`), raccourcis | toute action utilisateur a un `CommandId` |
| `fx-store` | réglages TOML, session, SQLite (étiquettes, préférences par dossier) | écritures hors thread UI |
| `fx-preview` | aperçus texte/code, markdown, infos | chargé seulement si le volet est ouvert |
| `fx-app` | `AppState`, `Msg`, `update`, `Effect`, scheduler, historique, opérations, mise à jour et notes de version (`upgrade`), branches Git (`git`, programme `git` de l'utilisateur) | `update` pur, < 1 ms |
| `fx-ui` | rendu egui, thème, caches de textures ; fenêtres Propriétés (`props_window`, processus `--properties`) et Paramètres (`settings_window`, processus `--settings`, modifications envoyées par `WM_COPYDATA`) | lit l'état, émet des `Msg` ; n'appelle `fx-win` que pour les exceptions ci-dessus (la fenêtre Propriétés, mini-application à part, lit ses données sur ses propres threads) |
| `app/folder` | binaire, manifeste, ressources | |
| `vendor/egui_software_backend` | rasteriseur CPU tiers (porté vers egui 0.37) | hors lints du workspace, modifications minimales |
| `tools/*` | `gen-tree`, `memprobe`, `renderbench` | outils de mesure, jamais livrés |

Sens des dépendances : `fx-ui → fx-app → {fx-commands, fx-store, fx-preview, fx-fs, fx-shell} → fx-win`, et tout le monde peut dépendre de `fx-core`. Pas encore de `fx-providers` (ARCHITECTURE.md §11.2).

## Règles de performance (à appliquer à chaque PR)

- **Virtualisation** : on ne construit que les lignes/tuiles visibles (+1 écran de marge).
- **Pas d'allocation par image** dans le chemin de rendu de la liste : réutiliser les buffers,
  mettre en cache les `Galley` des noms visibles.
- **Repaint réactif** : jamais de `request_repaint()` en boucle ; les animations s'arrêtent
  d'elles-mêmes (≤ 150 ms).
- **Rendu CPU** : rendu partiel (ARCHITECTURE.md §3.4) — seules les zones modifiées sont
  rasterisées, le contenu qui défile est recopié ; une image qui change toute la fenêtre
  coûte encore une rasterisation complète (~3 ms en 1080p, ~4 ms en 1440p, ~10 ms en 4K).
  Ne pas réactiver le cache du rasteriseur (mesuré plus lent et 3× plus gourmand en
  défilement) sans nouvelle mesure `renderbench`. Fond opaque : le tampon n'est jamais
  effacé, toute zone de la fenêtre doit être peinte par un panneau opaque, sinon il reste des
  traces. Fond Acrylic (`Palette::with_acrylic`, ARCHITECTURE.md §7.7) : panneaux
  transparents ou translucides, chaque zone redessinée est effacée avant d'être rasterisée.
  Toute modification du moteur vendu passe `renderbench --verify` et
  `renderbench --verify --acrylic` (rendu partiel = rendu complet).
- **Annulation** : tout travail lié à une navigation porte `(pane_id, generation)` + `CancelToken`
  et vérifie l'annulation entre chaque lot.
- **Streaming** : l'énumération envoie des lots (512 éléments ou 8 ms) ; l'UI affiche dès le premier.
- **Paresse** : propriétés étendues, tailles de dossiers, miniatures et aperçus seulement pour ce
  qui est visible et demandé.
- **Caches bornés en octets** (LRU) : icônes, miniatures, galleys, propriétés. Toute nouvelle
  structure qui grandit avec l'usage doit avoir une borne et apparaître dans l'overlay F12.
- Nouvelle dépendance : justifier son coût (taille du binaire, threads créés, mémoire au repos)
  dans la description de la PR. Préférer `default-features = false`.

## Règles Windows / COM

- Chaque thread qui touche au shell appelle `fx_win::com::init_sta()` (RAII) ; le thread UI est STA.
- Chemins passés à Win32 : toujours via `fx_win::path::to_wide_extended()` (préfixe `\\?\`,
  gestion > 260 caractères).
- Noms de fichiers stockés en WTF-8 (`OsStr::as_encoded_bytes`) ; conversion en `String`
  uniquement pour l'affichage (`to_string_lossy`).
- Énumération : `GetFileInformationByHandleEx(FileFullDirectoryInfo)` avec tampon 64 Ko
  réutilisé ; repli `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)`.
- Opérations fichiers : `IFileOperation` (corbeille, UAC, compatibilité Explorateur), jamais
  de `std::fs::copy` pour une opération utilisateur.
- Tri naturel : doit donner le même ordre que `StrCmpLogicalW` (test de propriété dans `fx-core`).
- `HRESULT` → `fx_win::Error` ; pas de `unwrap()` sur un résultat Win32/COM.

## Ajouter une commande (exemple de tâche courante)

1. Ajouter une ligne dans la macro `commands!` de `crates/fx-commands/src/lib.rs` :
   identifiant, catégorie, libellé, glyphe Segoe Fluent Icons, raccourcis par défaut.
2. L'exécuter dans `crates/fx-app/src/update/commands.rs` (`run`) : modifier l'état et
   retourner des `Effect` (pas d'I/O directe) ; si elle dépend du contexte (sélection,
   emplacement), compléter `Fx::enabled` dans `crates/fx-ui/src/view/mod.rs`.
3. Test unitaire dans `crates/fx-app/src/update/tests.rs` : état et effets produits.
4. Elle apparaît alors automatiquement dans la palette et l'éditeur de raccourcis ; l'ajouter
   à la main dans `fx-ui` (barre d'outils, menus) si besoin.

## Style de code

- `rustfmt` par défaut ; `clippy::pedantic` toléré au cas par cas, pas imposé.
- Erreurs : `thiserror` dans les bibliothèques, `anyhow` seulement dans `app/folder` et les outils.
- Journaux : `tracing` (`debug!` dans les chemins chauds, désactivé en release par filtre).
- Commentaires : expliquer le *pourquoi* (contrainte Win32, budget, cas limite), pas le *quoi*.
- Textes visibles : en français pour l'instant (libellés de commandes dans `fx-commands`) ;
  la traduction viendra en phase 3, éviter de dupliquer un libellé qui existe déjà.
- Tests : unitaires à côté du code ; tests d'intégration fichiers dans un dossier temporaire
  (`tempfile`), jamais dans le profil utilisateur.

## Référence : application d'origine

Files 4.3.0 a été inspectée avec REA (voir ARCHITECTURE.md §1). Retenir :
.NET 10 NativeAOT + WinUI 3 + WebView2, ~88 Mo de working set / ~196 Mo privés et 84 threads
au repos ; ~180 actions via un `CommandManager` ; énumération Win32 `FindFirstFileExW` en
`LARGE_FETCH` ; `IFileOperation` et `IContextMenu` pour l'intégration shell. Ce sont les
chiffres à battre et les comportements à égaler, pas du code à porter.
