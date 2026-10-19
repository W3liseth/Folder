//! Types de domaine purs de fx : aucune I/O, aucune dépendance Windows ni UI.
//!
//! Le cœur est [`DirSnapshot`] : un dossier listé stocké en structure de tableaux
//! (40 octets par entrée + nom dans une arène), sans objet alloué par fichier.

pub mod cancel;
pub mod entry;
pub mod format;
pub mod names;
pub mod selection;
pub mod snapshot;
pub mod sort;
pub mod wtf8;

pub use cancel::CancelToken;
pub use entry::{Batch, EntryFlags, EntryMeta, ExtId};
pub use names::{NameArena, NameSpan};
pub use selection::Selection;
pub use snapshot::{DirSnapshot, DirStats, EntryView};
pub use sort::{SortDir, SortKey, SortSpec, natural_cmp};
