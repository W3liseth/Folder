//! Accès au système de fichiers local : énumération en flux, surveillance des changements,
//! recherche récursive. Les opérations (`IFileOperation`) sont exposées par `fx_win::fileop`
//! et orchestrées par `fx-app`.

pub mod archive;
pub mod enumerate;
pub mod measure;
pub mod recycle;
pub mod search;
pub mod watch;

pub use enumerate::{EnumOptions, EnumStats, Outcome, enumerate};
pub use fx_win::Error;
pub use search::{SearchStats, search};
pub use watch::{Change, DirEvent, WatchService};
