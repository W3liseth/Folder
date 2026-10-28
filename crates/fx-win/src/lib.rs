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
pub mod http;
pub mod image;
pub mod ipc;
pub mod launch;
pub mod overlay;
pub mod path;
pub mod process;
pub mod security;
pub mod shell;
pub mod shellfolder;
pub mod shortcuts;
pub mod sort;
pub mod sys;
pub mod theme;
pub mod time;
pub mod trust;
mod util;
pub mod watch;
pub mod win_e;
pub mod window;

pub use error::{Error, Result};
