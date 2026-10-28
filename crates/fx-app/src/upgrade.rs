//! Mise à jour automatique et notes de version (ARCHITECTURE.md §7.12).
//!
//! - La dernière version publiée est lue sur les releases GitHub du dépôt (redirection de
//!   `releases/latest`, préversions exclues) par un processus éphémère
//!   `folder.exe --update-check` : WinHTTP n'est jamais chargé dans une fenêtre folder
//!   (mémoire et threads au repos inchangés). Si elle est plus récente, ce processus
//!   télécharge son installeur, vérifie sa signature et note le résultat dans `update.toml`,
//!   partagé par toutes les fenêtres (une par processus).
//! - Sans demande explicite, au plus une requête par jour, et seulement si le réglage
//!   `auto_update` est actif ; aucune donnée envoyée (pas même un identifiant).
//! - L'installation reste à la demande : installeur Inno Setup en mode silencieux, qui ferme
//!   folder puis le relance (`/RELAUNCH=1`, cf. `installer/folder.iss`).

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use fx_store::{Paths, UpdateState};

/// Notes de version, de la plus récente à la plus ancienne (`CHANGELOG.md` à la racine).
pub const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// Argument du processus de vérification (cf. `app/folder`).
pub const CHECK_ARG: &str = "--update-check";

const REPO: &str = env!("CARGO_PKG_REPOSITORY");
const DAY: i64 = 24 * 3600;
/// Garde-fou : l'installeur fait quelques Mo.
const MAX_INSTALLER: u64 = 64 << 20;
/// Pas de console (build debug) pour le processus de vérification.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Version `majeur.mineur.correctif[-préversion]`. Une préversion précède la version finale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub pre: Option<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let (num, pre) = match s.split_once('-') {
            Some((n, p)) if !p.is_empty() => (n, Some(p.to_owned())),
            Some(_) => return None,
            None => (s, None),
        };
        let mut it = num.split('.').map(|p| p.parse::<u32>().ok());
        let v = Self {
            major: it.next()??,
            minor: it.next()??,
            patch: it.next()??,
            pre,
        };
        it.next().is_none().then_some(v)
    }

    /// Version de ce binaire.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self {
            major: 0,
            minor: 0,
            patch: 0,
            pre: None,
        })
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(o.major, o.minor, o.patch))
            .then_with(|| match (&self.pre, &o.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

/// État de la mise à jour, tel que l'UI l'affiche.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    /// Pas encore vérifié.
    #[default]
    Unknown,
    Checking,
    /// `latest` : dernière version publiée ; `checked` : heure de la vérification (s, UTC).
    UpToDate {
        latest: String,
        checked: i64,
    },
    /// Installeur téléchargé et vérifié, prêt à être lancé.
    Ready {
        version: String,
        installer: PathBuf,
    },
    /// folder ne tourne pas depuis son installation (`cargo run`, copie de l'exécutable) :
    /// l'installeur ne saurait pas le remplacer.
    NotInstalled,
    Failed(String),
}

impl Status {
    pub fn ready_version(&self) -> Option<&str> {
        match self {
            Self::Ready { version, .. } => Some(version),
            _ => None,
        }
    }
}

/// folder lancé depuis le dossier où l'installeur l'a mis (son désinstalleur est à côté).
pub fn installed() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("unins000.exe").is_file()))
        .unwrap_or(false)
}

/// Dernier résultat connu (fichier partagé), sans accès réseau.
pub fn cached(paths: &Paths) -> Status {
    if !installed() {
        return Status::NotInstalled;
    }
    fx_store::load_update(paths)
        .and_then(|s| from_state(&s))
        .unwrap_or_default()
}

/// Vérifie (et télécharge) si nécessaire : `force` ignore le résultat du jour. Bloquant
/// (processus de vérification) : à appeler sur un thread de travail. Le résultat du jour
/// est rendu aussitôt ; sinon le processus de vérification part après `delay` (laisser
/// passer le démarrage). Renvoie le statut et s'il vient d'une vérification en ligne
/// (sinon : du fichier partagé).
pub fn check(paths: &Paths, force: bool, delay: std::time::Duration) -> (Status, bool) {
    if !installed() {
        return (Status::NotInstalled, false);
    }
    if !force
        && let Some(state) = fx_store::load_update(paths)
        && (0..DAY).contains(&(now() - state.checked))
        && let Some(status) = from_state(&state)
    {
        return (status, false);
    }
    std::thread::sleep(delay);
    (run_check_process(), true)
}

/// Statut d'après le fichier partagé ; `None` s'il faut revérifier (version plus récente
/// annoncée mais installeur absent, par exemple supprimé).
fn from_state(s: &UpdateState) -> Option<Status> {
    let latest = Version::parse(&s.latest)?;
    if latest <= Version::current() {
        return Some(Status::UpToDate {
            latest: s.latest.clone(),
            checked: s.checked,
        });
    }
    let installer = s.installer.clone().filter(|p| p.is_file())?;
    Some(Status::Ready {
        version: latest.to_string(),
        installer,
    })
}

fn run_check_process() -> Status {
    use std::os::windows::process::CommandExt;
    let out = std::env::current_exe().and_then(|exe| {
        Command::new(exe)
            .arg(CHECK_ARG)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
    });
    match out {
        Ok(o) => decode(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => Status::Failed(format!("vérification impossible : {e}")),
    }
}

/// Corps du processus `folder.exe --update-check` : une ligne à écrire sur stdout.
pub fn check_process(paths: &Paths) -> String {
    encode(&check_online(paths).unwrap_or_else(Status::Failed))
}

fn check_online(paths: &Paths) -> Result<Status, String> {
    let agent = format!("folder/{}", env!("CARGO_PKG_VERSION"));
    let head = fx_win::http::head(&format!("{REPO}/releases/latest"), &agent).map_err(net)?;
    let latest = match (head.status, head.location.as_deref()) {
        (301 | 302 | 303 | 307 | 308, Some(loc)) => {
            tag_version(loc).ok_or("aucune version publiée")?
        }
        (404, _) => return Err("releases introuvables (dépôt privé ?)".into()),
        (s, _) => return Err(format!("réponse HTTP {s}")),
    };
    let dir = paths.updates_dir();
    let mut state = UpdateState {
        checked: now(),
        latest: latest.to_string(),
        installer: None,
    };
    if latest <= Version::current() {
        remove_installers(&dir, None);
        save(paths, &state);
        return Ok(Status::UpToDate {
            latest: state.latest,
            checked: state.checked,
        });
    }
    let name = format!("folder-setup-{latest}-x64.exe");
    let file = dir.join(&name);
    if !file.is_file() {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let part = dir.join(format!("{name}.{}.part", std::process::id()));
        let url = format!("{REPO}/releases/download/v{latest}/{name}");
        let done = fx_win::http::download(&url, &agent, &part, MAX_INSTALLER)
            .map_err(|e| match e {
                fx_win::http::DownloadError::Http(e) => net(e),
                e => format!("téléchargement : {e}"),
            })
            .and_then(|_| std::fs::rename(&part, &file).map_err(|e| e.to_string()));
        if let Err(e) = done {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    }
    if let Err(e) = verify(&file) {
        let _ = std::fs::remove_file(&file);
        save(paths, &state);
        return Err(e);
    }
    remove_installers(&dir, Some(&file));
    state.installer = Some(file.clone());
    save(paths, &state);
    Ok(Status::Ready {
        version: latest.to_string(),
        installer: file,
    })
}

/// Un exécutable Windows, signé par le même éditeur que ce folder s'il est signé (une
/// version non signée, construite à la main, ne peut rien exiger).
fn verify(file: &Path) -> Result<(), String> {
    let mut magic = [0u8; 2];
    let is_exe = std::fs::File::open(file)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic))
        .is_ok()
        && &magic == b"MZ";
    if !is_exe {
        return Err("installeur téléchargé invalide".into());
    }
    let me = std::env::current_exe()
        .ok()
        .and_then(|exe| fx_win::trust::signer(&exe));
    match me {
        Some(me) if fx_win::trust::signer(file).as_deref() != Some(&me) => {
            Err(format!("installeur non signé par {me} : ignoré"))
        }
        _ => Ok(()),
    }
}

/// Supprime les installeurs (et téléchargements interrompus) autres que `keep`.
fn remove_installers(dir: &Path, keep: Option<&Path>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if Some(p.as_path()) != keep
            && p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("folder-setup-"))
        {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn save(paths: &Paths, s: &UpdateState) {
    if let Err(e) = fx_store::save_update(paths, s) {
        tracing::warn!(error = %e, "update.toml non enregistré");
    }
}

/// `…/releases/tag/v1.2.3` → 1.2.3.
fn tag_version(location: &str) -> Option<Version> {
    Version::parse(location.rsplit_once("/tag/")?.1)
}

/// Message lisible d'une erreur WinHTTP (codes 12xxx, sans texte système associé).
fn net(e: fx_win::Error) -> String {
    let hr = e.hresult() as u32;
    let code = if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    };
    match code {
        12002 => "délai dépassé".into(),
        12007 => "serveur introuvable (pas de connexion ?)".into(),
        12029 | 12030 => "connexion impossible".into(),
        12175 | 12045 | 12044 | 12169 => "certificat du serveur refusé".into(),
        _ => {
            let m = e.message();
            let m = m.trim();
            if m.is_empty() {
                format!("erreur réseau {code}")
            } else {
                m.to_owned()
            }
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Ligne de réponse du processus de vérification (une valeur par champ, tabulations).
fn encode(s: &Status) -> String {
    match s {
        Status::UpToDate { latest, checked } => format!("current\t{latest}\t{checked}"),
        Status::Ready { version, installer } => {
            format!("ready\t{version}\t{}", installer.display())
        }
        Status::Failed(e) => format!("error\t{}", e.replace(['\t', '\n', '\r'], " ")),
        Status::NotInstalled | Status::Unknown | Status::Checking => "error\t".into(),
    }
}

fn decode(line: &str) -> Status {
    let mut it = line.trim_end_matches(['\r', '\n']).split('\t');
    match (it.next(), it.next(), it.next()) {
        (Some("current"), Some(latest), Some(checked)) => Status::UpToDate {
            latest: latest.to_owned(),
            checked: checked.parse().unwrap_or(0),
        },
        (Some("ready"), Some(version), Some(path)) if !path.is_empty() => Status::Ready {
            version: version.to_owned(),
            installer: PathBuf::from(path),
        },
        (Some("error"), Some(e), _) if !e.is_empty() => Status::Failed(e.to_owned()),
        _ => Status::Failed("vérification interrompue".into()),
    }
}

/// Lance l'installeur prêt d'après le fichier partagé (cf. [`launch_installer`]).
pub fn install(paths: &Paths) -> std::io::Result<()> {
    match cached(paths) {
        Status::Ready { installer, .. } => launch_installer(&installer),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "aucune mise à jour téléchargée",
        )),
    }
}

/// Lance l'installeur en mode silencieux : il ferme les fenêtres folder (gestionnaire de
/// redémarrage), remplace l'exécutable, puis relance folder (`/RELAUNCH=1`), dont la
/// session est restaurée. Une installation « pour tous les utilisateurs » demande
/// l'élévation (UAC).
pub fn launch_installer(installer: &Path) -> std::io::Result<()> {
    if !installer.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "installeur introuvable",
        ));
    }
    Command::new(installer)
        .args([
            "/SILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
            "/RELAUNCH=1",
        ])
        .spawn()
        .map(drop)
}

/// Une version des notes : numéro, date, changements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: &'static str,
    pub date: &'static str,
    pub items: Vec<String>,
}

/// Notes de version embarquées, de la plus récente à la plus ancienne.
pub fn notes() -> Vec<Release> {
    parse_notes(CHANGELOG)
}

/// Format de `CHANGELOG.md` : `## <version> — <date>`, puis une liste `- …` (lignes de
/// continuation indentées). Le reste (titre, paragraphes) est ignoré ; les accents graves
/// du Markdown sont retirés.
fn parse_notes(text: &'static str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            let (version, date) = h
                .split_once(" — ")
                .or_else(|| h.split_once(" - "))
                .unwrap_or((h, ""));
            out.push(Release {
                version: version.trim(),
                date: date.trim(),
                items: Vec::new(),
            });
            continue;
        }
        let Some(r) = out.last_mut() else { continue };
        if let Some(item) = line.strip_prefix("- ") {
            r.items.push(item.trim().replace('`', ""));
        } else if line.starts_with("  ")
            && !line.trim().is_empty()
            && let Some(last) = r.items.last_mut()
        {
            last.push(' ');
            last.push_str(&line.trim().replace('`', ""));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).expect(s)
    }

    #[test]
    fn versions() {
        assert_eq!(v("v0.5.2").to_string(), "0.5.2");
        assert_eq!(v("1.2.3-beta.1").pre.as_deref(), Some("beta.1"));
        assert!(v("0.6.0") > v("0.5.2"));
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("0.6.0") > v("0.6.0-beta.1"));
        assert!(v("0.6.0-beta.2") > v("0.6.0-beta.1"));
        assert_eq!(v("0.5.2").cmp(&v("v0.5.2")), Ordering::Equal);
        for bad in ["", "1.2", "1.2.3.4", "1.x.3", "1.2.3-", "latest"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn latest_from_redirect() {
        assert_eq!(
            tag_version("https://github.com/o/r/releases/tag/v0.6.0"),
            Some(v("0.6.0"))
        );
        // Dépôt sans release : GitHub redirige vers la liste des releases.
        assert_eq!(tag_version("https://github.com/o/r/releases"), None);
    }

    #[test]
    fn process_reply_roundtrip() {
        for s in [
            Status::UpToDate {
                latest: "0.5.2".into(),
                checked: 1_793_000_000,
            },
            Status::Ready {
                version: "0.6.0".into(),
                installer: PathBuf::from(r"C:\Users\x y\folder-setup-0.6.0-x64.exe"),
            },
            Status::Failed("délai dépassé".into()),
        ] {
            assert_eq!(decode(&format!("{}\r\n", encode(&s))), s);
        }
        assert!(matches!(decode(""), Status::Failed(_)));
        assert!(matches!(decode("ready\t0.6.0\t"), Status::Failed(_)));
    }

    #[test]
    fn shared_state() {
        let dir = std::env::temp_dir().join(format!("fx-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let installer = dir.join("folder-setup-99.0.0-x64.exe");
        let newer = UpdateState {
            checked: now(),
            latest: "99.0.0".into(),
            installer: Some(installer.clone()),
        };
        // Installeur absent : il faut revérifier.
        assert_eq!(from_state(&newer), None);
        std::fs::write(&installer, b"MZ").expect("write");
        assert_eq!(
            from_state(&newer),
            Some(Status::Ready {
                version: "99.0.0".into(),
                installer: installer.clone(),
            })
        );
        let same = UpdateState {
            latest: env!("CARGO_PKG_VERSION").into(),
            installer: None,
            ..newer
        };
        assert!(matches!(from_state(&same), Some(Status::UpToDate { .. })));
        assert_eq!(from_state(&UpdateState::default()), None);
        let keep = dir.join("folder-setup-100.0.0-x64.exe");
        std::fs::write(&keep, b"MZ").expect("write");
        std::fs::write(dir.join("autre.txt"), b"x").expect("write");
        remove_installers(&dir, Some(&keep));
        assert!(!installer.exists() && keep.exists() && dir.join("autre.txt").exists());
        assert!(verify(&dir.join("autre.txt")).is_err(), "pas un exécutable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_cover_every_version() {
        let notes = notes();
        let first = notes.first().expect("CHANGELOG.md vide");
        assert_eq!(
            first.version,
            env!("CARGO_PKG_VERSION"),
            "CHANGELOG.md doit commencer par la version de Cargo.toml"
        );
        for r in &notes {
            assert!(Version::parse(r.version).is_some(), "{}", r.version);
            assert!(!r.date.is_empty(), "date manquante : {}", r.version);
            assert!(!r.items.is_empty(), "aucun changement : {}", r.version);
        }
        assert!(
            notes.windows(2).all(|w| v(w[0].version) > v(w[1].version)),
            "versions dans l'ordre décroissant"
        );
    }

    #[test]
    fn notes_format() {
        let n = parse_notes(
            "# Notes\n\nIntro.\n\n## 1.1.0 — 2026-01-02\n\n- Un `code`\n  suite\n- Deux\n\n## 1.0.0 - 2026-01-01\n- Trois\n",
        );
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].version, "1.1.0");
        assert_eq!(n[0].date, "2026-01-02");
        assert_eq!(n[0].items, ["Un code suite", "Deux"]);
        assert_eq!(n[1].date, "2026-01-01");
    }
}
