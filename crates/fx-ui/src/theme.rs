//! Thème Fluent (inspiré de Files et de l'Explorateur de Windows 11) : couleurs en couches,
//! polices système Segoe, accent système, clair/sombre.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Theme, Visuals,
};
use fx_store::ThemeChoice;

/// Famille des glyphes d'icônes (Segoe Fluent Icons / Segoe MDL2 Assets).
pub const ICONS: &str = "icons";
/// Famille semi-grasse (titres, onglet actif).
pub const SEMIBOLD: &str = "semibold";

/// Glyphes Segoe Fluent Icons (mêmes points de code dans Segoe MDL2 Assets).
pub mod glyph {
    pub const BACK: &str = "\u{E72B}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const UP: &str = "\u{E74A}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const HOME: &str = "\u{E80F}";
    pub const FOLDER: &str = "\u{E8B7}";
    pub const FILE: &str = "\u{E7C3}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const CHEVRON_UP: &str = "\u{E70E}";
    pub const SORT_UP: &str = "\u{E74A}";
    pub const SORT_DOWN: &str = "\u{E74B}";
    pub const ERROR: &str = "\u{E783}";
    pub const SEARCH: &str = "\u{E721}";
    pub const ADD: &str = "\u{E710}";
    pub const CLOSE: &str = "\u{E8BB}";
    pub const MINIMIZE: &str = "\u{E921}";
    pub const MAXIMIZE: &str = "\u{E922}";
    pub const RESTORE: &str = "\u{E923}";
    pub const MORE: &str = "\u{E712}";
    pub const DRIVE: &str = "\u{EDA2}";
    pub const NETWORK: &str = "\u{E968}";
    pub const USB: &str = "\u{E88E}";
    pub const DISC: &str = "\u{E958}";
    pub const TAG: &str = "\u{E8EC}";
    pub const PIN: &str = "\u{E718}";
    pub const RECYCLE: &str = "\u{E74D}";
    pub const SETTINGS: &str = "\u{E713}";
    pub const CHECK: &str = "\u{E73E}";
    pub const INFO: &str = "\u{E946}";
    pub const BRANCH: &str = "\u{E8B1}";
    pub const DETAILS: &str = "\u{E8EF}";
    pub const GRID: &str = "\u{E8A9}";
    pub const LIST: &str = "\u{E8FD}";
    pub const COLUMNS: &str = "\u{E80A}";
    pub const PREVIEW: &str = "\u{E8A1}";
    pub const SORT: &str = "\u{E8CB}";
    pub const VIEW: &str = "\u{E890}";
    pub const NEW: &str = "\u{E710}";
    pub const CUT: &str = "\u{E8C6}";
    pub const COPY: &str = "\u{E8C8}";
    pub const PASTE: &str = "\u{E77F}";
    pub const RENAME: &str = "\u{E8AC}";
    pub const DELETE: &str = "\u{E74D}";
    pub const CLOCK: &str = "\u{E823}";
    pub const HISTORY: &str = "\u{E81C}";
    pub const DUAL: &str = "\u{E89F}";
    pub const STAR: &str = "\u{E734}";
    pub const SUCCESS: &str = "\u{E930}";
    pub const WARNING: &str = "\u{E7BA}";
    /// Analyse de l'espace (secteur de camembert).
    pub const PIE: &str = "\u{EB05}";
    pub const BROOM: &str = "\u{EA99}";
    pub const CANCEL: &str = "\u{E711}";
    pub const OPEN: &str = "\u{E8E5}";
    pub const TERMINAL: &str = "\u{E756}";
    pub const ZIP: &str = "\u{F012}";
    pub const KEYBOARD: &str = "\u{E765}";
    pub const PALETTE: &str = "\u{E790}";
    pub const HIDDEN: &str = "\u{ED1A}";
    pub const DOWNLOAD: &str = "\u{E896}";
    pub const SYNC: &str = "\u{E895}";
    pub const NOTES: &str = "\u{E7BC}";
    pub const DESKTOP: &str = "\u{E7F4}";
    /// Restaurer depuis la corbeille.
    pub const UNDELETE: &str = "\u{E777}";
    /// Distribution Linux (WSL), en attendant l'icône de Windows.
    pub const LINUX: &str = "\u{E756}";
    pub const DOCUMENTS: &str = "\u{E8A5}";
    pub const PICTURES: &str = "\u{EB9F}";
    pub const MUSIC: &str = "\u{EC4F}";
    pub const VIDEOS: &str = "\u{E714}";
    pub const PERSON: &str = "\u{E77B}";
    pub const UNDO: &str = "\u{E7A7}";
    pub const REDO: &str = "\u{E7A6}";
    pub const SELECT_ALL: &str = "\u{E8B3}";
}

/// Couleurs dérivées du thème courant (inspirées des « layers » de Fluent).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// Fond de la fenêtre (barre de titre, barres d'outils).
    pub base: Color32,
    /// Fond des zones de contenu (cartes).
    pub layer: Color32,
    /// Fond de la barre latérale.
    pub side: Color32,
    pub card: Color32,
    pub stroke: Color32,
    pub stroke_strong: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub selection: Color32,
    pub selection_hover: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub folder: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub scrim: Color32,
}

impl Palette {
    pub fn new(dark: bool, accent: Color32) -> Self {
        Self::neutral(dark, accent)
    }

    fn neutral(dark: bool, accent: Color32) -> Self {
        if dark {
            Self {
                dark,
                base: Color32::from_rgb(32, 32, 32),
                layer: Color32::from_rgb(39, 39, 39),
                side: Color32::from_rgb(32, 32, 32),
                card: Color32::from_rgb(45, 45, 45),
                stroke: Color32::from_white_alpha(14),
                stroke_strong: Color32::from_white_alpha(28),
                text: Color32::from_gray(240),
                text2: Color32::from_gray(190),
                text3: Color32::from_gray(130),
                accent: lighten(accent, 0.35),
                on_accent: Color32::from_gray(10),
                selection: accent.gamma_multiply(0.32),
                selection_hover: accent.gamma_multiply(0.42),
                hover: Color32::from_white_alpha(10),
                pressed: Color32::from_white_alpha(6),
                folder: Color32::from_rgb(255, 202, 92),
                danger: Color32::from_rgb(255, 99, 97),
                success: Color32::from_rgb(108, 203, 95),
                warning: Color32::from_rgb(252, 225, 0),
                scrim: Color32::from_black_alpha(110),
            }
        } else {
            Self {
                dark,
                base: Color32::from_rgb(238, 238, 238),
                layer: Color32::from_rgb(251, 251, 251),
                side: Color32::from_rgb(238, 238, 238),
                card: Color32::WHITE,
                stroke: Color32::from_black_alpha(18),
                stroke_strong: Color32::from_black_alpha(36),
                text: Color32::from_gray(20),
                text2: Color32::from_gray(80),
                text3: Color32::from_gray(140),
                accent,
                on_accent: Color32::WHITE,
                selection: accent.gamma_multiply(0.18),
                selection_hover: accent.gamma_multiply(0.26),
                hover: Color32::from_black_alpha(10),
                pressed: Color32::from_black_alpha(6),
                folder: Color32::from_rgb(234, 175, 40),
                danger: Color32::from_rgb(196, 43, 28),
                success: Color32::from_rgb(15, 123, 15),
                warning: Color32::from_rgb(157, 93, 0),
                scrim: Color32::from_black_alpha(70),
            }
        }
    }
}

/// Mélange `c` et `with` (`t` = part de `with`).
/// Extrémités d'un dégradé autour d'une couleur : teinte décalée de part et d'autre (bleu :
/// du cyan vers l'indigo ; rouge : du magenta vers l'orange), début un peu plus lumineux.
pub fn gradient_pair(c: Color32) -> (Color32, Color32) {
    let h = egui::ecolor::Hsva::from(c);
    let shift = |d: f32, v: f32| {
        Color32::from(egui::ecolor::Hsva {
            h: (h.h + d).rem_euclid(1.0),
            v: (h.v * v).min(1.0),
            ..h
        })
    };
    (shift(-0.045, 1.12), shift(0.055, 0.96))
}

pub fn mix(c: Color32, with: Color32, t: f32) -> Color32 {
    let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(m(c.r(), with.r()), m(c.g(), with.g()), m(c.b(), with.b()))
}

fn lighten(c: Color32, t: f32) -> Color32 {
    let l = |v: u8| (f32::from(v) + (255.0 - f32::from(v)) * t) as u8;
    Color32::from_rgb(l(c.r()), l(c.g()), l(c.b()))
}

/// Couleur d'accentuation système (bleu Windows par défaut).
pub fn system_accent() -> Color32 {
    fx_win::theme::accent_color().map_or(Color32::from_rgb(0, 103, 192), |[r, g, b]| {
        Color32::from_rgb(r, g, b)
    })
}

/// Applique le choix de thème (« Système » suit le réglage Windows lu au démarrage).
pub fn apply(ctx: &egui::Context, choice: ThemeChoice, accent: Color32) {
    let dark = match choice {
        ThemeChoice::Dark => true,
        ThemeChoice::Light => false,
        ThemeChoice::System => fx_win::theme::apps_prefer_dark(),
    };
    let theme = if dark { Theme::Dark } else { Theme::Light };
    if ctx.theme() != theme {
        ctx.set_theme(theme);
    }
    let _ = accent;
}

/// Couleur d'accentuation effective : celle des réglages, sinon celle de Windows.
pub fn accent_of(settings: &fx_store::Settings, system: Color32) -> Color32 {
    settings
        .accent
        .map_or(system, |[r, g, b]| Color32::from_rgb(r, g, b))
}

/// Change la couleur d'accentuation (visuels des deux thèmes, sans toucher aux polices).
pub fn set_accent(ctx: &egui::Context, accent: Color32) {
    for (theme, dark) in [(Theme::Dark, true), (Theme::Light, false)] {
        ctx.set_visuals_of(theme, visuals(&Palette::new(dark, accent)));
    }
}

/// Couleurs d'accentuation proposées (celles de Windows).
pub const ACCENTS: [[u8; 3]; 12] = [
    [0, 120, 212],
    [0, 99, 177],
    [107, 105, 214],
    [135, 100, 184],
    [177, 70, 194],
    [227, 0, 140],
    [232, 17, 35],
    [247, 99, 12],
    [255, 185, 0],
    [16, 137, 62],
    [0, 183, 195],
    [45, 125, 154],
];

/// Installe polices, styles et visuels (une fois au démarrage).
pub fn install(ctx: &egui::Context, accent: Color32) {
    ctx.set_fonts(fonts(&[]));
    for (theme, dark) in [(Theme::Dark, true), (Theme::Light, false)] {
        ctx.set_visuals_of(theme, visuals(&Palette::new(dark, accent)));
    }
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Name(SEMIBOLD.into())),
            ),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = true;
        style.spacing.scroll.floating_allocated_width = 0.0;
        // Animations brèves : le rendu CPU paie chaque image.
        style.animation_time = 0.12;
        style.interaction.tooltip_delay = 0.5;
    });
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let r4 = CornerRadius::same(4);
    v.panel_fill = p.base;
    v.window_fill = p.card;
    v.extreme_bg_color = if p.dark {
        Color32::from_rgb(28, 28, 28)
    } else {
        Color32::WHITE
    };
    v.faint_bg_color = p.hover;
    v.window_stroke = Stroke::new(1.0, p.stroke_strong);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = egui::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 120 } else { 50 }),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }),
    };
    // Texte sélectionné (champs de saisie) : egui le dessine dans la couleur du trait de
    // sélection ; il doit contraster avec le fond d'accent, pas se confondre avec lui.
    v.selection.bg_fill = p.accent.gamma_multiply(0.55);
    v.selection.stroke = Stroke::new(
        1.0,
        if p.dark {
            Color32::WHITE
        } else {
            Color32::BLACK
        },
    );
    v.hyperlink_color = p.accent;
    v.override_text_color = Some(p.text);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text2);
    w.noninteractive.corner_radius = r4;
    // Boutons, cases à cocher, listes : cadre discret façon Fluent. Les menus d'egui
    // remettent ce fond et ce contour à zéro pour leurs entrées (`menu_style`).
    let (btn, btn_stroke) = if p.dark {
        (Color32::from_white_alpha(14), Color32::from_white_alpha(24))
    } else {
        (Color32::WHITE, Color32::from_black_alpha(34))
    };
    w.inactive.bg_fill = btn;
    w.inactive.weak_bg_fill = btn;
    w.inactive.bg_stroke = Stroke::new(1.0, btn_stroke);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r4;
    let btn_hover = if p.dark {
        Color32::from_white_alpha(22)
    } else {
        Color32::from_gray(246)
    };
    w.hovered.bg_fill = btn_hover;
    w.hovered.weak_bg_fill = btn_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, btn_stroke);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = r4;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.pressed;
    w.active.weak_bg_fill = p.pressed;
    w.active.bg_stroke = Stroke::NONE;
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = r4;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    v
}

/// Polices système : rien n'est embarqué dans l'exécutable. `extra` : polices de repli
/// chargées à la demande (CJK, symboles).
fn fonts(extra: &[(&str, &str)]) -> FontDefinitions {
    let mut defs = FontDefinitions::empty();
    let dir = fonts_dir();
    let mut load = |key: &str, files: &[&str]| -> bool {
        for f in files {
            if let Ok(bytes) = std::fs::read(dir.join(f)) {
                defs.font_data
                    .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
                return true;
            }
        }
        tracing::warn!(?files, "police système introuvable");
        false
    };
    let text = load("text", &["segoeui.ttf", "arial.ttf"]);
    let semi = load("semibold", &["seguisb.ttf", "segoeui.ttf"]);
    let mono = load("mono", &["consola.ttf", "cour.ttf"]);
    let icons = load("icons", &["SegoeIcons.ttf", "segmdl2.ttf"]);
    let mut fallbacks = Vec::new();
    for (key, file) in extra {
        if load(key, &[file]) {
            fallbacks.push((*key).to_owned());
        }
    }
    let family = |first: Option<&str>| -> Vec<String> {
        let mut v: Vec<String> = first.into_iter().map(str::to_owned).collect();
        if text {
            v.push("text".into());
        }
        v.extend(fallbacks.iter().cloned());
        v.dedup();
        v
    };
    defs.families.insert(FontFamily::Proportional, family(None));
    defs.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        family(semi.then_some("semibold")),
    );
    defs.families
        .insert(FontFamily::Monospace, family(mono.then_some("mono")));
    defs.families.insert(
        FontFamily::Name(ICONS.into()),
        family(icons.then_some("icons")),
    );
    defs
}

/// Polices de repli chargées paresseusement : seulement si un nom affiché en a besoin
/// (Microsoft YaHei pèse ~20 Mo en mémoire).
#[derive(Default)]
pub struct Fallbacks {
    cjk: bool,
    hangul: bool,
    symbols: bool,
    dirty: bool,
}

impl Fallbacks {
    /// Examine un texte affiché et note les polices manquantes.
    pub fn scan(&mut self, text: &str) {
        if text.is_ascii() {
            return;
        }
        for c in text.chars() {
            let u = c as u32;
            if !self.cjk
                && ((0x3040..=0x30FF).contains(&u)
                    || (0x3400..=0x9FFF).contains(&u)
                    || (0xF900..=0xFAFF).contains(&u))
            {
                self.cjk = true;
                self.dirty = true;
            } else if !self.hangul && (0xAC00..=0xD7AF).contains(&u) {
                self.hangul = true;
                self.dirty = true;
            } else if !self.symbols && ((0x2190..=0x2BFF).contains(&u) || u >= 0x1F000) {
                self.symbols = true;
                self.dirty = true;
            }
        }
    }

    /// Recharge les polices si un repli est devenu nécessaire.
    pub fn apply(&mut self, ctx: &egui::Context) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let mut extra = Vec::new();
        if self.cjk {
            extra.push(("cjk", "msyh.ttc"));
        }
        if self.hangul {
            extra.push(("hangul", "malgun.ttf"));
        }
        if self.symbols {
            extra.push(("symbols", "seguisym.ttf"));
        }
        ctx.set_fonts(fonts(&extra));
    }
}

fn fonts_dir() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    PathBuf::from(root).join("Fonts")
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}
