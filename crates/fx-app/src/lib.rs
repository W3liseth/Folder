//! Cœur applicatif de fx, indépendant de l'UI.
//!
//! Boucle « message → update → effets » (ARCHITECTURE.md §6) :
//! l'UI traduit les entrées en [`Msg`], [`update`] modifie l'[`AppState`] sans I/O et
//! renvoie des [`Effect`], que le [`Scheduler`] exécute hors du thread UI ; les résultats
//! reviennent sous forme de `Msg`.

pub mod effect;
pub mod git;
pub mod history;
pub mod msg;
pub mod ops;
pub mod scheduler;
pub mod state;
pub mod system;
pub mod update;
pub mod upgrade;

pub use effect::Effect;
pub use history::Location;
pub use msg::Msg;
pub use scheduler::{Scheduler, Waker};
pub use state::{AppState, ListingStatus, Pane, PaneId, Tags};
pub use update::update;

/// Titre de la fenêtre Paramètres (processus `--settings`), pour la ramener au premier plan.
pub const SETTINGS_TITLE: &str = "Paramètres de folder";
