//! Wrappers sûrs autour de Win32/COM.
//!
//! C'est la **seule** crate du workspace autorisée à contenir du code `unsafe`.
//! Chaque bloc `unsafe` est justifié par un commentaire `// SAFETY:`.

pub mod clipboard;
pub mod com;
pub mod ctxmenu;
pub mod details;
pub mod dir;
pub mod error;
pub mod fileop;
pub mod image;
pub mod ipc;
pub mod launch;
pub mod path;
pub mod process;
pub mod security;
pub mod shell;
pub mod shellfolder;
pub mod sort;
pub mod sys;
pub mod theme;
pub mod time;
mod util;
pub mod watch;
pub mod window;

pub use error::{Error, Result};
