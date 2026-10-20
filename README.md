<p align="center">
  <img src="app/folder/icon/folder.svg" width="128" alt="Icône de folder">
</p>

<h1 align="center">folder</h1>

<p align="center">
  Gestionnaire de fichiers pour Windows, écrit en Rust : <b>rapide</b>, <b>fluide</b> et <b>sobre</b>.
</p>

---

folder reprend les fonctions d'un gestionnaire de fichiers moderne comme
[Files](https://files.community/) : onglets, double volet, aperçus, étiquettes, palette de
commandes. Il s'intègre à l'Explorateur Windows et tient dans un seul exécutable natif, sans
WebView ni runtime .NET.

## Fonctionnalités

- **Navigation** : onglets, historique conservé entre deux sessions, barre de chemin
  cliquable et modifiable, page d'accueil (accès rapide, lecteurs, étiquettes, fichiers
  récents) et barre latérale (lecteurs, réseau, distributions WSL, corbeille). La session est
  restaurée au démarrage.
- **Vues** : Détails, Liste, Grille avec miniatures et Colonnes. Le tri est naturel, comme
  celui de l'Explorateur, et la vue est mémorisée pour chaque dossier. On peut ouvrir un
  double volet et un volet d'aperçu (texte et code colorés, Markdown, images).
- **Opérations** : copier, couper, coller, déplacer vers la corbeille, renommer (aussi en
  masse), annuler et rétablir, compresser ou extraire une archive zip. Elles passent par
  `IFileOperation`, avec une progression et une annulation possibles.
- **Intégration Windows** :
  - le menu contextuel du shell, avec « Afficher plus d'options » ;
  - le glisser-déposer avec l'Explorateur ;
  - une fenêtre Propriétés (Général, Partage, Sécurité, Détails) ;
  - la corbeille et les partages réseau.
  - Photos parcourt les images dans l'ordre de tri de folder.
- **Recherche** : un filtre instantané et une recherche récursive diffusée au fil de l'eau.
- **Commandes** : 64 commandes avec raccourcis configurables, une palette (`Ctrl+Maj+P`) et
  une barre d'outils qui s'adapte à la largeur de la fenêtre.
- **Apparence** : thème clair ou sombre suivant Windows, couleur d'accent libre, animations
  courtes, cadre natif (coins arrondis, ombre).

## Performances

Valeurs mesurées par l'outil `memprobe` sur un build release, avec le rendu CPU (le moteur par défaut) :

| Mesure | Valeur | Budget |
|---|---|---|
| Mémoire privée au repos | ~16 Mo | ≤ 30 Mo |
| Mémoire privée, dossier de 100 000 fichiers | ~23 Mo | ≤ 60 Mo |
| Threads | 12 | ≤ 12 |
| Première image | ~30 ms | ≤ 150 ms |
| Défilement 1080p / 1440p, p95 | ~3 ms / ~4 ms | ≤ 8 ms |

L'énumération envoie les fichiers par lots : l'affichage commence dès le premier, et rien
ne bloque l'interface, même sur un dossier de 100 000 fichiers.

## Installation

Télécharge `folder-setup-<version>-x64.exe` depuis la page
[Releases](../../releases) et lance-le.

- Windows 10 (1809) ou plus récent, en x64.
- L'installation se fait pour l'utilisateur courant, sans droits administrateur.
  L'assistant propose aussi une installation pour tous les utilisateurs.
- L'exécutable n'est pas encore signé : au premier lancement, Windows SmartScreen affiche un
  avertissement (« Informations complémentaires », puis « Exécuter quand même »).
- La désinstallation conserve les réglages (`%APPDATA%\folder`, `%LOCALAPPDATA%\folder`).

## Compiler

Prérequis : Windows, [Rust](https://rustup.rs/) 1.95 ou plus récent (toolchain MSVC) et les
outils de compilation C++ de Visual Studio.

```bash
cargo build --release
```

L'exécutable se trouve dans `target\release\folder.exe`. Le runtime C y est lié
statiquement : il ne dépend d'aucun redistribuable Visual C++.

```bash
cargo run --release -p folder
```

Avant chaque contribution, ces commandes doivent passer :

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

Les outils de mesure (`memprobe`, `renderbench`, `gen-tree`) et la règle de chaque crate
sont décrits dans [CLAUDE.md](CLAUDE.md).

## Publier une version

La version de folder est celle de `[workspace.package]` dans [Cargo.toml](Cargo.toml).

1. Change la version, par exemple `version = "0.3.0"`.
2. Pousse le commit sur `main`.

Le workflow [Installeur](.github/workflows/installer.yml) construit alors l'installeur et
crée la release `v0.3.0`, avec le tag sur ce commit et les notes générées à partir des
changements. Une version avec suffixe, comme `0.3.0-beta.1`, est publiée en préversion.
Si la release existe déjà, rien n'est publié. Lancé à la main depuis l'onglet Actions, le
workflow crée la release qui manque, ou construit seulement l'installeur.

Pour construire l'installeur en local, installe [Inno Setup 6](https://jrsoftware.org/isinfo.php),
puis lance :

```bash
iscc /DAppVersion=0.2.0 installer/folder.iss
```

L'installeur est écrit dans `target\installer`.

## Architecture

| Crate | Rôle |
|---|---|
| `fx-core` | Types de domaine : instantané de dossier compact, tri, sélection, formats |
| `fx-win` | Couche sûre au-dessus de Win32 et COM (seul crate avec du code `unsafe`) |
| `fx-fs` | Énumération en flux, surveillance des dossiers, recherche, zip |
| `fx-shell` | Worker COM : icônes, miniatures, actions du shell |
| `fx-commands` | Commandes et raccourcis |
| `fx-store` | Réglages, session, étiquettes (TOML et SQLite) |
| `fx-preview` | Aperçus de texte, de code et de Markdown |
| `fx-app` | État de l'application, messages, effets, ordonnanceur |
| `fx-ui` | Interface egui, thème, fenêtres Paramètres et Propriétés |
| `app/folder` | Exécutable, manifeste, icône |

L'interface utilise [egui](https://github.com/emilk/egui) avec un moteur de rendu CPU
(`vendor/egui_software_backend`) qui ne redessine que les zones modifiées. Un moteur
OpenGL est disponible avec `--renderer glow`. Les choix, les budgets et l'état de
l'implémentation sont détaillés dans [ARCHITECTURE.md](ARCHITECTURE.md).

## Licence

MIT ou Apache-2.0, au choix (voir `license` dans [Cargo.toml](Cargo.toml)).
