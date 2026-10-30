//! Suggestions de nettoyage tirées d'une analyse de l'espace : emplacements connus pour
//! être régénérables (temporaires, caches, mises à jour, rapports d'erreur) et éléments
//! volumineux repérés dans l'arbre (dépendances `node_modules`, dossiers `CACHEDIR.TAG`,
//! gros fichiers anciens, gros téléchargements).
//!
//! Rien n'est supprimé ici : chaque suggestion dit quoi supprimer et comment ; l'UI
//! demande confirmation et passe par `IFileOperation`.

use std::path::{Path, PathBuf};

use fx_core::disktree::{DiskTree, NodeKind, flags};
use fx_core::wtf8;

/// Une suggestion n'est faite qu'à partir de cette taille.
const MIN_BYTES: u64 = 10 << 20;
/// Dépendances et sorties de compilation proposées à partir de cette taille.
const MIN_BUILD_DIR: u64 = 50 << 20;
const OLD_FILE_MIN: u64 = 1 << 30;
const DOWNLOAD_MIN: u64 = 100 << 20;
/// Un an, en intervalles de 100 ns.
const ONE_YEAR: i64 = 365 * 24 * 3600 * 10_000_000;
/// Chemins listés au plus par suggestion (les plus gros).
const MAX_PATHS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanKind {
    RecycleBin,
    Temp,
    WindowsUpdate,
    WindowsOld,
    CrashDumps,
    BrowserCache,
    PackageCache,
    NodeModules,
    BuildOutput,
    OldLargeFiles,
    LargeDownloads,
}

/// Ce que « Nettoyer » supprime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanMode {
    /// Le contenu des dossiers listés (les dossiers eux-mêmes restent).
    Contents,
    /// Les éléments listés.
    Items,
    /// Vider la corbeille de Windows.
    RecycleBin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub kind: CleanKind,
    pub title: &'static str,
    pub detail: &'static str,
    /// Du plus gros au plus petit.
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    pub mode: CleanMode,
    /// Supprimé définitivement (contenu régénérable : le mettre à la corbeille ne libérerait
    /// rien) ; sinon mis à la corbeille.
    pub permanent: bool,
}

/// Emplacements de l'utilisateur et du système (variables d'environnement, dossiers connus).
#[derive(Debug, Clone, Default)]
pub struct Places {
    pub temp: Option<PathBuf>,
    pub windows: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub profile: Option<PathBuf>,
    pub downloads: Option<PathBuf>,
    /// FILETIME de l'analyse (âge des fichiers).
    pub now: i64,
}

impl Places {
    pub fn current() -> Self {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
        Self {
            temp: env("TEMP"),
            windows: env("SystemRoot"),
            local_app_data: env("LOCALAPPDATA"),
            profile: env("USERPROFILE"),
            downloads: fx_win::sys::known_folder(fx_win::sys::KnownFolder::Downloads),
            now: fx_win::time::now_filetime(),
        }
    }
}

/// Suggestions pour `tree`, de la plus grosse à la plus petite.
pub fn suggest(tree: &DiskTree, places: &Places) -> Vec<Suggestion> {
    let mut out = Vec::new();
    let mut claimed: Vec<u32> = Vec::new();
    // Racine du lecteur analysé (`C:\`), suffixe ajouté à la main : `join("\\")`
    // remplacerait le chemin.
    let drive = tree.root_path().components().next().map(|c| {
        let mut d = c.as_os_str().to_os_string();
        d.push("\\");
        PathBuf::from(d)
    });
    if let Some(d) = &drive
        && let Some(bin) = tree.find(&d.join("$Recycle.Bin"))
    {
        claimed.push(bin);
        push(
            &mut out,
            tree,
            CleanKind::RecycleBin,
            "Corbeille",
            "Éléments supprimés, encore sur le disque tant que la corbeille n'est pas vidée.",
            CleanMode::RecycleBin,
            true,
            vec![bin],
        );
    }
    let mut known = |out: &mut Vec<Suggestion>,
                     kind: CleanKind,
                     title: &'static str,
                     detail: &'static str,
                     mode: CleanMode,
                     paths: Vec<PathBuf>| {
        let nodes: Vec<u32> = paths.iter().filter_map(|p| tree.find(p)).collect();
        claimed.extend(&nodes);
        push(out, tree, kind, title, detail, mode, true, nodes);
    };
    let win = places.windows.as_deref();
    let lad = places.local_app_data.as_deref();
    let prof = places.profile.as_deref();
    let under = |base: Option<&Path>, rel: &[&str]| -> Vec<PathBuf> {
        base.map(|b| vec![rel.iter().fold(b.to_path_buf(), |p, r| p.join(r))])
            .unwrap_or_default()
    };
    let mut temp: Vec<PathBuf> = places.temp.iter().cloned().collect();
    temp.extend(under(win, &["Temp"]));
    known(
        &mut out,
        CleanKind::Temp,
        "Fichiers temporaires",
        "Laissés par les applications et les installations. Les fichiers en cours \
         d'utilisation sont ignorés.",
        CleanMode::Contents,
        temp,
    );
    known(
        &mut out,
        CleanKind::WindowsUpdate,
        "Téléchargements de Windows Update",
        "Mises à jour déjà installées ; Windows les retélécharge au besoin.",
        CleanMode::Contents,
        under(win, &["SoftwareDistribution", "Download"]),
    );
    if let Some(d) = &drive {
        known(
            &mut out,
            CleanKind::WindowsOld,
            "Installation précédente de Windows",
            "Windows.old permet de revenir à la version précédente ; le supprimer y renonce. \
             Demande les droits d'administrateur.",
            CleanMode::Items,
            vec![d.join("Windows.old")],
        );
    }
    let mut dumps = under(lad, &["CrashDumps"]);
    dumps.extend(under(win, &["Minidump"]));
    dumps.extend(under(win, &["LiveKernelReports"]));
    known(
        &mut out,
        CleanKind::CrashDumps,
        "Rapports d'erreur",
        "Vidages mémoire écrits lors de plantages, utiles seulement pour un diagnostic.",
        CleanMode::Contents,
        dumps,
    );
    known(
        &mut out,
        CleanKind::BrowserCache,
        "Caches des navigateurs",
        "Pages et images gardées pour aller plus vite ; recréés au fil de la navigation. \
         Fermez le navigateur avant de nettoyer.",
        CleanMode::Contents,
        browser_caches(tree, lad),
    );
    let mut pkgs = Vec::new();
    for rel in [
        &["npm-cache"][..],
        &["pip", "cache"],
        &["Yarn", "Cache"],
        &["NuGet", "v3-cache"],
        &["pnpm", "store"],
    ] {
        pkgs.extend(under(lad, rel));
    }
    pkgs.extend(under(prof, &[".cargo", "registry", "cache"]));
    pkgs.extend(under(prof, &[".gradle", "caches"]));
    known(
        &mut out,
        CleanKind::PackageCache,
        "Caches de paquets",
        "Paquets téléchargés par npm, pip, Yarn, NuGet, Cargo ou Gradle ; retéléchargés au \
         besoin.",
        CleanMode::Contents,
        pkgs,
    );

    // Parcours de l'arbre : dépendances, sorties de compilation, gros fichiers.
    let mut node_modules = Vec::new();
    let mut build = Vec::new();
    let mut old = Vec::new();
    let mut downloads = Vec::new();
    let skip: Vec<u32> = [
        win.and_then(|w| tree.find(w)),
        drive
            .as_ref()
            .and_then(|d| tree.find(&d.join("Program Files"))),
        drive
            .as_ref()
            .and_then(|d| tree.find(&d.join("Program Files (x86)"))),
        drive
            .as_ref()
            .and_then(|d| tree.find(&d.join("ProgramData"))),
    ]
    .into_iter()
    .flatten()
    .chain(claimed.iter().copied())
    .collect();
    let dl = places.downloads.as_deref().and_then(|d| tree.find(d));
    // (nœud, sous le dossier Téléchargements)
    let mut stack = vec![(DiskTree::root(), dl == Some(DiskTree::root()))];
    while let Some((n, in_dl)) = stack.pop() {
        for c in tree.children(n) {
            match tree.kind(c) {
                NodeKind::Dir => {
                    if skip.contains(&c) {
                        continue;
                    }
                    if tree.name(c).eq_ignore_ascii_case(b"node_modules") {
                        if tree.size(c) >= MIN_BUILD_DIR {
                            node_modules.push(c);
                        }
                        continue;
                    }
                    if tree.flags(c) & flags::CACHE_TAG != 0 {
                        if tree.size(c) >= MIN_BUILD_DIR {
                            build.push(c);
                        }
                        continue;
                    }
                    // Rien à trouver dans un dossier plus petit que le plus petit seuil.
                    if tree.size(c) >= MIN_BUILD_DIR {
                        stack.push((c, in_dl || dl == Some(c)));
                    }
                }
                NodeKind::File => {
                    let size = tree.size(c);
                    if in_dl && size >= DOWNLOAD_MIN {
                        downloads.push(c);
                    } else if size >= OLD_FILE_MIN
                        && places.now - tree.modified(c) >= ONE_YEAR
                        && !(n == DiskTree::root() && is_system_file(tree.name(c)))
                    {
                        old.push(c);
                    }
                }
                NodeKind::SmallFiles => {}
            }
        }
    }
    push(
        &mut out,
        tree,
        CleanKind::NodeModules,
        "Dépendances node_modules",
        "Réinstallées par npm install dans chaque projet. Supprimées définitivement.",
        CleanMode::Items,
        true,
        node_modules,
    );
    push(
        &mut out,
        tree,
        CleanKind::BuildOutput,
        "Sorties de compilation et caches",
        "Dossiers marqués CACHEDIR.TAG (cible Cargo, caches d'outils) : régénérés à la \
         prochaine compilation. Supprimés définitivement.",
        CleanMode::Items,
        true,
        build,
    );
    push(
        &mut out,
        tree,
        CleanKind::LargeDownloads,
        "Gros téléchargements",
        "Fichiers de plus de 100 Mo dans Téléchargements (installeurs, images disque, \
         archives). Vérifiez avant de les mettre à la corbeille.",
        CleanMode::Items,
        false,
        downloads,
    );
    push(
        &mut out,
        tree,
        CleanKind::OldLargeFiles,
        "Gros fichiers anciens",
        "Fichiers de plus de 1 Go non modifiés depuis un an. Vérifiez avant de les mettre à \
         la corbeille.",
        CleanMode::Items,
        false,
        old,
    );
    out.sort_by_key(|s| std::cmp::Reverse(s.bytes));
    out
}

#[allow(clippy::too_many_arguments)]
fn push(
    out: &mut Vec<Suggestion>,
    tree: &DiskTree,
    kind: CleanKind,
    title: &'static str,
    detail: &'static str,
    mode: CleanMode,
    permanent: bool,
    mut nodes: Vec<u32>,
) {
    nodes.sort_by_key(|&n| std::cmp::Reverse(tree.size(n)));
    nodes.dedup();
    nodes.truncate(MAX_PATHS);
    let bytes = nodes.iter().map(|&n| tree.size(n)).sum::<u64>();
    if bytes < MIN_BYTES {
        return;
    }
    out.push(Suggestion {
        kind,
        title,
        detail,
        paths: nodes.iter().map(|&n| tree.path(n)).collect(),
        bytes,
        mode,
        permanent,
    });
}

/// `Cache`, `Code Cache` et `GPUCache` des profils Chrome, Edge et Brave ; `cache2` des
/// profils Firefox (sous `%LOCALAPPDATA%`).
fn browser_caches(tree: &DiskTree, lad: Option<&Path>) -> Vec<PathBuf> {
    let Some(lad) = lad else { return Vec::new() };
    let mut out = Vec::new();
    for base in [
        lad.join(r"Google\Chrome\User Data"),
        lad.join(r"Microsoft\Edge\User Data"),
        lad.join(r"BraveSoftware\Brave-Browser\User Data"),
    ] {
        let Some(n) = tree.find(&base) else { continue };
        for profile in tree.children(n).filter(|&c| tree.kind(c) == NodeKind::Dir) {
            for name in ["Cache", "Code Cache", "GPUCache"] {
                if let Some(c) = tree.child_by_name(profile, name.as_bytes()) {
                    out.push(tree.path(c));
                }
            }
        }
    }
    if let Some(n) = tree.find(&lad.join(r"Mozilla\Firefox\Profiles")) {
        for profile in tree.children(n).filter(|&c| tree.kind(c) == NodeKind::Dir) {
            if let Some(c) = tree.child_by_name(profile, b"cache2") {
                out.push(tree.path(c));
            }
        }
    }
    out
}

/// Fichiers de Windows à la racine du lecteur (pagination, veille prolongée).
fn is_system_file(name: &[u8]) -> bool {
    let n = wtf8::display(name).to_ascii_lowercase();
    matches!(
        n.as_str(),
        "pagefile.sys" | "hiberfil.sys" | "swapfile.sys" | "dumpstack.log" | "dumpstack.log.tmp"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_core::CancelToken;
    use std::time::Duration;

    #[test]
    fn finds_build_dirs_known_caches_and_large_downloads() {
        let root = std::env::temp_dir().join(format!("fx-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let big = |p: &Path, n: usize| {
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(p, vec![1u8; n]).expect("w");
        };
        big(&root.join(r"proj\node_modules\lib\a.js"), 60 << 20);
        big(&root.join(r"rust\target\debug\x.rlib"), 55 << 20);
        std::fs::write(root.join(r"rust\target\CACHEDIR.TAG"), b"Signature").expect("w");
        big(&root.join(r"tmp\setup.tmp"), 12 << 20);
        big(&root.join(r"dl\image.iso"), 110 << 20);
        let tree = crate::diskscan::scan(
            &root,
            64 << 10,
            2,
            &CancelToken::new(),
            Duration::MAX,
            |_| {},
        )
        .expect("scan");
        let places = Places {
            temp: Some(root.join("tmp")),
            downloads: Some(root.join("dl")),
            now: fx_win::time::now_filetime(),
            ..Places::default()
        };
        let s = suggest(&tree, &places);
        let kinds: Vec<CleanKind> = s.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            vec![
                CleanKind::LargeDownloads,
                CleanKind::NodeModules,
                CleanKind::BuildOutput,
                CleanKind::Temp
            ]
        );
        let nm = &s[1];
        assert_eq!(nm.paths, vec![root.join(r"proj\node_modules")]);
        assert!(nm.permanent && nm.mode == CleanMode::Items);
        assert_eq!(s[3].mode, CleanMode::Contents);
        assert!(!s[0].permanent);
        let _ = std::fs::remove_dir_all(&root);
    }
}
