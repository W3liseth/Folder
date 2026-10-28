//! Persistance de fx.
//!
//! - `settings.toml` (dans `%APPDATA%\folder`) : réglages et raccourcis personnalisés ;
//! - `session.toml` (dans `%LOCALAPPDATA%\folder`) : onglets ouverts ;
//! - `folder.db` (SQLite, `%LOCALAPPDATA%\folder`) : étiquettes et préférences d'affichage par dossier ;
//! - `update.toml` et `updates\` (`%LOCALAPPDATA%\folder`) : dernière vérification de mise à
//!   jour, installeur téléchargé.
//!
//! Les écritures sont faites par le thread de persistance de `fx-app`, jamais par l'UI.
//! Les écritures de fichiers sont atomiques (fichier temporaire puis renommage).

mod db;
pub mod remote;
mod settings;

use std::io;
use std::path::{Path, PathBuf};

pub use db::{Db, FolderPrefs, Tag, TagId};
pub use settings::{
    Layout, PaneSession, Session, Settings, TabSession, ThemeChoice, WindowPlacement,
};

/// Noms des données de l'application, et ceux de son ancien nom (« fx »).
const APP_DIR: &str = "folder";
const OLD_APP_DIR: &str = "fx";
const DB: &str = "folder.db";
const OLD_DB: &str = "fx.db";

/// Renomme `old` en `new` si seul l'ancien existe (dossier ou fichier).
fn migrate(old: &Path, new: &Path) {
    if old.exists()
        && !new.exists()
        && let Err(e) = std::fs::rename(old, new)
    {
        tracing::warn!(error = %e, "reprise de {} impossible", old.display());
    }
}

/// Emplacements des fichiers de folder.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub local: PathBuf,
}

impl Paths {
    /// `%APPDATA%\folder` et `%LOCALAPPDATA%\folder` (repli : dossier temporaire).
    ///
    /// L'application s'appelait « fx » : ses anciens dossiers sont repris s'ils existent
    /// encore (simple renommage, une seule fois).
    pub fn standard() -> Self {
        let env = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir)
        };
        let paths = Self {
            config: env("APPDATA").join(APP_DIR),
            local: env("LOCALAPPDATA").join(APP_DIR),
        };
        migrate(&env("APPDATA").join(OLD_APP_DIR), &paths.config);
        migrate(&env("LOCALAPPDATA").join(OLD_APP_DIR), &paths.local);
        migrate(&paths.local.join(OLD_DB), &paths.db());
        paths
    }

    /// Tout dans un même dossier (tests, mode portable).
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            config: dir.to_path_buf(),
            local: dir.to_path_buf(),
        }
    }

    fn settings(&self) -> PathBuf {
        self.config.join("settings.toml")
    }

    fn session(&self) -> PathBuf {
        self.local.join("session.toml")
    }

    pub(crate) fn db(&self) -> PathBuf {
        self.local.join(DB)
    }

    fn update(&self) -> PathBuf {
        self.local.join("update.toml")
    }

    /// Installeurs téléchargés par la mise à jour automatique.
    pub fn updates_dir(&self) -> PathBuf {
        self.local.join("updates")
    }
}

/// Résultat de la dernière vérification de mise à jour, partagé par toutes les fenêtres
/// (une par processus) : une seule requête réseau par jour, quel que soit leur nombre.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// Heure de la vérification (secondes depuis 1970, UTC).
    pub checked: i64,
    /// Dernière version publiée (sans le `v` du tag).
    pub latest: String,
    /// Installeur de `latest`, téléchargé et vérifié, si elle est plus récente.
    pub installer: Option<PathBuf>,
}

pub fn load_update(paths: &Paths) -> Option<UpdateState> {
    load_toml(&paths.update())
}

pub fn save_update(paths: &Paths, s: &UpdateState) -> io::Result<()> {
    save_toml(&paths.update(), s)
}

pub fn load_settings(paths: &Paths) -> Settings {
    load_toml(&paths.settings()).unwrap_or_default()
}

pub fn save_settings(paths: &Paths, s: &Settings) -> io::Result<()> {
    save_toml(&paths.settings(), s)
}

pub fn load_session(paths: &Paths) -> Option<Session> {
    load_toml(&paths.session())
}

pub fn save_session(paths: &Paths, s: &Session) -> io::Result<()> {
    save_toml(&paths.session(), s)
}

fn load_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    match toml::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "fichier de configuration illisible, valeurs par défaut");
            None
        }
    }
}

fn save_toml<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let text = toml::to_string_pretty(value).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_commands::{CommandId, HotKey};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fx-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn settings_roundtrip_and_defaults() {
        let dir = tmp("settings");
        let paths = Paths::in_dir(&dir);
        assert_eq!(load_settings(&paths), Settings::default());
        // Réglage absent d'un ancien fichier : valeur par défaut.
        let old: Settings = toml::from_str("show_hidden = true").expect("toml");
        assert!(old.show_hidden && (old.scroll_lines - 3.0).abs() < f32::EPSILON);
        let mut s = Settings {
            show_hidden: true,
            grid_size: 128,
            ..Settings::default()
        };
        s.keybindings
            .insert(CommandId::Copy, vec![HotKey::parse("Ctrl+K").expect("hk")]);
        s.pinned.push(PathBuf::from(r"C:\Projets"));
        save_settings(&paths, &s).expect("save");
        assert_eq!(load_settings(&paths), s);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_settings_fall_back_to_defaults() {
        let dir = tmp("corrupt");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("settings.toml"), "show_hidden = 'oui'").expect("write");
        assert_eq!(load_settings(&Paths::in_dir(&dir)), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_roundtrip() {
        let dir = tmp("session");
        let paths = Paths::in_dir(&dir);
        let s = Session {
            tabs: vec![TabSession {
                left: PaneSession {
                    path: Some(PathBuf::from(r"C:\")),
                    layout: Layout::Grid,
                    back: vec![PathBuf::from("folder:home"), PathBuf::from(r"D:\x")],
                    forward: vec![PathBuf::from(r"C:\Windows")],
                },
                right: Some(PaneSession {
                    path: None,
                    layout: Layout::Details,
                    ..PaneSession::default()
                }),
            }],
            active: 0,
            placement: Some(WindowPlacement {
                left: -2400,
                top: 40,
                right: -1300,
                bottom: 740,
                maximized: true,
            }),
        };
        save_session(&paths, &s).expect("save");
        assert_eq!(load_session(&paths), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_state_roundtrip() {
        let dir = tmp("update");
        let paths = Paths::in_dir(&dir);
        assert_eq!(load_update(&paths), None);
        let s = UpdateState {
            checked: 1_793_000_000,
            latest: "0.6.0".into(),
            installer: Some(paths.updates_dir().join("folder-setup-0.6.0-x64.exe")),
        };
        save_update(&paths, &s).expect("save");
        assert_eq!(load_update(&paths), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
