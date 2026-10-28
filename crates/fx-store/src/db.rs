//! Base SQLite : étiquettes, préférences d'affichage par dossier et historique des
//! opérations (centre d'état), partagés par toutes les fenêtres (un processus chacune).
//!
//! Les étiquettes sont indexées par chemin. Les renommages et déplacements faits par fx
//! mettent la base à jour (`move_path`) ; ceux faits hors de fx perdent l'étiquette
//! (cf. ARCHITECTURE.md §7.8).

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::Paths;
use crate::settings::Layout;

pub type TagId = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    /// Couleur RGB.
    pub color: [u8; 3],
}

/// Préférences d'affichage mémorisées pour un dossier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderPrefs {
    pub layout: Layout,
    /// 0 nom, 1 date, 2 type, 3 taille.
    pub sort_key: u8,
    pub sort_desc: bool,
}

/// Issue d'une opération terminée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpOutcome {
    Done,
    Cancelled,
    Failed(String),
}

/// Opération terminée, dans l'historique du centre d'état. Identifiée par `(ended, title)` :
/// les identifiants d'opération sont propres à chaque processus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpEntry {
    /// Fin de l'opération (FILETIME UTC).
    pub ended: i64,
    pub title: String,
    pub items: u32,
    pub outcome: OpOutcome,
    pub duration_ms: u64,
}

/// Opérations gardées dans l'historique (les plus récentes).
pub const MAX_OP_HISTORY: usize = 100;

pub struct Db {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL, color INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS tagged (path TEXT NOT NULL COLLATE NOCASE, tag INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE, PRIMARY KEY (path, tag));
CREATE TABLE IF NOT EXISTS folder_prefs (path TEXT PRIMARY KEY COLLATE NOCASE, layout TEXT NOT NULL, sort_key INTEGER NOT NULL, sort_desc INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS op_history (id INTEGER PRIMARY KEY, ended INTEGER NOT NULL, title TEXT NOT NULL, items INTEGER NOT NULL, outcome TEXT NOT NULL, error TEXT, duration_ms INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS op_history_ended ON op_history (ended);
";

/// Étiquettes proposées au premier lancement (comme Files).
const DEFAULT_TAGS: [(&str, [u8; 3]); 4] = [
    ("Important", [232, 17, 35]),
    ("Travail", [0, 120, 212]),
    ("Personnel", [16, 137, 62]),
    ("Plus tard", [255, 140, 0]),
];

fn rgb(c: [u8; 3]) -> i64 {
    (i64::from(c[0]) << 16) | (i64::from(c[1]) << 8) | i64::from(c[2])
}

fn unrgb(v: i64) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

fn layout_str(l: Layout) -> &'static str {
    match l {
        Layout::Details => "details",
        Layout::List => "list",
        Layout::Grid => "grid",
        Layout::Columns => "columns",
    }
}

fn layout_parse(s: &str) -> Layout {
    match s {
        "list" => Layout::List,
        "grid" => Layout::Grid,
        "columns" => Layout::Columns,
        _ => Layout::Details,
    }
}

impl Db {
    pub fn open(paths: &Paths) -> rusqlite::Result<Self> {
        let _ = std::fs::create_dir_all(&paths.local);
        Self::init(Connection::open(paths.db())?)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))?;
        if n == 0 {
            for (name, color) in DEFAULT_TAGS {
                conn.execute(
                    "INSERT INTO tags (name, color) VALUES (?1, ?2)",
                    params![name, rgb(color)],
                )?;
            }
        }
        Ok(Self { conn })
    }

    pub fn tags(&self) -> rusqlite::Result<Vec<Tag>> {
        let mut st = self
            .conn
            .prepare("SELECT id, name, color FROM tags ORDER BY id")?;
        st.query_map([], |r| {
            Ok(Tag {
                id: r.get(0)?,
                name: r.get(1)?,
                color: unrgb(r.get(2)?),
            })
        })?
        .collect()
    }

    /// Crée (id 0) ou met à jour une étiquette ; renvoie son identifiant.
    pub fn save_tag(&self, tag: &Tag) -> rusqlite::Result<TagId> {
        if tag.id == 0 {
            self.conn.execute(
                "INSERT INTO tags (name, color) VALUES (?1, ?2)",
                params![tag.name, rgb(tag.color)],
            )?;
            Ok(self.conn.last_insert_rowid() as TagId)
        } else {
            self.conn.execute(
                "UPDATE tags SET name = ?2, color = ?3 WHERE id = ?1",
                params![tag.id, tag.name, rgb(tag.color)],
            )?;
            Ok(tag.id)
        }
    }

    pub fn delete_tag(&self, id: TagId) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM tags WHERE id = ?1", params![id])
            .map(drop)
    }

    /// Historique des opérations, du plus récent au plus ancien.
    pub fn op_history(&self) -> rusqlite::Result<Vec<OpEntry>> {
        let mut st = self.conn.prepare(
            "SELECT ended, title, items, outcome, error, duration_ms FROM op_history
             ORDER BY ended DESC, id DESC LIMIT ?1",
        )?;
        st.query_map(params![MAX_OP_HISTORY as i64], |r| {
            let outcome: String = r.get(3)?;
            Ok(OpEntry {
                ended: r.get(0)?,
                title: r.get(1)?,
                items: r.get(2)?,
                outcome: match outcome.as_str() {
                    "cancelled" => OpOutcome::Cancelled,
                    "failed" => {
                        OpOutcome::Failed(r.get::<_, Option<String>>(4)?.unwrap_or_default())
                    }
                    _ => OpOutcome::Done,
                },
                duration_ms: r.get::<_, i64>(5)?.max(0) as u64,
            })
        })?
        .collect()
    }

    /// Ajoute une opération terminée ; au-delà de [`MAX_OP_HISTORY`], les plus anciennes
    /// sont retirées.
    pub fn add_op(&self, e: &OpEntry) -> rusqlite::Result<()> {
        let (outcome, error) = match &e.outcome {
            OpOutcome::Done => ("done", None),
            OpOutcome::Cancelled => ("cancelled", None),
            OpOutcome::Failed(err) => ("failed", Some(err.as_str())),
        };
        self.conn.execute(
            "INSERT INTO op_history (ended, title, items, outcome, error, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                e.ended,
                e.title,
                e.items,
                outcome,
                error,
                e.duration_ms.min(i64::MAX as u64) as i64
            ],
        )?;
        self.conn
            .execute(
                "DELETE FROM op_history WHERE id NOT IN
                 (SELECT id FROM op_history ORDER BY ended DESC, id DESC LIMIT ?1)",
                params![MAX_OP_HISTORY as i64],
            )
            .map(drop)
    }

    pub fn remove_op(&self, ended: i64, title: &str) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "DELETE FROM op_history WHERE ended = ?1 AND title = ?2",
                params![ended, title],
            )
            .map(drop)
    }

    pub fn clear_ops(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM op_history", []).map(drop)
    }

    /// Toutes les associations chemin → étiquette (chargées en mémoire au démarrage).
    pub fn tagged(&self) -> rusqlite::Result<Vec<(PathBuf, TagId)>> {
        let mut st = self.conn.prepare("SELECT path, tag FROM tagged")?;
        st.query_map([], |r| {
            Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?))
        })?
        .collect()
    }

    pub fn set_tags(&self, path: &Path, tags: &[TagId]) -> rusqlite::Result<()> {
        let p = path.to_string_lossy();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM tagged WHERE path = ?1", params![p])?;
        for t in tags {
            tx.execute(
                "INSERT OR IGNORE INTO tagged (path, tag) VALUES (?1, ?2)",
                params![p, t],
            )?;
        }
        tx.commit()
    }

    /// Suit un renommage ou un déplacement fait par fx (le chemin et tout son contenu).
    pub fn move_path(&self, from: &Path, to: &Path) -> rusqlite::Result<()> {
        let (f, t) = (from.to_string_lossy(), to.to_string_lossy());
        let prefix = format!("{f}\\");
        self.conn.execute(
            "UPDATE OR REPLACE tagged SET path = ?2 WHERE path = ?1",
            params![f, t],
        )?;
        self.conn.execute(
            "UPDATE OR REPLACE tagged SET path = ?2 || substr(path, ?3) WHERE path LIKE ?4 ESCAPE '^'",
            params![f, t, prefix.chars().count() as i64, format!("{}%", escape_like(&prefix))],
        )?;
        self.conn.execute(
            "UPDATE OR REPLACE folder_prefs SET path = ?2 WHERE path = ?1",
            params![f, t],
        )?;
        Ok(())
    }

    pub fn folder_prefs(&self, path: &Path) -> rusqlite::Result<Option<FolderPrefs>> {
        self.conn
            .query_row(
                "SELECT layout, sort_key, sort_desc FROM folder_prefs WHERE path = ?1",
                params![path.to_string_lossy()],
                |r| {
                    Ok(FolderPrefs {
                        layout: layout_parse(&r.get::<_, String>(0)?),
                        sort_key: r.get(1)?,
                        sort_desc: r.get::<_, i64>(2)? != 0,
                    })
                },
            )
            .optional()
    }

    /// Toutes les préférences par dossier (chargées en mémoire au démarrage).
    pub fn all_folder_prefs(&self) -> rusqlite::Result<Vec<(PathBuf, FolderPrefs)>> {
        let mut st = self
            .conn
            .prepare("SELECT path, layout, sort_key, sort_desc FROM folder_prefs")?;
        st.query_map([], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                FolderPrefs {
                    layout: layout_parse(&r.get::<_, String>(1)?),
                    sort_key: r.get(2)?,
                    sort_desc: r.get::<_, i64>(3)? != 0,
                },
            ))
        })?
        .collect()
    }

    pub fn set_folder_prefs(&self, path: &Path, p: FolderPrefs) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO folder_prefs (path, layout, sort_key, sort_desc) VALUES (?1, ?2, ?3, ?4)",
                params![path.to_string_lossy(), layout_str(p.layout), p.sort_key, i64::from(p.sort_desc)],
            )
            .map(drop)
    }
}

fn escape_like(s: &str) -> String {
    s.replace('^', "^^").replace('%', "^%").replace('_', "^_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn op_history_is_bounded_newest_first_and_clearable() {
        let db = Db::in_memory().expect("db");
        let entry = |ended: i64, outcome| OpEntry {
            ended,
            title: format!("op {ended}"),
            items: 2,
            outcome,
            duration_ms: 1500,
        };
        db.add_op(&entry(1, OpOutcome::Failed("Accès refusé.".into())))
            .expect("add");
        db.add_op(&entry(2, OpOutcome::Cancelled)).expect("add");
        let h = db.op_history().expect("history");
        assert_eq!(
            h,
            vec![
                entry(2, OpOutcome::Cancelled),
                entry(1, OpOutcome::Failed("Accès refusé.".into()))
            ]
        );
        db.remove_op(2, "op 2").expect("remove");
        assert_eq!(db.op_history().expect("history").len(), 1);
        for ended in 10..(10 + MAX_OP_HISTORY as i64 + 5) {
            db.add_op(&entry(ended, OpOutcome::Done)).expect("add");
        }
        let h = db.op_history().expect("history");
        assert_eq!(h.len(), MAX_OP_HISTORY);
        assert_eq!(h[0].ended, 10 + MAX_OP_HISTORY as i64 + 4);
        let rows: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM op_history", [], |r| r.get(0))
            .expect("count");
        assert_eq!(
            rows, MAX_OP_HISTORY as i64,
            "les plus anciennes sont retirées"
        );
        db.clear_ops().expect("clear");
        assert!(db.op_history().expect("history").is_empty());
    }

    #[test]
    fn default_tags_and_crud() {
        let db = Db::in_memory().expect("db");
        let tags = db.tags().expect("tags");
        assert_eq!(tags.len(), 4);
        let id = db
            .save_tag(&Tag {
                id: 0,
                name: "Factures".into(),
                color: [1, 2, 3],
            })
            .expect("save");
        assert_eq!(
            db.tags().expect("tags").last().map(|t| (t.id, t.color)),
            Some((id, [1, 2, 3]))
        );
        db.set_tags(Path::new(r"C:\a\f.txt"), &[1, id])
            .expect("set");
        assert_eq!(db.tagged().expect("tagged").len(), 2);
        db.delete_tag(id).expect("delete");
        assert_eq!(
            db.tagged().expect("tagged"),
            vec![(PathBuf::from(r"C:\a\f.txt"), 1)]
        );
    }

    #[test]
    fn move_path_follows_children_case_insensitively() {
        let db = Db::in_memory().expect("db");
        db.set_tags(Path::new(r"C:\Projets\a.txt"), &[1])
            .expect("set");
        db.set_tags(Path::new(r"C:\Projets"), &[2]).expect("set");
        db.set_tags(Path::new(r"C:\Projets2\b.txt"), &[3])
            .expect("set");
        db.move_path(Path::new(r"C:\projets"), Path::new(r"D:\Archives"))
            .expect("move");
        let mut t = db.tagged().expect("tagged");
        t.sort();
        assert_eq!(
            t,
            vec![
                (PathBuf::from(r"C:\Projets2\b.txt"), 3),
                (PathBuf::from(r"D:\Archives"), 2),
                (PathBuf::from(r"D:\Archives\a.txt"), 1),
            ]
        );
    }

    #[test]
    fn folder_prefs_roundtrip() {
        let db = Db::in_memory().expect("db");
        let p = FolderPrefs {
            layout: Layout::Grid,
            sort_key: 1,
            sort_desc: true,
        };
        db.set_folder_prefs(Path::new(r"C:\Images"), p)
            .expect("set");
        assert_eq!(
            db.folder_prefs(Path::new(r"c:\images")).expect("get"),
            Some(p)
        );
        assert_eq!(db.all_folder_prefs().expect("all").len(), 1);
    }
}
