//! Texte validé dans la barre d'adresse : emplacement à ouvrir ou programme à lancer, comme
//! dans l'Explorateur (`cmd`, `code .`, `notepad notes.txt`, `..`).
//!
//! La résolution touche le disque (existence des chemins, recherche dans le `PATH`) : elle
//! s'exécute sur le thread shell (`Effect::RunAddress`), jamais dans `update`. La logique est
//! séparée des appels système par [`Probe`] pour être testée sans fichiers.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Nature d'un chemin existant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
}

/// Accès au système de fichiers nécessaire à la résolution.
pub trait Probe {
    fn kind(&self, path: &Path) -> Option<Kind>;
}

/// Sonde réelle (thread de travail uniquement).
pub struct FsProbe;

impl Probe for FsProbe {
    fn kind(&self, path: &Path) -> Option<Kind> {
        match std::fs::metadata(path) {
            Ok(m) if m.is_dir() => Some(Kind::Dir),
            Ok(_) => Some(Kind::File),
            // Alias d'exécution d'application (`wt.exe`, `winget.exe` dans WindowsApps) : point
            // d'analyse que `metadata` ne sait pas suivre, mais bien un programme.
            Err(_) => std::fs::symlink_metadata(path)
                .ok()
                .map(|m| if m.is_dir() { Kind::Dir } else { Kind::File }),
        }
    }
}

/// Contexte de la saisie : dossier courant du volet et variables de recherche.
pub struct Env {
    /// Dossier affiché (ou le profil utilisateur hors d'un dossier) : base des chemins
    /// relatifs et dossier de travail du programme lancé.
    pub cwd: Option<PathBuf>,
    /// Dossiers du `PATH`, dans l'ordre.
    pub path: Vec<PathBuf>,
    /// Extensions de `PATHEXT` (`.COM`, `.EXE`, `.BAT`…), dans l'ordre.
    pub exts: Vec<String>,
}

impl Env {
    /// Variables du processus, lues sur le thread de travail.
    pub fn from_process(cwd: Option<PathBuf>) -> Self {
        let cwd = cwd.or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from));
        let path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        let exts = std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| e.starts_with('.') && e.len() > 1)
            .map(str::to_owned)
            .collect();
        Self { cwd, path, exts }
    }
}

/// Ce que désigne la saisie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Dossier existant : on y navigue.
    Dir(PathBuf),
    /// Fichier existant : ouvert avec son application.
    File(PathBuf),
    /// Programme à lancer (chemin trouvé, ou nom laissé au shell : App Paths, URL, protocole
    /// `ms-settings:`…) avec ses arguments, dans le dossier courant.
    Run {
        program: OsString,
        args: String,
        /// Script `.cmd`/`.bat` trouvé dans le `PATH` : lanceur d'application (`code.cmd` de
        /// VS Code, `cursor.cmd`…) exécuté sans fenêtre de console (cf. [`run_quiet`]).
        quiet: bool,
    },
}

/// Programme trouvé sur le disque.
struct Found {
    path: PathBuf,
    /// Script trouvé dans un dossier du `PATH` (ni le dossier courant, ni un chemin saisi).
    quiet: bool,
}

impl Found {
    fn run(self, args: &str) -> Target {
        Target::Run {
            program: self.path.into(),
            args: args.to_owned(),
            quiet: self.quiet,
        }
    }
}

/// Au-delà, on ne cherche plus de programme parmi les débuts de saisie (chemin non cité avec
/// espaces) : le premier mot est passé au shell.
const MAX_PREFIXES: usize = 16;

/// Résout la saisie dans l'ordre de l'Explorateur : chemin existant (absolu ou relatif au
/// dossier courant), puis programme suivi de ses arguments.
pub fn resolve(text: &str, env: &Env, fs: &impl Probe) -> Target {
    let t = text.trim();
    // Programme entre guillemets : "C:\Program Files\app.exe" --option
    if let Some(rest) = t.strip_prefix('"')
        && let Some(end) = rest.find('"')
    {
        let program = &rest[..end];
        let args = rest[end + 1..].trim_start();
        if args.is_empty()
            && let Some(target) = existing(program, env, fs)
        {
            return target;
        }
        return find_program(program, env, fs).map_or_else(
            || Target::Run {
                program: program.into(),
                args: args.to_owned(),
                quiet: false,
            },
            |found| found.run(args),
        );
    }
    if let Some(target) = existing(t, env, fs) {
        return target;
    }
    // Plus court début de saisie qui désigne un programme, coupé à un espace (comme
    // CreateProcess pour un chemin non cité : C:\Program Files\Git\bin\bash.exe --login).
    // Un nom seul ne contient pas d'espace : au-delà du premier mot, seulement des chemins.
    let cuts = t
        .char_indices()
        .filter(|&(i, c)| c.is_whitespace() && !t[..i].ends_with(char::is_whitespace))
        .map(|(i, _)| i)
        .chain(std::iter::once(t.len()));
    for (n, cut) in cuts.enumerate() {
        let head = &t[..cut];
        if n >= MAX_PREFIXES || (n > 0 && !is_path_like(head)) {
            break;
        }
        if let Some(found) = find_program(head, env, fs) {
            return found.run(t[cut..].trim_start());
        }
    }
    let (program, args) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    Target::Run {
        program: program.into(),
        args: args.trim_start().to_owned(),
        quiet: false,
    }
}

/// Chemin existant désigné par la saisie entière.
fn existing(t: &str, env: &Env, fs: &impl Probe) -> Option<Target> {
    if t.is_empty() {
        return None;
    }
    let p = normalize(&absolute(Path::new(t), env)?);
    Some(match fs.kind(&p)? {
        Kind::Dir => Target::Dir(p),
        Kind::File => Target::File(p),
    })
}

/// Chemin rapporté au dossier courant s'il est relatif (`src`, `..\x`, `\Windows`).
fn absolute(p: &Path, env: &Env) -> Option<PathBuf> {
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    // `push` d'un chemin commençant par `\` garde seulement le lecteur du dossier courant.
    env.cwd.as_ref().map(|cwd| cwd.join(p))
}

/// Retire `.` et `..` sans toucher au disque (pas de `canonicalize` : il impose la forme
/// `\\?\` et résout les liens, que le fil d'Ariane doit montrer tels quels).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.push(c);
                } else {
                    out.pop();
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn is_path_like(s: &str) -> bool {
    s.contains(['\\', '/', ':'])
}

/// Programme désigné par `name` : chemin (relatif au dossier courant) ou nom cherché dans le
/// dossier courant puis le `PATH`, comme l'invite de commandes. Sans extension, on essaie
/// celles de `PATHEXT` : `code` trouve `code.cmd`, pas le script sans extension voisin.
fn find_program(name: &str, env: &Env, fs: &impl Probe) -> Option<Found> {
    if name.is_empty() || name.ends_with(['\\', '/']) {
        return None;
    }
    let has_ext = Path::new(name).extension().is_some();
    let candidates = |base: PathBuf| {
        let plain = has_ext.then(|| base.clone());
        let with_ext = env.exts.iter().filter(move |_| !has_ext).map(move |e| {
            let mut s = base.clone().into_os_string();
            s.push(e.to_ascii_lowercase());
            PathBuf::from(s)
        });
        plain.into_iter().chain(with_ext)
    };
    let is_file = |p: &PathBuf| fs.kind(p) == Some(Kind::File);
    if is_path_like(name) {
        let base = absolute(Path::new(name), env)?;
        let path = candidates(normalize(&base)).find(is_file)?;
        return Some(Found { path, quiet: false });
    }
    if let Some(path) = env
        .cwd
        .iter()
        .flat_map(|dir| candidates(dir.join(name)))
        .find(is_file)
    {
        return Some(Found { path, quiet: false });
    }
    let path = env
        .path
        .iter()
        .flat_map(|dir| candidates(dir.join(name)))
        .find(is_file)?;
    let quiet = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    Some(Found { path, quiet })
}

/// Exécute un script lanceur sans fenêtre de console. Lancé par ShellExecute, `code.cmd`
/// ouvre une console le temps que VS Code démarre ; sans console (`CREATE_NO_WINDOW`), seule
/// la fenêtre de l'application apparaît. Entrées et sorties nulles : un `pause` ou une
/// question du script reçoit une fin de fichier au lieu d'attendre une réponse invisible.
pub fn run_quiet(script: &Path, args: &str, cwd: Option<&Path>) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let comspec =
        std::env::var_os("ComSpec").unwrap_or_else(|| r"C:\Windows\System32\cmd.exe".into());
    let mut cmd = Command::new(comspec);
    // `/s /c ""script" args"` : cmd retire seulement les guillemets extérieurs, les arguments
    // sont transmis tels que saisis (comme dans une invite de commandes) ; `/d` ignore AutoRun.
    cmd.raw_arg(format!("/d /s /c \"\"{}\" {args}\"", script.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Disque fictif : chemins comparés sans tenir compte de la casse, comme NTFS.
    struct Fake(HashMap<String, Kind>);

    impl Fake {
        fn new(entries: &[(&str, Kind)]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|&(p, k)| (p.to_lowercase(), k))
                    .collect(),
            )
        }
    }

    impl Probe for Fake {
        fn kind(&self, path: &Path) -> Option<Kind> {
            self.0.get(&path.to_string_lossy().to_lowercase()).copied()
        }
    }

    fn env() -> Env {
        Env {
            cwd: Some(r"D:\projets\app".into()),
            path: vec![
                r"C:\Windows\system32".into(),
                r"C:\Users\moi\AppData\Local\Programs\Microsoft VS Code\bin".into(),
            ],
            exts: [".COM", ".EXE", ".BAT", ".CMD"].map(String::from).to_vec(),
        }
    }

    fn disk() -> Fake {
        use Kind::{Dir, File};
        Fake::new(&[
            (r"D:\projets", Dir),
            (r"D:\projets\app", Dir),
            (r"D:\projets\app\src", Dir),
            (r"D:\projets\app\notes.txt", File),
            (r"D:\projets\app\build.bat", File),
            (r"D:\projets\autre", Dir),
            (r"C:\Windows\system32\cmd.exe", File),
            (r"C:\Windows\system32\notepad.exe", File),
            // VS Code installe un script bash sans extension à côté de code.cmd.
            (
                r"C:\Users\moi\AppData\Local\Programs\Microsoft VS Code\bin\code",
                File,
            ),
            (
                r"C:\Users\moi\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd",
                File,
            ),
            (r"C:\Program Files", Dir),
            (r"C:\Program Files\Git\bin\bash.exe", File),
        ])
    }

    fn run(program: &str, args: &str) -> Target {
        Target::Run {
            program: program.into(),
            args: args.into(),
            quiet: false,
        }
    }

    fn quiet(program: &str, args: &str) -> Target {
        Target::Run {
            program: program.into(),
            args: args.into(),
            quiet: true,
        }
    }

    #[test]
    fn bare_command_is_found_in_path() {
        assert_eq!(
            resolve("cmd", &env(), &disk()),
            run(r"C:\Windows\system32\cmd.exe", "")
        );
        assert_eq!(
            resolve("notepad   notes.txt", &env(), &disk()),
            run(r"C:\Windows\system32\notepad.exe", "notes.txt")
        );
    }

    #[test]
    fn pathext_wins_over_extensionless_script() {
        // Script du PATH : lanceur d'application, sans console.
        assert_eq!(
            resolve("code .", &env(), &disk()),
            quiet(
                r"C:\Users\moi\AppData\Local\Programs\Microsoft VS Code\bin\code.cmd",
                "."
            )
        );
    }

    #[test]
    fn current_folder_is_searched_first() {
        // Script du dossier courant ou désigné par son chemin : console visible, comme un
        // double-clic.
        assert_eq!(
            resolve("build release", &env(), &disk()),
            run(r"D:\projets\app\build.bat", "release")
        );
        assert_eq!(
            resolve(r".\build x", &env(), &disk()),
            run(r"D:\projets\app\build.bat", "x")
        );
    }

    #[test]
    fn relative_paths_navigate_or_open() {
        let (e, d) = (env(), disk());
        assert_eq!(
            resolve("src", &e, &d),
            Target::Dir(r"D:\projets\app\src".into())
        );
        assert_eq!(resolve("..", &e, &d), Target::Dir(r"D:\projets".into()));
        assert_eq!(
            resolve(r"..\autre\", &e, &d),
            Target::Dir(r"D:\projets\autre".into())
        );
        assert_eq!(
            resolve("notes.txt", &e, &d),
            Target::File(r"D:\projets\app\notes.txt".into())
        );
    }

    #[test]
    fn absolute_paths_with_spaces() {
        let (e, d) = (env(), disk());
        assert_eq!(
            resolve(r"C:\Program Files", &e, &d),
            Target::Dir(r"C:\Program Files".into())
        );
        assert_eq!(
            resolve(r"C:\Program Files\Git\bin\bash.exe --login -i", &e, &d),
            run(r"C:\Program Files\Git\bin\bash.exe", "--login -i")
        );
        assert_eq!(
            resolve(r#""C:\Program Files\Git\bin\bash" --login"#, &e, &d),
            run(r"C:\Program Files\Git\bin\bash.exe", "--login")
        );
        assert_eq!(
            resolve(r#""C:\Program Files""#, &e, &d),
            Target::Dir(r"C:\Program Files".into())
        );
    }

    #[test]
    fn unknown_names_go_to_the_shell() {
        let (e, d) = (env(), disk());
        // App Paths, URL et protocoles : ShellExecute sait les résoudre.
        assert_eq!(resolve("winword /n", &e, &d), run("winword", "/n"));
        assert_eq!(
            resolve("https://example.com", &e, &d),
            run("https://example.com", "")
        );
        assert_eq!(resolve("ms-settings:", &e, &d), run("ms-settings:", ""));
    }

    #[test]
    fn real_path_finds_cmd() {
        let target = resolve("cmd /k ver", &Env::from_process(None), &FsProbe);
        let Target::Run { program, args, .. } = target else {
            panic!("pas un programme : {target:?}");
        };
        assert!(
            program
                .to_string_lossy()
                .to_lowercase()
                .ends_with(r"\cmd.exe")
        );
        assert_eq!(args, "/k ver");
    }

    #[test]
    fn quiet_script_gets_its_arguments_and_working_folder() {
        // Espace dans le chemin, comme « Microsoft VS Code\bin\code.cmd ».
        let base = std::env::temp_dir().join(format!("fx-quiet {}", std::process::id()));
        let cwd = base.join("dossier courant");
        std::fs::create_dir_all(&cwd).expect("dossier");
        let script = base.join("lanceur.cmd");
        std::fs::write(
            &script,
            "@echo %* > \"%~dp0sortie.txt\"\r\n@cd >> \"%~dp0sortie.txt\"\r\n",
        )
        .expect("script");
        run_quiet(&script, ". --reuse-window", Some(&cwd)).expect("lancement");
        let out = base.join("sortie.txt");
        let mut text = String::new();
        for _ in 0..100 {
            text = std::fs::read_to_string(&out).unwrap_or_default();
            if text.lines().count() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = std::fs::remove_dir_all(&base);
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        assert_eq!(lines, [". --reuse-window", &*cwd.to_string_lossy()]);
    }

    #[test]
    fn without_current_folder() {
        let e = Env { cwd: None, ..env() };
        assert_eq!(
            resolve("cmd", &e, &disk()),
            run(r"C:\Windows\system32\cmd.exe", "")
        );
        assert_eq!(resolve("src", &e, &disk()), run("src", ""));
    }
}
