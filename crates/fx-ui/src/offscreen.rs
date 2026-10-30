//! Rendu de la vraie vue fx sans fenêtre, pour les bancs de rendu (`tools/renderbench`).
//!
//! L'appelant fournit un [`AppState`] déjà chargé : `fx-ui` ne fait pas d'I/O.

use std::sync::Arc;

use eframe::egui::{Context, Ui};
use fx_app::AppState;
use fx_shell::ShellWorker;

use crate::{images, theme, view};

pub struct Harness {
    state: AppState,
    view: view::ViewState,
    images: images::ImageCache,
    shell: ShellWorker,
}

impl Harness {
    pub fn new(ctx: &Context, state: AppState) -> Self {
        let accent = theme::system_accent();
        theme::install(ctx, accent);
        let (tx, rx) = crossbeam_channel::unbounded();
        let shell = ShellWorker::new(Arc::new(move |k, img| {
            let _ = tx.send((k, img));
        }))
        .expect("thread shell");
        Self {
            state,
            view: view::ViewState::new(accent),
            images: images::ImageCache::new(rx),
            shell,
        }
    }

    /// Dessine une image complète (barre de titre, navigation, liste, barre d'état).
    pub fn ui(&mut self, ui: &mut Ui) {
        let _ = view::draw(
            ui,
            &self.state,
            &mut self.view,
            &mut self.images,
            &self.shell,
            0,
        );
    }

    /// Comme [`Self::ui`], mais applique à l'état les messages émis (sélection, tri…) ; les
    /// effets (I/O) sont ignorés. Sert aux bancs qui simulent souris et clavier.
    pub fn ui_interactive(&mut self, ui: &mut Ui) {
        let msgs = view::draw(
            ui,
            &self.state,
            &mut self.view,
            &mut self.images,
            &self.shell,
            0,
        );
        for m in msgs {
            let _ = fx_app::update(&mut self.state, m);
        }
    }

    /// Palette Acrylic (fond de fenêtre transparent, couches translucides), comme sous un fond
    /// DWM : sert à vérifier le rendu partiel avec effacement des zones redessinées.
    pub fn set_acrylic(&mut self, on: bool) {
        self.view.backdrop = on;
    }

    /// Fait défiler la liste de `step` points à la prochaine image (retour en haut en fin de liste).
    pub fn scroll(&mut self, step: f32) {
        let pane = self.state.pane();
        let rows = pane.listing.as_ref().map_or(0, |l| l.len());
        self.view.bench_scroll(pane.id, step, rows);
    }

    pub fn rows(&self) -> usize {
        self.state.pane().listing.as_ref().map_or(0, |l| l.len())
    }
}
