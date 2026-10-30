# ARCHITECTURE — Explorateur de fichiers natif en Rust

> Objectif : un gestionnaire de fichiers pour Windows aux fonctionnalités comparables à
> **Files** (Files Community, v4.3.0), mais **natif Rust**, **rapide**, **fluide** et
> **sobre en mémoire**. Aucun Electron, JavaScript, WebView ou Tauri.

Ce document décrit (1) ce que l'inspection de l'application d'origine a révélé, (2) les
objectifs chiffrés, (3) l'architecture cible, (4) le plan de livraison par phases.

---

## 1. Ce que l'inspection de Files a montré

Cible inspectée : `C:\Program Files\WindowsApps\Files_4.3.0.0_x64__1y0xx7n9077q4\`
(paquet MSIX, éditeur « Yair A », licence MIT annoncée dans le manifeste).

Outils : REA (`inspect_managed_artifact`, `inspect_pe_resources`, Ghidra 12.1.4 via
`open_binary` / `search_procedures` / `procedure_callers` / `analyze_function`), plus
lecture du manifeste, de `Files.deps.json` et des chaînes embarquées du binaire.

### 1.1 Observations (faits vérifiés)

| Sujet | Observation | Source |
|---|---|---|
| Runtime | `Files.exe` (51,7 Mo) est un PE x86-64 **sans répertoire CLI** : .NET 10 compilé en **NativeAOT** (`tfm: net10.0`, `IsDynamicCodeSupported: false`). | REA `ev_723b133e…`, `Files.runtimeconfig.json` |
| Taille | 20,7 Mo de `.text`, 23,9 Mo de `.rdata`, 156 590 fonctions détectées par Ghidra. Paquet total : 539 fichiers, ~75 Mo. | REA `ev_db2fe639…` |
| UI | WinUI 3 / Windows App SDK 2.5, CommunityToolkit, Win2D, **WebView2** (aperçus HTML/Markdown), MVVM (`CommunityToolkit.Mvvm`). | `AppxManifest.xml`, `Files.deps.json` |
| Dépendances natives | `7z64.dll` (archives), `git2-*.dll` (libgit2), `e_sqlite3.dll` (SQLite), `WebView2Loader.dll`, `Microsoft.Graphics.Canvas.dll`. | listing du paquet |
| Autres bibliothèques | DiscUtils (ISO/UDF), FluentFTP, TagLibSharp (métadonnées média), Markdig, ColorCode, SharpZipLib, SevenZipSharp, LibGit2Sharp, UTF.Unknown (détection d'encodage), Sentry (télémétrie). | `Files.deps.json` |
| Énumération de dossiers | Appel observé : `FindFirstFileExW(path, FindExInfoBasic, …, FindExSearchNameMatch, NULL, 2 /* FIND_FIRST_EX_LARGE_FETCH */)` puis `FindNextFileW`, avec test de l'attribut `0x400` (point d'analyse). Les chaînes mentionnent aussi `NtQueryDirectoryFile` et `FindFirstFileExFromApp`. | REA `ev_20cbafac…`, `ev_68651045…` |
| Surveillance | `ReadDirectoryChangesW` importée (3 appelants) ; logs « FILE_ACTION_MODIFIED thumbnail retry… scheduling 2s timer retry ». | REA `ev_30e362ae…`, chaînes |
| Opérations fichiers | Interfaces COM `IFileOperation` + `IFileOperationProgressSink` (Copy/Move/Delete/Rename/NewItem, Pre/Post callbacks). | chaînes CsWin32 |
| Menu contextuel | `IContextMenu` / `IContextMenu2` (`QueryContextMenu`, `InvokeCommand`, `HandleMenuMsg`) = menu shell Windows natif. | chaînes CsWin32 |
| Miniatures | `GetThumbnailAsync` WinRT + cache de miniatures paramétrable (`ThumbnailCacheSizeLimit`, `EnableThumbnailCache`), chargement différé (`NeedsDelayedThumbnailLoad`). | chaînes |
| Persistance | `user_settings.json`, `user_settings.db`, `filetags.db` / `filetags.json`, `session.json`, base de préférences de disposition par dossier (`LayoutPreferencesDatabaseManager`). | chaînes |
| Mémoire au repos | Instance en cours d'exécution : **~88 Mo working set, ~196 Mo privés, 84 threads**. | `Get-Process` (observation passive, hors REA) |

### 1.2 Inventaire fonctionnel (déduit des noms de types embarqués)

Les noms de types NativeAOT décrivent la structure de l'application :

- **Fenêtre / navigation** : `MainPage`, onglets (`TabBar`, `TabBarItem`, réouverture d'onglet fermé),
  **double volet** (`ShellPanesPage`, disposition horizontale/verticale, focus de l'autre volet),
  historique Précédent/Suivant/Parent, mode compact overlay, plein écran.
- **Dispositions** (`Views.Layouts.*`) : Détails, Liste, Grille, Cartes, **Colonnes (Miller)**,
  groupement (par type, date, taille, dossier, statut de synchro…), tri multi-critères,
  dossiers en premier, en-tête de filtre, ajustement auto des colonnes.
- **Omnibar** (`Files.App.Controls.Omnibar`) : barre d'adresse multi-modes (chemin / palette de
  commandes / recherche) avec suggestions ; fil d'Ariane (`BreadcrumbBar`).
- **Barre latérale** (`SidebarView`) : accès rapide épinglé, bibliothèques, lecteurs, lecteurs
  cloud (OneDrive, Dropbox, Google Drive, Box, Synology… via `*CloudDetector`), réseau,
  distributions **WSL**, **étiquettes** (tags).
- **Accueil** (`HomePage`) : widgets Accès rapide, Lecteurs, Emplacements réseau, Étiquettes,
  Fichiers récents.
- **Volets** : volet d'info / aperçu (texte, code, Markdown, image, média, PDF, HTML, RTF,
  gestionnaire d'aperçu shell), **Shelf** (étagère de fichiers temporaire), barre d'état,
  **centre d'état** (opérations en cours + graphe de débit `SpeedGraph`).
- **Commandes** : ~180 actions (`*Action`) pilotées par `CommandManager` + `CommandCodes` +
  `HotKey`, éditeur de raccourcis (`KeyBindingEditorDialog`), barre d'outils personnalisable.
- **Opérations** : copier/couper/coller, supprimer (corbeille ou définitif), renommer,
  **renommage en masse**, nouveau fichier/dossier/raccourci, aplatir un dossier,
  annuler/rétablir (`StorageHistoryHelpers`), gestion des conflits de noms, élévation.
- **Archives** : créer (zip/7z, niveau, dictionnaire, chiffrement, découpage) et extraire.
- **Git** : init, clone, fetch, pull, push, sync, branches (libgit2).
- **Divers** : FTP, montage ISO, fenêtre Propriétés (Général, Détails, Hachages, Sécurité/ACL,
  Compatibilité, Raccourci, Bibliothèque, Signatures, Personnalisation), installation de
  polices/certificats/pilotes INF, définir comme fond d'écran, ouvrir dans le terminal / l'IDE,
  formater un lecteur, épingler au menu Démarrer, intégration PowerToys Peek / QuickLook / Seer.
- **Paramètres** : Général, Apparence, Disposition, Dossiers, Étiquettes, Actions,
  Avancé, DevTools, À propos ; restauration de session.

### 1.3 Inconnues

- Le PDB (`Files.pdb`) n'est pas fourni : les fonctions décompilées sont anonymes ; seul
  un échantillon des appels Win32 a été suivi.
- `FindFirstFileExFromApp` / `NtQueryDirectoryFile` apparaissent en chaînes mais pas dans
  la table d'imports : liaison dynamique probable, non vérifiée.
- Le comportement exécutable (latences, ordre réel des opérations) n'a **pas** été observé
  dynamiquement ; seule l'empreinte mémoire au repos l'a été.

### 1.4 Leçons retenues pour la réécriture

1. Le coût mémoire de Files vient surtout de la pile (runtime .NET + WinUI 3 + XAML +
   WebView2 + Win2D) et de son modèle objet riche (`ListedItem` ≈ 70 propriétés observables
   par fichier). → **modèle de données compact, pas d'objet par fichier**.
2. Files utilise déjà les bonnes API Win32 pour l'énumération (`FindExInfoBasic` +
   `LARGE_FETCH`) : on les reprend, sans couche WinRT `StorageFile`.
3. Le shell Windows (menus contextuels, `IFileOperation`, icônes, miniatures) est
   indispensable pour une intégration native : on l'utilise via le crate `windows`.
4. L'architecture « commande = action enregistrée » (`IAction` + `CommandCodes`) est
   excellente : elle alimente barre d'outils, menus, raccourcis et palette. On la conserve.
5. 84 threads au repos est excessif → **pool de threads borné**.

---

## 2. Objectifs non fonctionnels (budgets)

Ces budgets sont des **exigences**. Toute régression est un bug.

| Mesure (build release, Windows 11, SSD) | Budget |
|---|---|
| Démarrage à froid → première image | ≤ 150 ms |
| Mémoire privée au repos (1 fenêtre, 1 onglet, dossier ~100 éléments) | ≤ 30 Mo |
| Mémoire privée, dossier de 100 000 fichiers (détails, sans miniatures) | ≤ 60 Mo |
| Coût mémoire par entrée listée (hors nom) | ≤ 48 octets |
| Premier lot affiché après navigation (dossier local) | ≤ 16 ms |
| Énumération complète de 100 000 fichiers (cache chaud) | ≤ 300 ms |
| Tri de 100 000 entrées (tri naturel) | ≤ 50 ms |
| Frame time pendant défilement / redimensionnement | ≤ 8 ms (120 Hz) |
| CPU au repos | 0 % (aucun repaint sans événement) |
| Threads au repos | ≤ 12 |
| Taille de l'exécutable (sans DLL optionnelles) | ≤ 15 Mo |

---

## 3. Choix technologiques

### 3.1 Décision : Windows uniquement, Rust stable, `windows` + `egui`

| Couche | Choix | Pourquoi |
|---|---|---|
| Langage | Rust stable, édition 2024 | Exigence. |
| Plateforme | Windows 10 19041+ / Windows 11 (comme Files) | L'intégration shell est la valeur du produit. |
| API système | crate `windows` (windows-rs, Microsoft) | Accès direct Win32/COM, sans couche runtime. |
| UI | `egui` 0.37 | Mode immédiat, repaint uniquement sur événement, listes virtualisées natives (`show_rows`), MIT/Apache. |
| Rendu (défaut) | **rasterisation CPU sans cache, partielle** : rasteriseur `egui_software_backend` vendu (`vendor/`), fenêtre `winit` + `softbuffer` ; seules les zones modifiées sont rasterisées et présentées, le contenu qui défile est recopié | Aucun pilote GPU chargé : 15–24 Mo au repos, 12 threads, première image ~60 ms, défilement p95 ≈ 4 ms en 1440p (mesures §3.4, §11.3). |
| Rendu (option) | `eframe` + `glow` (OpenGL), `--renderer glow` ; `wgpu`/DX12 derrière la feature `wgpu` | Fallback si le rendu CPU pose problème sur une machine ; ~10× plus de mémoire (pilote GPU), cf. §3.3. |
| Texte | rendu `egui` + polices système (Segoe UI Variable / Segoe Fluent Icons) chargées depuis `C:\Windows\Fonts` | Aucune police embarquée → binaire plus petit. |
| Concurrence | `std::thread` + `crossbeam-channel` | **Pas de tokio** : pas besoin d'I/O réseau massive ; moins de threads et de mémoire. |
| Persistance | `serde` + `toml` (réglages), `rusqlite` feature `bundled` (étiquettes, préférences par dossier) | Simple, robuste, compact. |
| Archives | `zip` (compression/extraction, implémenté) ; `sevenz-rust2` (7z) et RAR via `unrar` optionnel plus tard | Pas de `7z64.dll` obligatoire. |
| Git | branche active lue dans `.git/HEAD` (aucune dépendance) ; liste des branches et changement de branche par le programme `git` de l'utilisateur (`fx-app::git`, sans console, jamais de commande réseau) ; `gix` (pur Rust) envisagé pour statut/opérations en phase 3 | Pas de dépendance C ni de bibliothèque Git embarquée : `git switch` gère seul références compactées, worktrees et arbre de travail. |
| Markdown | `pulldown-cmark` rendu dans egui (pas de WebView) | Exigence « pas de JS ». |
| Allocateur | allocateur système + **Segment Heap** activé via le manifeste | Réduit l'empreinte sur Windows 10 2004+. |

Décision du rendu (2026-10-18), sur mesures (§3.3, §3.4) : le coût mémoire du GPU vient
du pilote chargé dans le processus (≥ 117 Mo et 36 threads en OpenGL sur NVIDIA, 328 Mo en
DX12), pas d'egui. Le rendu CPU sans cache tient tous les budgets jusqu'en 1440p ; la 4K
(~10 ms p95) était le seul écart. Depuis le rendu partiel et le défilement par recopie
(phase 1, §3.4), la 4K passe aussi sous 8 ms hors écran (à confirmer sur un écran 4K).

Alternatives évaluées :

- **egui sur GPU (eframe glow/wgpu)** : le plus rapide à rasteriser, mais incompatible avec
  les budgets mémoire et threads ; conservé en option.
- **Slint** : très faible mémoire et bon rendu Fluent, mais licence (GPLv3 ou licence
  royalty-free avec attribution) et modèle déclaratif moins souple pour les listes
  massives personnalisées. Reste le plan B : la couche UI est isolée (cf. §4) pour pouvoir
  basculer.
- **Win32 pur + Direct2D/DirectWrite** : empreinte minimale absolue, mais il faudrait
  réécrire tous les widgets (saisie de texte, IME, accessibilité). Réservé à des
  composants ciblés si egui ne tient pas un budget.
- **iced / gpui** : viables, mais gpui est surtout éprouvé hors Windows et iced redessine
  plus largement ; aucun gain décisif.
- **WinUI 3 via windows-rs** : on retrouverait le coût mémoire de Files.

### 3.2 Profil de compilation

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true
debug = false
panic = "unwind"      # un panic dans un worker ne doit pas tuer l'UI

[profile.release-dbg]  # pour profiler (WPA, Superluminal, cargo-flamegraph)
inherits = "release"
debug = true
strip = false
```

Manifeste applicatif (embarqué via `embed-resource` dans `build.rs`) :
`dpiAwareness = PerMonitorV2`, `longPathAware = true`, `heapType = SegmentHeap`,
`activeCodePage = UTF-8`, Common Controls v6.

### 3.3 Mesures de la phase 0 (2026-10-18)

Machine : Windows 11, NVIDIA RTX 3090 (pilote 32.0.16.1742), SSD. `memprobe`, médiane de 3
exécutions, build release. Jeux `gen-tree` de 1 002 et 100 002 entrées.

| Mesure | glow (OpenGL) | wgpu (DX12) | Budget |
|---|---|---|---|
| Mémoire privée, 1k entrées | **117 Mo** | 328 Mo | ≤ 30 Mo |
| Mémoire privée, 100k entrées | **125 Mo** | — | ≤ 60 Mo |
| Threads | 36 | 23 | ≤ 12 |
| Première image | ~220 ms | ~400 ms | ≤ 150 ms |
| Premier lot, navigation à chaud | 2,5–4 ms | — | ≤ 16 ms ✓ |
| Énumération 100k | ~62 ms | — | ≤ 300 ms ✓ |
| Instantané 100k (`heap_bytes`) | 7,7 Mo | — | — |
| Exécutable | 6,4 Mo | 10,9 Mo | ≤ 15 Mo ✓ |

Bancs `criterion` (`fx-core`) : ajout de 100k entrées en un lot ≈ 67 ms, en lots croissants
≈ 116 ms au total, re-tri de 100k ≈ 52 ms. Le tri naturel dépasse donc légèrement le budget
de 50 ms (pistes : clés de tri précalculées, chemin rapide ASCII).

Constat : le modèle de données tient ses objectifs (100k entrées ≈ +7,5 Mo), mais le
**plancher mémoire et les threads viennent du pilote GPU** (`nvoglv64.dll`,
`nvgpucomp64.dll`, `nvwgf2umx.dll` chargés dans le processus). Prototype jetable de
comparaison, egui 0.34 + `egui_software_backend` 0.0.3 (rendu CPU via softbuffer, aucun
pilote GPU chargé), même fenêtre 1100×700 avec 1 000 lignes : **14 Mo privés, 8 threads**.
### 3.4 Prototype de rendu CPU dans folder (2026-10-18)

Le rasteriseur de `egui_software_backend` 0.0.3 est vendu dans `vendor/` et porté vers
egui 0.37 (cf. `vendor/egui_software_backend/VENDOR.md`). folder embarque les deux moteurs :
`folder --renderer cpu|glow` (ou `FOLDER_RENDERER`). Le moteur CPU tourne **sans cache de
primitives** : la mesure montre qu'en défilement il est plus rapide et ~3× moins gourmand
(le cache conserve plusieurs tampons plein écran qui ne servent pas quand la liste bouge).

En fenêtre (`memprobe`, 100 002 entrées, défilement de 3 lignes par image, 600 images) :

| Mesure | CPU sans cache | CPU avec cache | glow (OpenGL) | Budget |
|---|---|---|---|---|
| Mémoire privée au repos, 1k | **9,0 Mo** | 15,5 Mo | 117 Mo | ≤ 30 Mo |
| Mémoire privée au repos, 100k | **16,7 Mo** | 23,5 Mo | 125 Mo | ≤ 60 Mo |
| Threads | **11** | 11 | 36 | ≤ 12 |
| Première image | **~56 ms** | ~55 ms | ~220 ms | ≤ 150 ms |
| Défilement 1920×1080 : p50 / p95 / max | 3,6 / **4,5** / 6,4 ms | 5,1 / 6,8 / 9,6 ms | 0,5 / 0,8 / 1,1 ms¹ | p95 ≤ 8 ms |
| Mémoire pendant le défilement 1080p | **23 Mo** | 48 Mo | 137 Mo | ≤ 60 Mo |
| Défilement 2560×1440 : p50 / p95 / max | 5,6 / **7,3** / 14,4 ms | 8,1 / 9,5 / 16,0 ms | 0,7 / 1,0 / 1,5 ms¹ | p95 ≤ 8 ms |
| Mémoire pendant le défilement 1440p | **30 Mo** | 73 Mo | 141 Mo | ≤ 60 Mo |

¹ Temps CPU seulement (`cpu_usage` d'eframe) : le travail du GPU n'y figure pas.

Hors écran (`renderbench`, sans la présentation à l'écran, ~0,7 ms en 1440p ; écrans de
la machine limités à 2560×1440, donc 4K mesurée uniquement hors écran) :

| Résolution | egui + tessellation p95 | Rasterisation CPU p95 (sans cache) | Total p95 | Δ mémoire² |
|---|---|---|---|---|
| 1920×1080 | 0,5 ms | 4,2 ms | 4,7 ms | 18 Mo |
| 2560×1440 | 0,5 ms | 4,9 ms | 5,3 ms | 25 Mo |
| 3840×2160 @150 % | 0,6 ms | 10,0 ms | 10,4 ms | 43 Mo |
| 3840×2160 @100 % | 0,7 ms | 9,0 ms | 9,7 ms | 42 Mo |

² Y compris le tampon de sortie lui-même (33 Mo en 4K).

Avec rayon (`--features rayon`), seul le mode avec cache est parallélisé : 4K ≈ 9,5–10,8 ms
p95, pour 16 threads de plus. Aucun gain sur le mode sans cache.

Bilan :
- Le rendu CPU sans cache respecte **tous** les budgets jusqu'en 1440p, avec une empreinte
  6 à 13× plus faible que le GPU et un démarrage 4× plus rapide.
- En 4K, il tient ~100 images/s (p95 ≈ 10–11 ms avec la présentation) : fluide à 60 Hz,
  mais au-dessus du budget de 8 ms (120 Hz). Pistes : ne pas effacer le tampon quand un
  panneau couvre tout, rendre seulement les zones modifiées (survol, barre d'état), décaler
  les pixels existants lors du défilement et ne dessiner que les lignes découvertes —
  appliquées depuis (ci-dessous).
- Risque : le rasteriseur vient d'une crate jeune (0.0.3) que nous maintenons désormais
  nous-mêmes dans `vendor/`.

Mesures avec l'application complète (2026-10-18, §11) : 1080p p95 ≈ 5,2 ms ; 1440p p95
7,9–8,8 ms selon l'exécution, soit **à la limite du budget** (l'interface finale dessine plus
que le prototype : barre d'outils, fil d'Ariane, colonnes, barre d'état). La piste n° 1 de
la phase 1 (ne plus effacer le tampon, `egui_software_backend::set_clear_each_frame(false)`)
est appliquée ; son gain se perd dans le bruit de mesure.

#### Rendu partiel et défilement par recopie (pistes 2 et 3, 2026-10-18)

Le moteur CPU ne rasterise plus toute la fenêtre à chaque image (`render_damaged`,
`vendor/egui_software_backend/src/damage.rs`, détails dans `VENDOR.md`) :

- **Zones modifiées** : la liste de primitives egui est comparée à celle de l'image
  précédente (empreinte de chaque primitive, appariement dans l'ordre de dessin) ; seules les
  tuiles (16×8 px) touchées par une primitive apparue, disparue ou modifiée sont redessinées,
  du fond vers le dessus, puis présentées par `softbuffer::present_with_damage` (bandes de
  64 px). Survoler une ligne ne redessine que cette ligne ; une image sans changement visible
  ne rasterise rien.
- **Défilement par recopie** : un contenu translaté d'un nombre entier de pixels dans une
  découpe restée en place (la liste) est détecté automatiquement (egui arrondit l'origine du
  contenu défilant au pixel, même pendant le défilement lissé de la molette). Les pixels
  sont recopiés là où du contenu mobile se trouvait ou arrive (pas le fond uni entre les
  colonnes) ; seule la bande découverte est rasterisée, plus les bords de ce qui ne défile
  pas (ombres de bord de la zone de défilement, coins arrondis et contour de la carte).
- **Repli** : rendu complet au premier affichage, après redimensionnement, restauration,
  changement d'échelle ou de thème, ou si plus de 60 % des tuiles sont touchées.
  `FOLDER_CPU_FULL_REDRAW=1` désactive le rendu partiel (diagnostic, mesures A/B).
- **Exactitude** : `renderbench --verify` compare chaque image partielle à un rendu complet
  (défilements variés, survol, clics, Ctrl+A, menu contextuel ; 1080p, 1440p, 4K à 150 %,
  1600×900 à 125 %) : 600 images par configuration, aucune fausse ; écart maximal 2 niveaux
  par canal, sur des primitives texturées coupées par un rectangle (interpolation des UV).
  Le test `partial_render_matches_full_render` (`tools/renderbench`) le vérifie dans
  `cargo test --workspace`.

En fenêtre (`memprobe --scroll`, 100 002 entrées, 3 lignes par image ; machine chargée,
mesures alternées complet/partiel avec le même binaire, 4 manches chacune) :

| Mesure | Rendu complet | Rendu partiel | Budget |
|---|---|---|---|
| Défilement 1920×1080 : p50 / p95 | 3,4–3,5 / 4,1–4,3 ms | 2,2–2,3 / **2,7–2,9** ms | p95 ≤ 8 ms |
| Mémoire pendant le défilement 1080p | 28,8–28,9 Mo | 31,3–32,4 Mo | ≤ 60 Mo |
| Défilement 2560×1440 : p50 / p95 | 5,4–5,8 / 6,4–7,6 ms | 3,3–3,7 / **3,9–4,4** ms | p95 ≤ 8 ms |
| Mémoire pendant le défilement 1440p | 34,7–35,0 Mo | 38,1–38,6 Mo | ≤ 60 Mo |

Avant toute modification, la même mesure donnait 1440p p95 7,7–7,9 ms et 1080p p95
4,7–4,9 ms (3 exécutions chacune). En 1440p, une image de défilement coûte désormais ~0,7 ms
d'egui, ~0,2 ms de tessellation, ~0,4 ms de comparaison et de recopie, ~1 ms de
rasterisation (≈ 10 % des pixels) et ~1,5 ms de présentation.

Hors écran (`renderbench`, 300 images, deux passes ; « complet » n'efface plus le tampon,
comme folder depuis la piste 1, d'où des valeurs plus basses que le premier tableau) :

| Résolution | Rasterisation p95 complet → partiel | Total p95 complet → partiel | Δ mémoire partiel |
|---|---|---|---|
| 1920×1080 | 2,4–3,7 → 1,2–1,7 ms | 3,0–5,1 → **1,7–2,5** ms | 21 Mo |
| 2560×1440 | 3,9–5,8 → 2,0–2,5 ms | 4,6–7,0 → **2,6–3,5** ms | 27–28 Mo |
| 3840×2160 @150 % | 13,8–15,4 → 3,2–4,6 ms | 14,9–16,8 → **3,8–5,7** ms | 46–50 Mo |
| 3840×2160 @100 % | 9,3–16,7 → 3,1–5,3 ms | 10,3–18,3 → **3,8–6,9** ms | 47 Mo |

Coût mémoire : l'image précédente (maillages en pixels et empreintes) est conservée, soit
+1,3 Mo au repos (1k entrées : 15,6 Mo contre 14,3 Mo en rendu complet) et +3 à +4 Mo en
défilement 1440p. Pas de thread supplémentaire.

**Décision (2026-10-18)** : le rendu CPU sans cache est le moteur par défaut ; `glow` reste
disponible (`--renderer glow` ou `FOLDER_RENDERER=glow`), `wgpu` derrière la feature `wgpu`
(`cargo build -p folder --features wgpu`). L'optimisation 4K est une tâche de phase 1 (§9).

---

## 4. Organisation du code (workspace Cargo)

```
folder/                      (application « folder », anciennement « fx »)
├─ Cargo.toml                workspace
├─ crates/
│  ├─ fx-core/               types de domaine purs, sans I/O ni unsafe (instantané, tri, sélection, formats)
│  ├─ fx-win/                SEULE crate avec `unsafe` Win32/COM (wrappers sûrs) : énumération,
│  │                         IFileOperation, IContextMenu, presse-papiers, images shell, watch IOCP, DWM
│  ├─ fx-fs/                 énumération en flux, service de surveillance, recherche récursive, archives zip,
│  │                         analyse de l'espace et suggestions de nettoyage
│  ├─ fx-shell/              worker STA unique : icônes, miniatures, ouvrir/ouvrir avec/propriétés
│  ├─ fx-commands/           métadonnées des commandes (libellé, glyphe, raccourcis), raccourcis clavier
│  ├─ fx-store/              réglages TOML, session, SQLite (étiquettes, préférences par dossier)
│  ├─ fx-preview/            aperçus texte/code (coloration maison), markdown, infos
│  ├─ fx-app/                état applicatif, Msg, update (pur), Effect, scheduler, historique, opérations
│  └─ fx-ui/                 rendu egui (vues, thème, caches de textures) ; lancement des deux moteurs
├─ app/folder/                   binaire : main, manifeste, ressources
├─ vendor/egui_software_backend/  rasteriseur CPU tiers porté vers egui 0.37 (cf. VENDOR.md)
└─ tools/
   ├─ gen-tree/              génère des arborescences de test (1k/100k)
   ├─ memprobe/              mesure mémoire privée / threads / première image / défilement
   └─ renderbench/           coût de rendu hors écran, 1080p à 4K
```

`fx-providers` n'existe pas encore (cf. §11) : les emplacements sont un `Location`
(`Dir`, `Home`, `Search`, `RecycleBin`…) traité directement par `fx-app`.

Règles de dépendance (revue) :

```
fx-ui ──► fx-app ──► fx-commands ──► fx-core
   │         │──► fx-fs ──► fx-win
   │         │──► fx-shell ─► fx-win
   │         │──► fx-store ─► fx-commands
   │         └──► fx-preview ─► fx-win
   └──► fx-win (uniquement : menu shell, glisser vers l'extérieur, cadre DWM — §11)
```

- `fx-core` ne dépend de rien de lourd (ni `windows`, ni `egui`).
- `fx-ui` ne fait **jamais** d'I/O : il lit l'état et émet des messages.
- Tout `unsafe` est dans `fx-win`, chaque bloc avec un commentaire `// SAFETY:`.

---

## 5. Modèle de données

### 5.1 Instantané de dossier compact (structure de tableaux)

Pas d'objet alloué par fichier. Un dossier listé est un `DirSnapshot` :

```rust
pub struct DirSnapshot {
    pub location: Location,          // provider + chemin
    pub generation: u64,             // incrémenté à chaque navigation (annulation)
    names: NameArena,                // octets WTF-8 contigus (OsStr::as_encoded_bytes)
    meta: Vec<EntryMeta>,            // 40 octets par entrée
    order: Vec<u32>,                 // vue triée (indices dans meta)
    visible: Option<Vec<u32>>,       // vue filtrée (filtre rapide), None = tout
    pub stats: DirStats,             // nb fichiers/dossiers, taille totale connue
}

#[repr(C)]
pub struct EntryMeta {               // 40 octets
    name: NameSpan,                  // u32 offset + u32 len          (8)
    size: u64,                       //                               (8)
    modified: i64,                   // FILETIME brut                 (8)
    created: i64,                    //                               (8)
    attrs: u32,                      // FILE_ATTRIBUTE_*              (4)
    ext: ExtId,                      // u16, extension internée       (2)
    flags: EntryFlags,               // u16 : dossier, caché, lien, cloud, tag… (2)
}
```

- 100 000 entrées ≈ 4 Mo (meta) + ~2 Mo (noms) + 0,4 Mo (ordre).
- Les noms sont stockés en **WTF-8** pour supporter les noms Windows contenant des
  surrogates isolés ; l'affichage utilise une conversion à la volée (`to_string_lossy`)
  uniquement pour les lignes visibles, avec petit cache LRU de `egui::Galley`.
- Les propriétés étendues (dimensions d'image, durée média, version de fichier, auteur…)
  ne sont **jamais** chargées en masse : elles sont demandées pour les lignes visibles,
  si la colonne correspondante est affichée, et stockées dans une table creuse
  `HashMap<u32, ExtProps>` bornée.
- `ExtId` : table d'extensions internée par processus (`.txt` → 17), sert aussi de clé au
  cache d'icônes.

### 5.2 Tri, groupement, filtre

- Tri naturel compatible Explorateur : `StrCmpLogicalW` (via `fx-win`) ou implémentation
  Rust équivalente testée contre elle. Clé précalculée pour éviter la reconversion UTF-16
  à chaque comparaison.
- Tri **stable**, sur `order` uniquement (les `meta` ne bougent pas) ; `rayon` est
  **interdit** (threads supplémentaires) — tri parallèle maison sur le pool existant au-delà
  de 200 000 entrées seulement.
- Groupement = vecteur de plages `(group_key, start, end)` sur `order`.
- Filtre rapide (taper au clavier) : recherche sous-chaîne insensible à la casse
  sur les noms, incrémentale (on filtre `visible` quand la requête s'allonge).

### 5.3 État applicatif

```rust
pub struct AppState {
    pub windows: Vec<WindowState>,
    pub settings: Settings,
    pub tags: TagIndex,
    pub ops: OperationCenter,        // opérations en cours (centre d'état)
    pub clipboard: ClipboardState,
}
pub struct WindowState { pub tabs: Vec<Tab>, pub active_tab: usize, pub sidebar: SidebarState, /* volets */ }
pub struct Tab { pub panes: PaneLayout, pub title: CompactString }
pub enum PaneLayout { Single(Pane), Dual { a: Pane, b: Pane, arrangement: Arrangement, focus: Side } }
pub struct Pane {
    pub history: History,            // back/forward bornés (100 entrées)
    pub view: ViewKind,              // Details | List | Grid(size) | Cards | Columns
    pub listing: Listing,            // Loading{snapshot partiel} | Ready(DirSnapshot) | Error
    pub selection: Selection,        // bitset ou ranges sur `order`
    pub scroll: ScrollState,
    pub layout_prefs: LayoutPrefs,   // colonnes, tri, groupement (persisté par dossier)
}
```

---

## 6. Flux de données et threads

### 6.1 Boucle « message → update → effets » (style Elm)

```
 ┌────────── UI thread (egui) ──────────┐
 │ rend l'état ; entrées → Msg          │
 │ drain(rx) → update(&mut state, msg)  │──► Vec<Effect> ──► Scheduler
 └──────────────▲───────────────────────┘                     │
                │ Msg (résultats)  + ctx.request_repaint()     ▼
          crossbeam rx  ◄──────────────────────  workers (pool borné)
```

- `update` est **pur et rapide** (pas d'I/O, < 1 ms). Il retourne des `Effect`
  (`Enumerate`, `Watch`, `LoadIcons`, `RunFileOp`, `SaveSettings`…).
- Les workers renvoient des `Msg` ; chaque envoi déclenche `egui::Context::request_repaint()`.
- Chaque requête liée à une navigation porte `(pane_id, generation)` : les résultats d'une
  génération périmée sont ignorés ; un `CancelToken` (AtomicBool) interrompt le travail.

### 6.2 Threads (≤ 12 au repos)

| Thread | Rôle |
|---|---|
| UI (1) | egui/winit, COM STA (requis pour drag-drop OLE, menus contextuels, presse-papiers) |
| Pool I/O (2–4) | énumération, tailles de dossiers, propriétés étendues |
| Pool shell (1–2, STA) | icônes, miniatures (`IShellItemImageFactory`), aperçus shell |
| Watcher (1) | toutes les surveillances `ReadDirectoryChangesW` via un seul IOCP |
| Opérations (1 par opération active, max 3) | `IFileOperation` (STA) ; les suivantes sont mises en file |
| Persistance (1) | écritures SQLite/TOML regroupées (debounce 500 ms) |

Les pools sont créés au démarrage avec une taille fixe ; aucune création de thread à la volée
dans les chemins chauds.

---

## 7. Sous-systèmes

### 7.1 Énumération (`fx-fs::enumerate`)

1. Ouvrir le dossier avec `CreateFileW(FILE_LIST_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS)` puis
   `GetFileInformationByHandleEx(FileFullDirectoryInfo / FileIdBothDirectoryInfo)` avec un
   **tampon réutilisé de 64 Ko** (un seul appel système renvoie des centaines d'entrées).
2. Repli : `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)` + `FindNextFileW`
   — exactement ce que fait Files (observé), utile pour les chemins réseau/particuliers.
3. Diffusion en continu : envoyer un lot à l'UI tous les **512 éléments ou toutes les 8 ms**.
   Le premier lot est trié et affiché immédiatement ; le tri final s'applique à la fin
   (en conservant la sélection et la position de défilement).
4. Fichiers cachés/système filtrés à l'énumération selon les réglages (pas après coup).
5. Taille des dossiers (option) : calcul paresseux, uniquement pour les lignes visibles,
   cache par `(volume, FileId)`.

### 7.2 Surveillance (`fx-fs::watch`)

- Un handle par dossier affiché, `ReadDirectoryChangesW` asynchrone (OVERLAPPED) sur un
  **IOCP unique** ; tampon 64 Ko ; débordement → ré-énumération complète.
- Événements regroupés (**debounce 50 ms**) puis appliqués comme diff sur le `DirSnapshot`
  (insert/remove/update + renommage old→new), sans recharger le dossier.
- Désabonnement immédiat quand le volet quitte le dossier.
- Les opérations de folder ne dépendent pas de la surveillance : à leur fin, les chemins
  touchés (sources, destinations, éléments créés, archive extraite) situés dans un dossier
  affiché sont relus (`fx_fs::restat`, pool d'I/O) et appliqués comme les notifications.
  Certains lecteurs virtuels (Google Drive) ou réseau ne signalent rien ; ailleurs, la
  relecture retrouve l'état déjà appliqué et `DirSnapshot::upsert` l'ignore (pas de
  décalage O(n)).

### 7.3 Opérations sur fichiers (`fx-fs::ops`)

- Copier / déplacer / supprimer / renommer / nouveau : **`IFileOperation`** (comme Files) pour
  bénéficier de la corbeille, des boîtes de conflit natives optionnelles, de l'élévation
  UAC et du comportement Explorateur. Notre `IFileOperationProgressSink` alimente le
  centre d'état (progression, débit, éléments).
- Mode « silencieux » (`FOF_NO_UI`) + gestion des conflits par notre propre dialogue
  (Remplacer / Ignorer / Renommer / Appliquer à tous), comme `FileNameConflictResolveOptionType`.
- **Annuler/Rétablir** : journal des opérations (`OpHistory`) avec l'inverse de chaque
  opération (renommer ↔ renommer, déplacer ↔ déplacer, supprimer → restaurer depuis la
  corbeille via `IShellItem` de la corbeille).
- Renommage en masse : prévisualisation, motif (`{name}`, `{n:000}`, regex) ; exécuté en une
  seule `IFileOperation`.

### 7.4 Intégration shell (`fx-shell`)

- **Icônes** : `SHGetFileInfoW(SHGFI_SYSICONINDEX)` + `SHGetImageList` ; cache par `ExtId`
  (sauf `.exe`, `.lnk`, `.ico`, `.url` : cache par chemin). Textures egui créées à la
  demande, cache LRU borné (par défaut 8 Mo de textures).
- **Miniatures** : `IShellItemImageFactory::GetImage` (exploite le cache de miniatures de
  Windows ; pas de décodeurs d'images embarqués). Seulement pour les éléments visibles en
  vue Grille/Cartes ; requêtes annulées si l'élément sort de l'écran ; cache LRU borné
  (défaut 32 Mo, réglable). Repli : icône. Réessai sur `FILE_ACTION_MODIFIED`, comme Files.
- **Menu contextuel** : on affiche notre menu (commandes folder) + sous-menu / section
  « Plus d'options » qui construit le vrai menu shell via `IContextMenu` /
  `IContextMenu2/3` (`QueryContextMenu` → `TrackPopupMenuEx` → `InvokeCommand`),
  avec `HandleMenuMsg` pour les sous-menus dynamiques (Envoyer vers, Ouvrir avec…).
  (Réalisé autrement : le menu shell est lu puis dessiné par folder, cf. §11.1 « Shell ».)
- **Glisser-déposer** : OLE `IDropTarget` sur la fenêtre (`RegisterDragDrop`) et
  `DoDragDrop` + `IDataObject` shell (`SHCreateDataObject`) en sortie ; compatible avec
  l'Explorateur et les autres applications.
- **Presse-papiers** : `CF_HDROP` + `Preferred DropEffect` (copier vs couper),
  interopérable avec l'Explorateur.
- **Propriétés** : dialogue natif `SHObjectProperties` en phase 1 ; notre fenêtre Propriétés
  (hachages, ACL, signatures…) en phase 3.
- **Emplacements spéciaux** : Corbeille, Ce PC, Réseau, Bibliothèques via
  `IShellFolder`/`IShellItem` (provider `ShellNamespace`).
- Toute la COM vit dans des threads STA (`CoInitializeEx(COINIT_APARTMENTTHREADED)`).

### 7.5 Providers (`fx-providers`)

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;
    fn enumerate(&self, loc: &Location, sink: &mut dyn BatchSink, cancel: &CancelToken) -> Result<()>;
    fn watch(&self, loc: &Location) -> Option<WatchHandle>;  // None = pas de surveillance
    fn capabilities(&self, loc: &Location) -> Capabilities;   // lecture seule, renommer, corbeille…
}
```

Implémentations : `LocalFs` (P0), `ShellNamespace` (P0 : Ce PC, Corbeille), `Archive`
(P2 : parcourir un .zip/.7z comme un dossier), `Wsl` (via `\\wsl.localhost\`, P2),
`Ftp` (P3, optionnel).

### 7.6 Commandes et raccourcis (`fx-commands`)

Reprise du modèle de Files (`CommandCodes` + `IAction` + `HotKey`) :

```rust
pub trait Action: Send + Sync {
    fn id(&self) -> CommandId;                     // enum généré par macro
    fn label(&self) -> &'static str;               // clé de traduction
    fn glyph(&self) -> Option<char>;               // Segoe Fluent Icons
    fn default_hotkeys(&self) -> &'static [HotKey];
    fn is_executable(&self, cx: &CommandContext) -> bool;
    fn execute(&self, cx: &mut CommandContext) -> Vec<Effect>;
}
```

- Un seul registre alimente : barre d'outils, menus, menu contextuel, raccourcis clavier,
  **palette de commandes** (mode de l'omnibar), éditeur de raccourcis.
- `CommandContext` expose la sélection, le volet actif, les capacités du provider.
- Liste initiale (P0) : NavigateBack/Forward/Up/Home, NewTab, CloseTab, NextTab/PreviousTab,
  ReopenClosedTab, NewWindow, Copy/Cut/Paste, CopyPath, Delete, DeletePermanently, Rename,
  CreateFolder, CreateFile, SelectAll, InvertSelection, ClearSelection, Refresh, EditPath,
  Search, ToggleShowHiddenItems, ToggleShowFileExtensions, Layout{Details,List,Grid},
  SortBy*, OpenItem, OpenWith, OpenInNewTab, OpenTerminal, OpenProperties, Undo, Redo.

### 7.7 Interface (`fx-ui`)

```
┌─────────────────────────────────────────────────────────────────────┐
│ Onglets ─────────────────────────────────────────────── [ _ □ X ]   │  barre de titre personnalisée
├─────────────────────────────────────────────────────────────────────┤
│ ← → ↑ ⟳ │ Omnibar : C: › Users › moi › Documents     │ 🔍 │ ⋯ │ ◷  │
├─────────────────────────────────────────────────────────────────────┤
│ Barre d'outils (Nouveau, Couper, Copier, Coller, Renommer, Trier…)  │
├───────────┬───────────────────────────────────────────┬─────────────┤
│ Sidebar   │ Liste virtualisée (Détails/Liste/Grille)  │ Volet info/ │
│ Épinglés  │  Nom          Modifié       Type   Taille │ aperçu      │
│ Lecteurs  │  ...                                      │ (optionnel) │
│ Étiquettes│                                           │             │
├───────────┴───────────────────────────────────────────┴─────────────┤
│ Barre d'état : 1 245 éléments · 3 sélectionnés (12,4 Mo)            │
└─────────────────────────────────────────────────────────────────────┘
```

- **Virtualisation obligatoire** : `ScrollArea::show_rows` (Détails/Liste) et calcul
  de grille manuel (Grille/Cartes) ; on ne construit que les lignes visibles + 1 écran
  de marge.
- **Repaint réactif** : les deux moteurs ne produisent une image que sur événement ;
  aucune animation continue ; les animations (survol, ouverture de volet) durent ≤ 150 ms
  puis s'arrêtent. Avec le rendu CPU, une image ne rasterise que les zones qui ont changé
  (§3.4) : une animation se paie au prorata de la surface qu'elle touche, et une animation
  qui couvre toute la fenêtre coûte une rasterisation complète (~3 ms en 1080p).
- **Textures en RAM** : avec le rendu CPU, icônes et miniatures vivent en mémoire système ;
  leurs caches LRU (§7.4) comptent dans le budget mémoire privée.
- Thème Fluent (clair/sombre suivant `AppsUseLightTheme`, couleur d'accentuation
  système), coins arrondis, Segoe UI Variable, glyphes Segoe Fluent Icons (police système).
  Fond Mica optionnel via `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)`.
- Accessibilité : AccessKit (intégré à egui) ; navigation clavier complète ; tous les
  éléments interactifs atteignables sans souris. **Attention** : le moteur CPU vendu ne
  branche pas encore AccessKit (pas d'appel à `egui_winit::State::init_accesskit`) —
  tâche de phase 1.
- Polices : seules les plages nécessaires sont chargées ; repli CJK chargé à la demande
  si un nom de fichier l'exige.

### 7.8 Persistance (`fx-store`)

| Donnée | Format | Emplacement |
|---|---|---|
| Réglages | `settings.toml` (serde, versionné, migrations) | `%APPDATA%\folder\` |
| Raccourcis personnalisés | `keybindings.toml` | `%APPDATA%\folder\` |
| Session (onglets, volets, historique Précédent/Suivant : 30 entrées par sens) | `session.toml`, écrit à la fermeture + toutes les 30 s si modifié | `%LOCALAPPDATA%\folder\` |
| Préférences par dossier (vue, tri, colonnes) | SQLite `folder.db`, table `layout_prefs(path_hash, …)` | `%LOCALAPPDATA%\folder\` |
| Étiquettes | SQLite `folder.db`, tables `tags`, `tagged(file_id, volume_serial, path, tag_id)` | idem |
| Historique du centre d'état (100 dernières opérations terminées : titre, issue, heure, durée) | SQLite `folder.db`, table `op_history`, écrite à chaque fin d'opération ; partagée par toutes les fenêtres, relue au lancement et à l'ouverture du centre | idem |

Clé d'étiquette : `(numéro de série du volume, FileId NTFS)` + chemin de secours, pour
survivre aux renommages/déplacements sur le même volume (Files stocke `FileFRN`, observé).

### 7.9 Aperçus (`fx-preview`)

Chargés **uniquement** quand le volet d'aperçu est ouvert, sur le thread pool I/O, avec
limite de taille (texte : 1 Mo lus max) :

- Texte / code : détection d'encodage (`chardetng` + BOM), coloration syntaxique simple
  (tokenizer maison par langage courant, ou `syntect` chargé paresseusement — mesurer).
- Image : miniature shell grande taille (`IShellItemImageFactory` 1024 px).
- Markdown : `pulldown-cmark` → widgets egui.
- PDF / Office / autres : **gestionnaires d'aperçu Windows** (`IPreviewHandler`) hébergés
  dans une fenêtre enfant Win32 positionnée sous la zone d'aperçu (phase 2).
- Média : métadonnées via propriétés shell (`IPropertyStore`), lecture hors périmètre.

### 7.10 Recherche

- Filtre instantané dans le dossier courant (§5.2).
- Recherche récursive : parcours parallèle borné (pool I/O), résultats diffusés comme une
  énumération dans un « dossier virtuel » ; annulable.
- Phase 3 : Windows Search (`ISearchFolderItemFactory` / requêtes AQS) pour les index.

### 7.11 Télémétrie et journaux

- Aucune télémétrie réseau par défaut (Files embarque Sentry ; nous non).
- Journal local `tracing` + `tracing-appender` en fichier rotatif, niveau `info` en release.
- Rapport de crash : minidump local optionnel (`MiniDumpWriteDump`), jamais envoyé
  automatiquement.

### 7.12 Mise à jour automatique et notes de version (`fx-app::upgrade`)

Seule requête réseau que folder fait sans qu'on ouvre un emplacement réseau ; décision du
2026-10-28 (auparavant hors périmètre, §9).

- **Source** : releases GitHub du dépôt (`repository` de `[workspace.package]`). La dernière
  version est lue dans la redirection de `…/releases/latest` (GET sans suivre les
  redirections, vers `…/releases/tag/v<version>`) : pas d'API JSON (ni limite de débit, ni
  dépendance d'analyse), préversions exclues par GitHub. Installeur :
  `…/releases/download/v<version>/folder-setup-<version>-x64.exe`. Le dépôt doit être public.
- **Processus éphémère** `folder.exe --update-check` : HTTPS par WinHTTP (`fx_win::http`,
  fourni par Windows, proxy du système, pas de pile TLS embarquée), téléchargement dans
  `%LOCALAPPDATA%\folder\updates` (`.part` renommé une fois complet, taille plafonnée à
  64 Mo, longueur vérifiée), contrôle de l'installeur (`MZ`, et signature Authenticode du
  même éditeur que le `folder.exe` en cours s'il est signé, `fx_win::trust`, sans
  vérification de révocation en ligne), résultat dans `update.toml` et une ligne sur
  stdout. winhttp.dll et ses threads ne sont ainsi jamais chargés dans une fenêtre : mémoire
  et threads au repos inchangés.
- **Fréquence** : réglage `auto_update` (actif par défaut, Paramètres › Mises à jour). Au
  démarrage, un thread temporaire (`fx-update`) lit `update.toml`, partagé par toutes les
  fenêtres (une par processus) : vérifié il y a moins d'une heure (`MAX_AGE`), le résultat
  est repris tel quel (thread terminé en quelques millisecondes) ; sinon le processus de
  vérification part 5 s après le démarrage. Une fenêtre ouverte revérifie chaque heure à
  l'occasion d'une image (aucun réveil programmé). L'ouverture de Paramètres › Mises à jour
  revérifie si le résultat a plus de 5 min (`PAGE_MAX_AGE`) : la page n'annonce jamais
  « à jour » d'après une vieille vérification, et dit quelle est la dernière version
  publiée. La recherche à la demande (commande « Rechercher les mises à jour », bouton des
  Paramètres) ignore le résultat partagé. Désactivée, aucune requête automatique.
  2026-10-30 : d'abord 24 h (6 h pour une fenêtre ouverte) ; une version publiée n'était
  alors proposée qu'après des heures, et la page affichait « à jour » d'après le cache.
- **Délai de publication** : la release n'existe qu'à la fin du workflow Installeur
  (≈ 4 min après le push, plus l'approbation SignPath quand la signature est active) ;
  avant, folder répond à raison que la version installée est la dernière publiée.
- **Rien d'autre n'est envoyé** : deux GET anonymes, agent `folder/<version>`.
- **Installation, seulement à la demande** : bouton « Mettre à jour » de la barre de titre
  (`Msg::InstallUpdate`), commande « Installer la mise à jour », ou Paramètres (relayé à la
  fenêtre principale par `Remote::InstallUpdate`). L'installeur Inno Setup est lancé en
  silencieux (`/SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RELAUNCH=1`), la
  fenêtre se ferme normalement (session enregistrée), le gestionnaire de redémarrage ferme
  les autres fenêtres folder, puis `[Run]` relance folder si `/RELAUNCH=1` (session
  restaurée). Une installation « pour tous les utilisateurs » demande l'élévation (UAC).
  Seul un folder lancé depuis son installation (désinstalleur `unins000.exe` à côté) se met
  à jour ; `cargo run` affiche « Indisponible ».
- **Notes de version** : `CHANGELOG.md` (racine), embarqué par `include_str!` et affiché
  dans Paramètres › Mises à jour › Notes de version (commande « Notes de version » :
  `folder.exe --settings --notes`). Format : `## <version> — <date>` puis une liste ; la
  première section doit être la version de Cargo.toml (test `notes_cover_every_version`).
  Le workflow Installeur en tire la description de la release.

### 7.13 Analyse de l'espace (`fx-fs::diskscan`, `fx-fs::cleanup`, `fx-app::disk`)

Voir ce qui occupe un lecteur ou un dossier et libérer la place, à la WinDirStat. Décision
du 2026-10-30 : un emplacement de volet (pas une fenêtre à part), taille **sur disque**,
suggestions de nettoyage dès la première version.

- **Emplacement** `Location::DiskUsage { root, dir }` : l'analyse de `root`, regardée dans
  `dir`. Ouvrir un dossier, remonter, le fil d'Ariane ou un chemin saisi restent dans
  l'analyse tant qu'on est sous `root` (`update::disk::redirect`, sur toute navigation qui
  ajoute à l'historique) ; « Fermer l'analyse » ouvre `dir` normalement. Non restauré avec
  la session. Points d'entrée : commande « Analyser l'espace » (`Ctrl+Maj+A` : dossier
  sélectionné, sinon dossier affiché, sinon lecteur système), menus contextuels d'un dossier
  et du fond, lecteurs de la barre latérale et de l'accueil, et le bouton de la barre de
  navigation (à côté de la palette, commande « Analyser des lecteurs… »,
  `fx-ui::view::diskpicker`) : menu de cases à cocher des lecteurs (jauge, place libre,
  « déjà analysé » ; lecteur système coché à la première ouverture) et du dossier affiché.
  Plusieurs lecteurs : le premier dans le volet actif, chacun des autres dans un nouvel
  onglet, analyses en parallèle (`Msg::AnalyzeDrives`).
- **Taille sur disque** : `AllocationSize` de `GetFileInformationByHandleEx(FileFullDirectoryInfo)`
  (`fx_win::dir::for_each_sized_entry`, tampon de 64 Ko par thread). Un fichier cloud non
  téléchargé compte 0, un fichier compressé sa taille compressée : c'est ce qu'on libère.
  Points d'analyse (jonctions, liens, points de montage) non suivis, comme l'Explorateur.
  Les liens physiques (WinSxS) sont comptés à chaque nom, comme WinDirStat.
- **Parcours** : un thread coordinateur temporaire (`fx-scan`), seul à écrire l'arbre, et au
  plus 4 threads de lecture temporaires (`std::thread::scope`, pile de 256 Ko), file en
  profondeur sous `Mutex` + `Condvar` ; tous se terminent avec l'analyse (threads au repos
  inchangés). Annulable (`CancelToken`, vérifié entre deux dossiers et tous les 4 096
  éléments). Avancement toutes les 250 ms (`Msg::DiskProgress` : totaux, dossier courant,
  premier niveau) ; la vue l'affiche à la place de la liste.
- **Arbre** `fx_core::DiskTree` (structure de tableaux, nœuds de 48 octets, noms WTF-8 dans
  une arène) : un nœud par dossier, par fichier gardé, et un nœud « petits fichiers » par
  dossier. Un fichier est gardé à partir d'un 200 000ᵉ de l'espace occupé du lecteur
  (entre 4 Kio et 1 Mio ; 64 Kio pour un dossier), seuil relevé à 1 Mio au-delà de 100 000
  fichiers gardés ; seuls leurs noms entrent dans l'arène. Plafond de 1 000 000 de nœuds :
  au-delà, le contenu des nouveaux dossiers est compté dans leur parent (totaux exacts).
  Mesuré : profil utilisateur de 1,67 million de fichiers (535 Go) → 490 000 nœuds,
  28 Mo, 11 s à chaud. Au-delà de 2 analyses (`disk::MAX_SCANS`), celles qu'aucun volet
  n'affiche sont libérées, de la plus ancienne à la plus récente (les analyses affichées,
  plusieurs lecteurs par exemple, restent) ; mémoire visible dans F12.
- **Liste** : le vrai contenu du dossier (`Effect::Enumerate` avec `allocated` et les
  éléments masqués), les dossiers prenant la taille de l'analyse au fil des lots
  (`update::disk::patch_batch`) ; tri par taille décroissante, dossiers et fichiers mêlés ;
  barre de proportion dans la colonne Taille. Sélection, menu contextuel, suppression et
  glisser fonctionnent donc comme ailleurs. Pas de surveillance : les notifications
  donneraient la taille du contenu.
- **Carte proportionnelle** (treemap « squarified », relief en coussin) du dossier regardé,
  au-dessus de la liste : couleur par extension (les 12 plus gourmandes), gris pour les
  petits fichiers. Calculée en pixels physiques dans une image (`fx-ui::view::disk`) quand
  l'arbre, le dossier ou la taille changent ; les tuiles de moins de 36 points² ne sont pas
  subdivisées. L'image, calée sur la grille des pixels et une colonne plus large que la
  zone affichée, suivie d'un triangle vide dans son maillage, prend le chemin de recopie
  directe du moteur CPU (sinon : deux triangles échantillonnés, ~7 ms en 1080p). Survol :
  contour et info-bulle ; clic : sélection dans la liste ; double-clic : ouvrir le dossier.
- **Mise en page adaptée à la fenêtre** : la carte prend 45 % de la hauteur du volet
  (séparateur déplaçable entre 12 et 85 %, double-clic : retour à 45 %), la liste gardant
  au moins son en-tête et trois lignes. Pendant un redimensionnement, l'image en place est
  étirée et les tuiles mises à l'échelle ; disposition et image ne sont recalculées
  qu'une fois la taille stable depuis 120 ms. Sous 760 points de large (petite fenêtre,
  double volet) : résumé sur une seconde ligne, libellés raccourcis, titre tronqué ; la
  page Types masque la description puis le nombre de fichiers. Colonnes de la liste
  proportionnelles à la largeur du volet (`list::disk_layout`, sans séparateurs à glisser,
  les réglages de largeur des autres vues n'y s'appliquent pas) ; la date puis le type
  disparaissent quand la place manque, la taille reste toujours visible.
- **Pages** : Contenu (carte + liste), Types de fichiers (place par extension, état à la
  fin de l'analyse), Récupérable (suggestions).
- **Suggestions** (`fx_fs::cleanup::suggest`, calculées avec l'arbre) : corbeille ;
  contenu des dossiers temporaires, de `SoftwareDistribution\Download`, des rapports
  d'erreur, des caches de navigateurs (Chrome, Edge, Brave, Firefox) et de paquets (npm,
  pip, Yarn, NuGet, pnpm, Cargo, Gradle) ; `Windows.old` ; dossiers `node_modules` et
  marqués `CACHEDIR.TAG` (cible Cargo…) de plus de 50 Mo ; fichiers de plus de 100 Mo
  dans Téléchargements ; fichiers de plus de 1 Go non modifiés depuis un an (hors Windows,
  Program Files, ProgramData, fichiers de pagination). Régénérables : supprimés
  définitivement (les mettre à la corbeille ne libérerait rien) ; données de l'utilisateur :
  mises à la corbeille. Toujours après confirmation (`Dialog::ConfirmClean`, place libérée
  et éléments), par `IFileOperation` dans le centre d'état ; le contenu d'un dossier est
  listé au moment du nettoyage (`Effect::ListContents`), les éléments en cours d'utilisation
  sont ignorés.
- **Après une opération** (de folder) touchant un dossier analysé : son parent est relu
  sans récursion (`Effect::RefreshDisk` → `DiskTree::refresh` : fichiers recomptés,
  sous-dossiers disparus retirés, nouveaux lus en entier), ainsi que la corbeille du
  lecteur ; les suggestions et les volets suivent. Les changements faits hors de folder
  attendent « Relancer ». La place libre des lecteurs (barre latérale, accueil, barre du
  volume) est relue à la fin de chaque opération, au plus une fois par seconde pendant une
  longue opération, et quand la corbeille est vidée ; une mise à la corbeille ne libère
  rien tant qu'elle n'est pas vidée, ce que rappelle la confirmation dans l'analyse.
- **Sélection** : « Tout sélectionner » et « Inverser la sélection » dépendent de la
  présence d'une liste (`CommandContext::has_listing`), plus d'un dossier où créer : ils
  marchent dans l'analyse, la corbeille et les résultats de recherche.
- **Coût du rendu** (`renderbench --disk`, 1080p) : 0,5 ms par image au survol (rendu
  partiel), 3,3 à 4,5 ms pour une image complète (2,6 ms pour la liste seule).

---

## 8. Mesure et vérification des performances

- `tools/gen-tree` crée des jeux de test reproductibles (1k, 100k, 1M fichiers ;
  noms Unicode, chemins longs > 260, liens, points de jonction, fichiers cachés).
- `benches/` (criterion) : énumération, tri naturel, filtre, application de diffs du watcher,
  construction de la vue grille.
- `tools/memprobe` : lance `folder.exe`, navigue via arguments de ligne de commande
  (`folder.exe --bench-nav <dossier>`), relève `PrivateUsage` (`GetProcessMemoryInfo`), le
  nombre de threads et le temps jusqu'à la première image ; échoue si un budget du §2
  est dépassé. À exécuter avant chaque fusion touchant `fx-fs`, `fx-app` ou `fx-ui`.
- Overlay développeur (F12) : temps de frame, mémoire privée, taille des caches,
  nombre de textures, file des workers.

---

## 9. Plan de livraison

État au 2026-10-18 : phases 0 et 1 implémentées, l'essentiel des phases 2 et 3 aussi ;
détail et écarts en §11.

### Phase 0 — Squelette (fondations mesurables)
Workspace, `fx-win` (COM init, HRESULT → `Result`), fenêtre (rendu CPU par défaut,
eframe/glow en option), thème, boucle Msg/Update/Effect, `memprobe`, `gen-tree`,
`renderbench`, CI (build, clippy `-D warnings`, tests, benches de fumée). **Terminée**
(2026-10-18) ; mesures en §3.3–3.4.

### Phase 1 — MVP « remplace l'Explorateur au quotidien »
Énumération + streaming, vue **Détails** virtualisée, tri, sélection (clic, Maj, Ctrl,
rectangle), navigation + historique, omnibar (chemin) + fil d'Ariane, sidebar (épinglés,
lecteurs, dossiers connus), **onglets**, surveillance, icônes, `IFileOperation`
(copier/couper/coller/supprimer/renommer/nouveau), presse-papiers interop, menu
contextuel shell, glisser-déposer OLE, ouvrir/ouvrir avec, filtre rapide, réglages
de base, restauration de session, raccourcis par défaut.

Tâches de rendu issues de la phase 0 :
- **Optimisation du rendu CPU en 4K** — objectif : défilement p95 ≤ 8 ms en 3840×2160
  (100 % et 150 %), mesuré par `renderbench` puis confirmé en fenêtre sur un écran 4K.
  Pistes, par ordre de rapport gain/effort :
  1. ne pas effacer le tampon (`fill(0)`) quand un panneau opaque couvre toute la fenêtre —
     **fait** ;
  2. rendu partiel : ne rasteriser que les rectangles modifiés (survol, sélection, barre
     d'état) et ne présenter que ces zones (`softbuffer` `present_with_damage`) — **fait** ;
  3. défilement par décalage : recopier les pixels existants de la liste et ne dessiner que
     les lignes nouvellement visibles — **fait** (détection automatique, sans aide de
     `fx-ui`) ;
  4. en dernier recours, rasterisation parallèle par bandes horizontales sur un pool borné
     (≤ 4 threads, pas de rayon), à mesurer contre le budget de threads — inutile à ce jour.

  Résultat (§3.4) : 4K hors écran p95 3,8–6,9 ms (contre 10,3–18,3 ms) ; 1440p en fenêtre
  p95 3,9–4,4 ms. Reste la confirmation en fenêtre sur un écran 4K (machine de mesure limitée
  à 2560×1440).
- **AccessKit dans le moteur CPU** : brancher `egui_winit::State::init_accesskit` dans la
  boucle vendue ; vérifier avec le Narrateur.
- **Tri naturel ≤ 50 ms sur 100k** (aujourd'hui ≈ 52 ms) : clés de tri précalculées.

### Phase 2 — Confort
Vues **Liste/Grille/Cartes** avec miniatures, **double volet**, volet d'aperçu/info,
centre d'état (progression, débit), annuler/rétablir, résolution de conflits, palette de
commandes, éditeur de raccourcis, barre d'outils personnalisable, groupement, préférences
par dossier, Corbeille, Ce PC, WSL, détection des lecteurs cloud et statut de synchro,
**étiquettes**, page d'accueil (widgets), parcours d'archives zip.

### Phase 3 — Parité étendue
Vue **Colonnes (Miller)**, création/extraction d'archives (zip/7z), renommage en masse,
fenêtre Propriétés complète (hachages, ACL, signatures), recherche Windows Search, Git
(statut, branches, puis fetch/pull/push), Shelf, multi-fenêtres dans un même processus,
traduction (fichiers Fluent `.ftl` via `fluent-bundle`), FTP (optionnel), montage ISO
(`MountVhd`/shell « Monter »).

### Hors périmètre
WebView, moteur JavaScript, lecteur multimédia intégré, installation d'une mise à jour sans
demande de l'utilisateur (la vérification et le téléchargement sont automatiques, §7.12),
télémétrie distante.

---

## 10. Risques et parades

| Risque | Parade |
|---|---|
| Rasteriseur CPU vendu (crate 0.0.3) : bogues de rendu, maintenance à notre charge | Copie isolée dans `vendor/` avec `VENDOR.md` ; modifications minimales ; moteur `glow` toujours disponible en repli (`--renderer glow`) ; `renderbench` et `memprobe --scroll` en garde-fou. |
| Rendu partiel : une zone modifiée non détectée laisserait une trace de l'image précédente | Rendu partiel comparé pixel à pixel au rendu complet (`renderbench --verify`, test `partial_render_matches_full_render`) ; replis en rendu complet (redimensionnement, restauration, thème, changement d'échelle) ; `FOLDER_CPU_FULL_REDRAW=1` pour l'écarter en cas de doute. |
| Rendu CPU trop lent sur grands écrans / machines modestes | Tâche d'optimisation 4K (§9, phase 1) ; mesure systématique 1080p/1440p/4K ; repli GPU possible par machine. |
| egui ne tient pas un budget (ex. texte massif en grille) | Mesurer tôt (phase 1) ; cache de galleys ; si besoin, rendu custom de la liste via `egui::Painter` ; plan B Slint, isolé par `fx-ui`. |
| Extensions shell tierces lentes ou qui plantent (menus contextuels, aperçus) | Appels shell dans des threads STA dédiés avec délai d'expiration ; aperçus hébergés hors du thread UI ; jamais d'appel shell bloquant sur le thread UI sauf les options du menu contextuel (`IContextMenu`). |
| Dossiers réseau lents | Énumération toujours asynchrone + indicateur de chargement + annulation ; pas de surveillance sur les partages lents (option). |
| Chemins longs / noms exotiques | Préfixe `\\?\` systématique dans `fx-win` ; noms WTF-8 ; jeux de tests dédiés. |
| Fuite mémoire de textures | Caches LRU bornés en octets, compteurs dans l'overlay, test `memprobe` en navigation répétée (1 000 navigations → dérive < 2 Mo). |

---

## 11. État de l'implémentation (2026-10-18)

### 11.1 Fonctionnalités livrées

- **Navigation** : onglets (dupliquer, rouvrir le dernier fermé, glisser : l'onglet reste
  attaché au pointeur au point saisi, curseur normal ; dans la barre les autres s'écartent
  et l'ordre change au relâchement ; tiré hors de la barre il flotte et, relâché, part dans
  une nouvelle fenêtre de même taille, son onglet sous le pointeur :
  `folder.exe --rect G,H,D,B <emplacement>`, ou, lâché sur la barre d'onglets d'une autre
  fenêtre folder, la rejoint ; seul onglet de sa fenêtre, c'est la fenêtre entière qui suit
  le pointeur (restaurée si elle était agrandie), masquée au-dessus de la barre d'une autre
  fenêtre, et qui s'y fond au relâchement en se fermant sans enregistrer sa session ; suivi maison, `egui::DragAndDrop` imposant le curseur « saisie »,
  flèche multidirectionnelle sous Windows),
  historique par volet,
  omnibar (fil d'Ariane cliquable, édition du chemin), page d'accueil (accès rapide,
  lecteurs avec jauge, étiquettes, fichiers récents), barre latérale (accueil, accès rapide
  avec la corbeille, lecteurs, réseau, distributions WSL lues dans le registre, étiquettes ;
  icônes de Windows), restauration de session (onglets, dossiers ouverts,
  écran, taille et état agrandi de la fenêtre : créée cachée, placée par
  `SetWindowPlacement` en pixels physiques puis affichée directement dans son état final), branche Git en bas à droite de la barre d'état (logo, menu des branches locales et distantes, changement de branche par `git switch` sur un thread temporaire, dossier relu ensuite).
- **Vues** : Détails (colonnes ancrées à gauche comme l'Explorateur : chaque séparateur ne
  redimensionne que la colonne à sa gauche, Nom automatique jusqu'au premier redimensionnement,
  colonnes coupées au bord plutôt que masquées ; séparateurs créés après les en-têtes et
  largeur suivant la position du curseur ; tailles lisibles en Ko, Mo, Go), Liste,
  Grille (miniatures shell, zoom), **Colonnes (Miller)** ; tri naturel, dossiers en premier,
  éléments masqués / extensions, préférences de vue et de tri par dossier ; sélection
  clic, Maj, Ctrl, rectangle, cases à cocher posées sur l'icône au survol (case
  « tout sélectionner » au-dessus des icônes dans l'en-tête, ou en haut des vues Liste et
  Grille) ;
  recherche par frappe ; Ctrl + molette (et Ctrl + Plus/Moins)
  pour passer de Détails à Liste puis aux icônes de 48 à 256 points ; **double volet** ; volet d'aperçu
  (texte et code colorés, markdown, grande miniature, infos).
- **Opérations** : `IFileOperation` avec progression (centre d'état : bouton de la barre de
  navigation à côté de Rechercher et de la palette, menu déroulant avec les opérations en
  cours — progression, annulation — puis l'historique des opérations terminées, borné à
  100 entrées sans liste de chemins, enregistré dans `folder.db` (conservé entre les
  lancements, commun à toutes les fenêtres), effaçable ; anneau de progression sur le bouton et
  pastille rouge pour un échec pas encore vu), copier / couper / coller interopérables avec l'Explorateur (`CF_HDROP`),
  renommage en place (F2, ou clic lent sur l'élément déjà sélectionné comme dans
  l'Explorateur, champ aligné sur le nom), renommage en masse (motif + aperçu), corbeille / suppression
  définitive, nouveau dossier / fichier, **annuler / rétablir**, compression et extraction
  zip, ouvrir avec, terminal, exécuter en administrateur.
- **Corbeille** dans folder : lecture directe de `X:\$Recycle.Bin\<SID>\` (`$I…` : chemin
  d'origine, taille, date ; `$R…` : l'élément), colonnes « Supprimé le » et « Emplacement
  d'origine », restaurer (sélection ou tout, `IFileOperation::MoveItem` vers le dossier et le
  nom d'origine), supprimer définitivement, vider ; rechargée après chaque opération.
- **Réseau** : énumération du dossier shell Réseau sur un thread STA temporaire, affichée au
  fil de la découverte (les appareils arrivent en ~2 s, la découverte dure ~30 s) ;
  ordinateurs → leurs partages → dossiers ; autres appareils ouverts par le shell
  (`ShellExecuteEx` sur leur PIDL).
- **Propriétés** : vraie fenêtre Windows (barre de titre native, déplaçable hors de la
  fenêtre principale), dans un processus `folder.exe --properties` lancé à la demande (le
  moteur CPU ne gère qu'une fenêtre par processus ; ~11 Mo, 9 threads, visible en ~120 ms,
  terminé à la fermeture). Onglets **Général** (icône, nom modifiable, type, « S'ouvre avec »,
  taille et contenu au fil du calcul, dates, attributs, espace d'un lecteur, corbeille),
  **Partage** (partages lus dans `LanmanServer\Shares`, partage avancé de Windows),
  **Sécurité** (propriétaire et autorisations par compte via `GetNamedSecurityInfoW`,
  « Modifier… » ouvre la page Sécurité de Windows), **Détails** (liste
  `System.PropList.FullDetails` de l'Explorateur, valeurs mises en forme par Windows).
- **Animations** (≤ 180 ms, cadencées sur l'écran) : transition du contenu à chaque
  navigation (glissement depuis la droite en entrant ou en avançant, depuis la gauche en
  revenant ou en remontant, fondu sinon ; Précédent/Suivant marquent explicitement leur
  sens, l'historique désignant déjà la destination), apparition des onglets et glissement
  des autres, menus contextuels et déroulants (zoom et fondu), surlignage de la barre
  latérale qui glisse d'un élément à l'autre (position et hauteur : les lignes des lecteurs
  grandissent avec leur jauge, libellé et jauge centrés ; position retenue par rapport au
  contenu qui défile, si bien qu'il reste collé à sa ligne pendant le défilement et la
  retrouve en place quand elle revient dans la vue), sélection en fondu.
- **Style** (couleurs unies, inspiré de Files avec une touche futuriste) : petites capitales
  espacées pour les sections et les en-têtes de colonnes, liseré « néon » sous l'onglet actif
  et en haut des cartes, halo d'accent partant de la pastille de sélection (liste, barre
  latérale), carte du contenu en relief (ombre douce, reflet), champ d'adresse en pilule
  (contour 1,5 px) avec halo d'accent qui apparaît en 200 ms au focus (saisie centrée sur la
  hauteur d'une ligne), curseurs de réglage opaques (piste en dégradé, poignée pleine),
  bouton « Nouveau » d'accent avec reflet, barres de progression avec reflet, jauges des
  lecteurs en dégradé (deux teintes voisines de l'accent ; épaisseur réglable, 2 à 10 px).
  Les dégradés ne sont posés que sur des bords droits (un maillage n'est pas lissé par le
  rasteriseur) ; aucune animation continue.
- **Couleur d'accentuation** réglable (celle de Windows, 12 couleurs proposées ou libre via un
  sélecteur complet : carré saturation × luminosité, barre de teinte, saisie hexadécimale),
  appliquée à chaud.
- **Chrome** : en haut à droite, boutons Rechercher et Palette de commandes, chacun ouvrant
  sa fenêtre flottante (la recherche propose la recherche récursive et les premiers éléments
  correspondants du dossier) ; barre de commandes où chaque outil a icône et libellé
  (presse-papiers, Trier, Afficher, Étiquette, annuler, sélection, chemin, zip, terminal…),
  affichés tant que la largeur le permet, « … » seulement pour ceux qui débordent ;
  Paramètres en bas à gauche, dans la barre d'état.
- **Images et vidéos** : ouvertes comme depuis l'Explorateur, pour que Photos passe d'un
  fichier à l'autre avec les flèches **dans l'ordre de tri de folder**. Photos lit cet ordre
  dans la vue de dossier qui l'a lancé (service `SID_SFolderView`) et y accède pendant toute
  la navigation ; ni `ShellExecute` ni la requête WinRT `NeighboringFilesQuery` (inopérante
  depuis une application non empaquetée) ne conviennent. Un processus auxiliaire
  `folder.exe --open-media` héberge donc une vraie vue du shell invisible (`IExplorerBrowser`,
  sac de propriétés à part, état non enregistré : l'affichage du dossier dans l'Explorateur
  n'est pas modifié), la trie comme folder, y sélectionne le fichier, invoque son verbe par
  défaut, puis vit tant que la fenêtre ouverte existe (~24 Mo, 23 threads, hors du processus
  principal). La fenêtre principale ne guette qu'un échec rapide (repli `ShellExecute`).
- **Icône** : dossier en perspective (feuilles verte, rouge-orangé et jaune, façade bleue
  vitrée) ; source `app/folder/icon/icone.png` (1254 px), transformée par `make_ico.ps1`
  (System.Drawing, sans dépendance) : pixels parasites du détourage retirés (seule la plus
  grande zone opaque est gardée, avec 3 px d'anticrénelage), recadrage carré, réduction en
  alpha prémultiplié vers `folder.ico` (10 tailles, de 256 à 16 px), `folder-256.png`
  (README) et la petite image de l'assistant d'installation (`installer/wizard-small-*.bmp`) ;
  ressource 1 de l'exécutable, posée sur chaque fenêtre (`WM_SETICON`). Version de
  l'exécutable (propriétés du fichier) : celle de `Cargo.toml`, insérée par `build.rs` dans
  une copie de `folder.rc`.
- **Analyse de l'espace** (§7.13) : carte proportionnelle, liste par taille sur disque,
  types de fichiers, suggestions de nettoyage avec suppression confirmée, mise à jour après
  les opérations de folder.
- **Raccourcis système** (Paramètres › Raccourcis › Système, `fx_win::shortcuts`), actifs
  même folder fermé, appliqués sur le thread shell quand le réglage change :
  « Windows + E ouvre folder » inscrit le processus résident `folder.exe --win-e`
  (`fx_win::win_e`) dans la clé `Run` de l'utilisateur et le lance. Ce processus, sans
  fenêtre ni egui (~2 Mo privés, 4 threads, hors du budget de la fenêtre principale), pose
  un crochet clavier bas niveau (`WH_KEYBOARD_LL`) qui avale Windows + E (sans Ctrl, Alt ni
  Maj), envoie une touche neutre (VK 0xE8) pour que le relâchement de Windows n'ouvre pas le
  menu Démarrer, et lance folder hors du crochet (message posté à son thread) : une fenêtre
  par appui, autorisée à passer au premier plan. Aucune fenêtre folder ouverte (titre et
  exécutable du processus, `fx_win::window::exists_for_exe`) : lancement sans argument,
  comme depuis l'icône, donc session restaurée ; sinon `folder.exe folder:home`. Une seule instance
  par session (mutex `Local\folder-win-e`) ; l'événement `Local\folder-win-e-quit` l'arrête.
  Désactivé : valeur `Run` retirée, processus arrêté, Windows + E rouvre aussitôt
  l'Explorateur. Au démarrage, folder relance le processus s'il manque (option active). La
  méthode du registre (copie de la commande `opennewwindow` de la classe
  `{52205fd8-5dfb-447d-801a-d0b52f2e83e1}`) est ignorée par l'Explorateur de Windows 11 26H2.
  Limite : une fenêtre élevée au premier plan ne transmet pas ses touches au crochet (UIPI).
  L'installeur arrête le processus avant une mise à jour (`--stop-win-e`) et le relance
  ensuite (il se termine aussitôt si l'option est désactivée). Sinon, une touche d'ouverture (au moins deux modificateurs parmi
  Ctrl, Alt et Maj, avec une lettre, un chiffre ou F1–F12, contrainte de l'Explorateur) est
  posée sur le raccourci « folder » du menu Démarrer de l'utilisateur, créé et marqué au
  besoin, supprimé quand la touche est retirée. Le désinstalleur lance
  `folder.exe --restore-explorer`, qui défait les deux.
- **Mises à jour** (§7.12) : vérification quotidienne sur les releases GitHub dans un
  processus éphémère, téléchargement et vérification de l'installeur, bouton « Mettre à
  jour » dans la barre de titre, installation silencieuse puis relance avec la session ;
  Paramètres › Mises à jour (état, recherche, réglage) et notes de version
  (`CHANGELOG.md` embarqué).
- **Souris** : boutons 4 et 5 pour précédent et suivant. Molette sans réglage de vitesse
  (`fx-ui::wheel`, toutes les fenêtres) : la distance d'un cran suit la cadence récente des
  crans (moyenne à décroissance exponentielle, 100 ms) — 60 points (2 lignes de liste) en
  dessous de ~6 crans/s, puis progressivement plus, jusqu'à ×7 (~95 points à 10 crans/s,
  ~190 à 20, ~280 à 30). Le contenu rejoint sa cible par un ressort à amortissement
  critique (90 % en ~0,13 s, arrêt ~0,25 s après le dernier cran) : la vitesse reste
  continue d'un cran à l'autre, sans à-coup ni dépassement ; un changement de sens arrête
  net. Le résultat remplace `smooth_scroll_delta` d'egui en début d'image. Fractions de cran
  (molette libre, pavé tactile) : pas d'accélération ; Ctrl + molette : zoom, inchangé.
  Barres de défilement flottantes : visibles pendant le défilement (molette, flèches,
  Page préc./suiv., Début/Fin), effacées en 150 ms au bout de 2 s sans défilement, même
  pointeur au-dessus de la liste (opacités `active_*` d'egui pilotées par `fx-ui::wheel`) ;
  survolées ou saisies, elles réapparaissent.
- **Étiquettes** : icône d'étiquette colorée (barre latérale, accueil, menu contextuel),
  activables dans les paramètres, ajout, suppression, couleurs proposées ou libres.
- **Shell** : menu contextuel folder + « Afficher plus d'options », icônes et miniatures via
  `IShellItemImageFactory`. Les options du shell ne sont pas affichées par Windows : le menu
  `IContextMenu` est construit (`QueryContextMenu`, entrées étendues avec Maj) puis lu
  (`GetMenuItemInfoW` : libellés sans `&`, raccourcis, icônes 32 bits, états grisé, coché, par
  défaut, verbes canoniques, sous-menus ; ceux que les extensions ne remplissent qu'à
  l'ouverture, comme Ouvrir avec et Envoyer vers, reçoivent d'abord `WM_INITMENUPOPUP` par
  `HandleMenuMsg`), dessiné par folder (`fx-ui::view::shellmenu`, défilant s'il dépasse la
  fenêtre), et l'entrée choisie exécutée par `InvokeCommand` (« Renommer » : renommage sur
  place ; « Ouvrir » un dossier : navigation dans folder). « Afficher plus d'options » ouvre
  ces entrées dans un menu folder à la place du premier ; Maj + clic droit les ajoute à la
  suite du menu folder, sans celles qu'il propose déjà (par verbe : ouvrir, couper, copier,
  coller, supprimer, renommer, propriétés, copier le chemin, épingler).
- **État de synchronisation cloud** (`fx_win::overlay`) : sur l'icône des fichiers (Détails,
  Liste, Grille, Colonnes), l'icône superposée que le fournisseur fait dessiner par
  l'Explorateur (Google Drive, OneDrive, Dropbox…), obtenue par `IShellIconOverlay` sur le
  dossier parent et la liste d'images système ; celles du shell (raccourci, partage) sont
  écartées. Interrogée seulement sous une racine cloud (racines `SyncRootManager`, dossiers
  OneDrive, volume « Google Drive ») : ailleurs, aucune extension n'est chargée (celle de
  Google Drive ajoute 4 threads au processus, dès qu'on parcourt son lecteur). Interrogation
  sur le thread shell, après les images ; état retenu par entrée affichée (borné à 4 096,
  compté dans F12), revérifié toutes les 3 s tant que l'entrée visible en porte un.
- **Glisser-déposer** (`fx-ui::view::dnd`) : dans la fenêtre, glisser géré par folder vers
  un dossier de la liste, le fond d'un volet, le fil d'Ariane, un onglet, la barre latérale
  ou la Corbeille (cible surlignée, aperçu « Déplacer vers… / Copier vers… » sous le
  pointeur ; Ctrl copie, Maj déplace, sinon selon le volume ; Échap abandonne). Le
  glisser-déposer OLE (`SHDoDragDrop`) ne prend le relais qu'à la sortie de la fenêtre :
  sa boucle modale fige l'interface et consomme le relâchement du bouton, qui est donc
  renvoyé à la fenêtre ensuite (sinon plus aucun survol jusqu'au clic suivant). Dépôts venus
  de l'Explorateur ou d'une autre fenêtre folder : mêmes cibles, suivies par la position du
  curseur lue à Windows (aucun message de souris pendant un glisser OLE).
- **Recherche** : filtre instantané et recherche récursive diffusée dans un dossier virtuel.
- **Commandes** : 73 commandes avec raccourcis, palette (`Ctrl+Maj+P`), menus de la barre
  d'outils ; réglages (thème, affichage, comportement, raccourcis) ; étiquettes colorées ;
  diagnostics (F12 : temps d'image, mémoire, caches).
- **Fenêtre** : barre de titre personnalisée (onglets), cadre DWM (coins arrondis, ombre),
  redimensionnement par les bords, thème clair/sombre suivant Windows, couleur d'accent.

### 11.2 Écarts assumés par rapport aux sections précédentes

- **Distribution** : installeur Inno Setup (`installer/folder.iss`) plutôt que MSIX : un seul
  exécutable, installation par utilisateur sans droits administrateur (« pour tous les
  utilisateurs » en option), menu Démarrer, raccourci Bureau facultatif, fermeture de folder
  avant mise à jour, réglages conservés à la désinstallation. Runtime C lié statiquement
  (`.cargo/config.toml`, `+crt-static`) : aucun redistribuable Visual C++ à installer.
  Construit par `.github/workflows/installer.yml` : la version de `[workspace.package]`
  (Cargo.toml) fait foi ; un push sur `main` qui la change (ou un lancement à la demande)
  crée la release `v<version>` avec l'installeur si elle n'existe pas encore (préversion si
  la version a un suffixe). Signature de code par SignPath Foundation (gratuite pour les
  projets open source : dépôt public, licences MIT / Apache-2.0, politique de signature dans
  le README) : `folder.exe` et le désinstalleur sont signés avant d'entrer dans
  l'installeur, puis l'installeur ; deux demandes de signature par release, approuvées à la
  main dans SignPath, chacune avec la version contrôlée (nom et version du produit,
  `.signpath/artifact-configuration/`). Désinstalleur : méthode d'Inno Setup sans SignTool
  (`SignedUninstaller` + `SignedUninstallerDir`, définis par `/DSignUninstaller`) — une
  première compilation dépose l'exemplaire à signer, la seconde en intègre la signature.
  Étapes actives dès que la variable de dépôt `SIGNPATH_ORGANIZATION_ID` existe (secret
  `SIGNPATH_API_TOKEN`), et seulement si SignPath Foundation accepte le projet (dépôt
  public) ; sans elles, installeur non signé (avertissement SmartScreen, et blocage par
  Smart App Control).

- **Commandes** (§7.6) : `fx-commands` ne contient que les métadonnées (macro `commands!`
  dans `lib.rs` : identifiant, catégorie, libellé, glyphe, raccourcis) ; l'exécution vit
  dans `fx-app::update::commands::run`, qui retourne des `Effect`. Pas de trait `Action`.
- **Providers** (§7.5) : pas de crate `fx-providers` ; `Location` (`Home`, `Dir`, `Search`,
  `Tag`, `RecycleBin`, `Network`, dans `fx-app::history`) est traité par `fx-app`. La
  corbeille et le réseau sont des listes virtuelles : l'élément réel (`$R…`) ou le nom
  d'analyse shell (appareil) est retenu à part, par identifiant d'entrée. Le trait viendra
  avec le premier provider d'archive ou FTP.
- **Fenêtre** : créée cachée puis affichée par `SetWindowPlacement` ; winit est ensuite
  informé de l'état réel (`Visible`, `Maximized`), faute de quoi il recachait la fenêtre au
  premier clic sur Réduire ou Restaurer.
- **Fenêtre Paramètres** : processus `folder.exe --settings` (même raison que Propriétés),
  qui envoie chaque modification à la fenêtre principale par `WM_COPYDATA`
  (`fx_win::ipc`, format `fx_store::remote`) : la fenêtre principale reste la seule à écrire
  les réglages et la base, et applique tout à chaud ; réouverte au premier plan si déjà là,
  fermée avec la fenêtre principale (un thread attend la fin de son processus).
- **Onglets entre fenêtres** (`fx-ui::view::peer`) : chaque fenêtre est un processus et
  celle d'où part le glisser garde la souris (capture). Elle repère l'autre fenêtre sous le
  curseur (plus haute fenêtre visible qui couvre le point dans l'ordre d'`EnumWindows`, la
  sienne exclue puisqu'elle peut suivre le pointeur ; même titre et même classe) et lui
  envoie par `WM_COPYDATA`
  (`fx_store::remote` : `TabHover`, `TabHoverEnd`, `TabDrop`) la position du pointeur au
  survol de sa barre (au moins toutes les 150 ms : l'aperçu expire après 400 ms sans
  nouvelles), puis l'onglet au relâchement (emplacement et historique du volet). La cible
  montre l'onglet exactement comme un des siens qu'on glisse (sous le pointeur au point de
  saisie transmis, les autres s'écartant selon la même règle), puis l'insère à cette place,
  l'active et le fait glisser depuis l'endroit du dépôt (sans animation d'apparition). La
  fenêtre qui suit le pointeur (onglet unique) est déplacée par `SetWindowPos` à chaque
  image, demandée en continu le temps du glisser (le pointeur restant immobile dans la
  fenêtre, winit n'envoie plus d'événement), et masquée par DWM (`DWMWA_CLOAK` : toujours
  visible pour Windows, capture et contenu conservés) au-dessus de la barre de la cible.
  `fx_win::ipc::send` attend au plus une seconde (`SendMessageTimeoutW`, abandon si la
  fenêtre est figée).
- **Fenêtre Propriétés** : second processus plutôt que second viewport (le moteur CPU vendu
  ne gère qu'une fenêtre) ; `fx-ui` dépend donc aussi de `fx-fs` (calcul de taille) pour
  cette mini-application, qui ne passe pas par `fx-app`.
- **Thread UI** (§6, CLAUDE.md) : trois appels Win32 restent sur le thread UI parce qu'ils
  l'exigent — options du menu contextuel shell (`IContextMenu` construit, lu et exécuté sur
  ce thread : les extensions comptent sur la boucle de messages de la fenêtre propriétaire ;
  environ 1 s au premier menu, qui charge les extensions, 0,1 s ensuite), glisser vers l'extérieur
  (`SHDoDragDrop`, boucle modale OLE) et attributs DWM du cadre. `fx-ui` dépend donc de
  `fx-win` pour ces trois appels, et pour la position du curseur pendant un glisser OLE
  (`GetCursorPos`, non bloquant) ainsi que pour les onglets glissés d'une fenêtre à l'autre
  (fenêtre sous le curseur, `WM_COPYDATA` borné à une seconde, déplacement et masquage de
  la fenêtre).
- **Threads** (§6.2) : 12 au total — UI, pool I/O (2), worker shell STA (icônes, miniatures
  en LIFO, actions shell), watcher IOCP, threads du moteur et du système. Les opérations
  fichiers, la recherche et les boîtes modales natives utilisent un thread temporaire qui
  se termine avec elles (l'analyse de l'espace : un coordinateur et au plus 4 lecteurs,
  §7.13) ; la persistance passe par le pool I/O (pas de thread dédié).
  Les extensions shell tierces chargées à la demande (menu contextuel, état de
  synchronisation sur un lecteur cloud) peuvent créer leurs propres threads. La mise à jour
  (§7.12) ajoute un thread temporaire au démarrage, terminé en quelques millisecondes quand
  la vérification du jour est déjà faite, sinon après celle-ci (5 s puis le temps de la
  requête, dans un autre processus) ; aucun en mode mesure.
- **Conflits** (§7.3) : boîtes natives d'`IFileOperation` (pas encore notre dialogue).
- **Étiquettes** (§7.8) : clé = chemin (pas encore `FileId` + numéro de volume) ; un
  renommage hors de folder perd l'étiquette.
- **Icônes** (§7.4) : `IShellItemImageFactory` pour icônes et miniatures (pas de
  `SHGetImageList`) ; cache de textures par extension, et par chemin pour `.exe`, `.lnk`,
  `.ico`, `.url`.
- **Textes** : en français, en dur dans `fx-commands` et `fx-ui` ; la traduction (§9,
  phase 3) les déplacera vers des fichiers `.ftl`.
- **Moteur CPU vendu** : en plus du portage, déplacement et redimensionnement de la fenêtre
  sans décorations, IME, option `set_clear_each_frame`, rendu partiel avec défilement par
  recopie (`render_damaged`, `set_partial_redraw` ; cf. §3.4 et `VENDOR.md`), images
  différées réellement différées et cadence sur la fréquence de l'écran (`set_frame_pacing`).
  Sans ces deux corrections, une animation de survol de 80 ms produisait des centaines
  d'images et un délai (`request_repaint_after`) faisait tourner la boucle à vide : sur un
  même scénario (3 s d'inactivité, deux clics, renommage), 1 355 images avant, 51 après.
  `set_clipboard_shortcuts(false)` (fenêtre principale) : Ctrl+C/X/V arrivent comme des
  touches et commandent copier, couper, coller des fichiers ; egui-winit les changeait en
  événements de texte, et Ctrl+V ne produisait rien avec seulement des fichiers dans le
  presse-papiers. Les champs de texte retrouvent ces événements (`keys::text_clipboard`).
- **Icônes de dossiers** : barre latérale, page d'accueil, fil d'Ariane et onglets affichent
  l'icône que Windows définit pour le dossier (`ImageKey::Path`) ; dans les vues, les
  dossiers marqués lecture seule ou système (dossiers personnalisés, `desktop.ini`) aussi.
- **Fermeture** : le moteur CPU vendu appelle `App::on_exit` aussi pour
  `ViewportCommand::Close` (bouton ✕ de la barre de titre maison) ; auparavant, seule la
  sauvegarde périodique (30 s) enregistrait la session.
- **Messages de la vue** : une image qui émet des messages en demande une autre, pour
  montrer l'état qui en résulte sans attendre un événement d'entrée.
- **Nom** : l'application s'appelle « folder » (exécutable `folder.exe`, données dans
  `%APPDATA%\folder` et `%LOCALAPPDATA%\folder`, reprises automatiquement de l'ancien nom
  « fx ») ; les crates internes gardent le préfixe `fx-`.
- **Clavier** : les raccourcis à chiffres (`Ctrl+Maj+1…4`) utilisent la touche physique,
  pour fonctionner aussi en AZERTY.

### 11.3 Mesures de l'application complète

`memprobe`, build release, rendu CPU partiel (défaut), médiane de 3 exécutions (défilement :
4 exécutions séparées, alternées avec le rendu complet, cf. §3.4) :

| Mesure | Valeur | Budget |
|---|---|---|
| Mémoire privée au repos, 1k entrées | 15,6 Mo | ≤ 30 Mo ✓ |
| Mémoire privée, 100k entrées | 23,3–23,5 Mo | ≤ 60 Mo ✓ |
| Threads | 12 | ≤ 12 ✓ |
| Première image | 29–32 ms | ≤ 150 ms ✓ |
| Premier lot à chaud | 2,4–4,1 ms | ≤ 16 ms ✓ |
| Énumération 100k | 46–63 ms | ≤ 300 ms ✓ |
| Défilement 1920×1080, p95 | 2,7–3,1 ms (31–32 Mo) | ≤ 8 ms ✓ |
| Défilement 2560×1440, p95 | 3,9–4,4 ms (38–39 Mo) | ≤ 8 ms ✓ |
| Défilement 3840×2160, p95 (hors écran, `renderbench`) | 3,8–6,9 ms | ≤ 8 ms ✓ |
| Exécutable (0.6.0, avec la mise à jour : +0,15 Mo) | 13,0 Mo | ≤ 15 Mo ✓ |

Tests : `cargo test --workspace` (140 tests, dont la comparaison rendu partiel / rendu
complet) ; `clippy -D warnings` propre, y compris avec la feature `wgpu`.

### 11.4 Reste à faire

- Rendu CPU : confirmer le défilement 4K en fenêtre sur un écran 4K (hors écran : p95
  ≤ 6,9 ms). Vérifier à l'œil la partie d'une fenêtre sortie de l'écran puis ramenée : des
  captures `PrintWindow` y ont montré un contenu périmé avec le moteur CPU, en rendu complet
  comme partiel (glow non concerné) ; une présentation complète suit désormais chaque
  déplacement de la fenêtre.
- AccessKit dans le moteur CPU (accessibilité non fonctionnelle avec le moteur par défaut).
- Aperçus Office/PDF via `IPreviewHandler` ; propriétés étendues (dimensions, durée).
- Dialogue de conflits maison ; groupement ; vue Cartes ; barre d'outils personnalisable.
- Étiquettes par `FileId` ; « Ce PC » comme emplacement interne ; surveillance de la
  corbeille (aujourd'hui rechargée après chaque opération de folder, pas lors d'une
  suppression faite ailleurs) ; case « tout sélectionner » dans la vue Colonnes.
- Archives 7z/RAR et parcours d'archive comme dossier ; Windows Search ; Git (statut,
  opérations) ; FTP ; multi-fenêtres dans un même processus ; traduction ; Mica.
