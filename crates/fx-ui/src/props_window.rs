//! Fenêtre Propriétés de folder : une vraie fenêtre Windows, déplaçable hors de la fenêtre
//! principale comme celle de l'Explorateur, avec ses onglets Général, Partage, Sécurité et
//! Détails.
//!
//! Elle tourne dans un petit processus `folder.exe --properties …` (le moteur de rendu CPU ne
//! gère qu'une fenêtre par processus) qui vit le temps de la fenêtre : la fenêtre principale
//! n'est jamais ralentie par le calcul d'une taille de dossier ou la lecture des droits.
//! Toutes les lectures se font sur des threads de travail ; les réglages avancés (modifier
//! les droits, partage avancé) ouvrent la page correspondante de Windows.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, Context, FontId, Frame, Layout, Margin, RichText, ScrollArea, Sense,
    TextureHandle, Ui, ViewportBuilder, ViewportCommand, pos2, vec2,
};
use fx_core::CancelToken;
use fx_core::entry::attrs;
use fx_fs::measure::Totals;
use fx_win::details::Detail;
use fx_win::security::{Rights, Security};
use fx_win::sys::{Drive, Share, Stat};

use crate::theme::{self, Palette, glyph, icon_font, semibold};
use crate::{Error, widgets};

/// Paramètres passés par la fenêtre principale (ligne de commande).
#[derive(Debug, Clone, Default)]
pub struct PropsArgs {
    pub paths: Vec<PathBuf>,
    /// Élément de la corbeille : chemin d'origine et date de suppression.
    pub recycled: Option<(PathBuf, i64)>,
    /// Fenêtre principale, pour centrer celle-ci dessus.
    pub owner: Option<isize>,
}

/// Lit les arguments de `folder.exe --properties [--owner N] [--recycled ORIG DATE] -- CHEMINS`.
pub fn parse_args(args: &[String]) -> Result<PropsArgs, String> {
    let mut out = PropsArgs::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--owner" => {
                out.owner = it.next().and_then(|v| v.parse().ok());
            }
            "--recycled" => {
                let orig = it.next().ok_or("--recycled attend un chemin")?;
                let date = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--recycled attend une date")?;
                out.recycled = Some((PathBuf::from(orig), date));
            }
            "--" => out.paths.extend(it.by_ref().map(PathBuf::from)),
            other => out.paths.push(PathBuf::from(other)),
        }
    }
    if out.paths.is_empty() {
        return Err("aucun élément".into());
    }
    Ok(out)
}

/// Résultats des lectures en arrière-plan.
#[derive(Default)]
struct Data {
    icon: Option<fx_win::image::Rgba>,
    stat: Option<Stat>,
    totals: Option<Totals>,
    complete: bool,
    open_with: Option<String>,
    drive: Option<Drive>,
    shares: Vec<Share>,
    security: Option<Result<Security, String>>,
    details: Option<Result<Vec<Detail>, String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Sharing,
    Security,
    Details,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::General => "Général",
            Self::Sharing => "Partage",
            Self::Security => "Sécurité",
            Self::Details => "Détails",
        }
    }
}

struct PropsApp {
    args: PropsArgs,
    title: String,
    data: Arc<Mutex<Data>>,
    cancel: CancelToken,
    tab: Tab,
    pal: Palette,
    icon: Option<TextureHandle>,
    /// Nom en cours d'édition et attributs cochés (`None` : pas encore initialisés).
    name: String,
    attrs: Option<(bool, bool)>,
    account: usize,
    shown: bool,
    hwnd: Option<isize>,
    status: Option<String>,
    wheel: crate::wheel::WheelGlide,
}

/// Ouvre la fenêtre Propriétés et rend la main à sa fermeture.
pub fn run(args: PropsArgs) -> Result<(), Error> {
    let shown = args
        .recycled
        .as_ref()
        .map_or_else(|| args.paths[0].clone(), |(o, _)| o.clone());
    let title = if args.paths.len() == 1 {
        format!("Propriétés de {}", name_of(&shown))
    } else {
        format!("Propriétés de {} éléments", args.paths.len())
    };
    let viewport = ViewportBuilder::default()
        .with_title(&title)
        .with_app_id("folder-properties")
        .with_inner_size([470.0, 600.0])
        .with_min_inner_size([400.0, 460.0])
        // Créée cachée : placée au-dessus de la fenêtre principale puis affichée.
        .with_visible(false);
    let config = egui_software_backend::SoftwareBackendAppConfiguration::new()
        .viewport_builder(viewport)
        .caching(false);
    egui_software_backend::set_clear_each_frame(false);
    egui_software_backend::set_frame_pacing(true);
    egui_software_backend::run_app_with_software_backend(config, move |ctx| {
        PropsApp::new(&ctx, args.clone(), title.clone())
    })
    .map_err(|e| e.to_string().into())
}

impl PropsApp {
    fn new(ctx: &Context, args: PropsArgs, title: String) -> Self {
        let settings = fx_store::load_settings(&fx_store::Paths::standard());
        let accent = theme::accent_of(&settings, theme::system_accent());
        theme::install(ctx, accent);
        theme::apply(ctx, settings.theme, accent);
        let pal = Palette::new(ctx.theme() == egui::Theme::Dark, accent);
        let data = Arc::new(Mutex::new(Data::default()));
        let cancel = CancelToken::new();
        start_loading(ctx, &args, &data, &cancel);
        let name = args
            .recycled
            .as_ref()
            .map_or_else(|| name_of(&args.paths[0]), |(o, _)| name_of(o));
        Self {
            args,
            title,
            data,
            cancel,
            tab: Tab::General,
            pal,
            icon: None,
            name,
            attrs: None,
            account: 0,
            shown: false,
            hwnd: None,
            status: None,
            wheel: crate::wheel::WheelGlide::default(),
        }
    }

    fn single(&self) -> bool {
        self.args.paths.len() == 1
    }

    fn path(&self) -> &Path {
        &self.args.paths[0]
    }

    fn is_root(&self) -> bool {
        self.single() && self.args.recycled.is_none() && self.path().parent().is_none()
    }

    fn tabs(&self, d: &Data) -> Vec<Tab> {
        let mut tabs = vec![Tab::General];
        if !self.single() || self.args.recycled.is_some() {
            return tabs;
        }
        let is_dir = d.stat.is_some_and(|s| s.attributes & attrs::DIRECTORY != 0);
        if is_dir || self.is_root() {
            tabs.push(Tab::Sharing);
        }
        tabs.push(Tab::Security);
        if !self.is_root() {
            tabs.push(Tab::Details);
        }
        tabs
    }

    /// Premier affichage : au centre de la fenêtre principale, barre de titre au thème.
    fn show(&mut self, ctx: &Context) {
        if self.shown {
            return;
        }
        let Some(hwnd) = fx_win::window::main_window(&self.title) else {
            return;
        };
        self.shown = true;
        self.hwnd = Some(hwnd);
        fx_win::window::set_app_icon(hwnd);
        fx_win::window::set_dark_title(hwnd, self.pal.dark);
        let placed = self
            .args
            .owner
            .and_then(fx_win::window::rect_of)
            .zip(fx_win::window::rect_of(hwnd))
            .is_some_and(|((ol, ot, or, ob), (l, t, r, b))| {
                let (w, h) = (r - l, b - t);
                let (cx, cy) = ((ol + or) / 2, (ot + ob) / 2);
                let left = cx - w / 2 + 24;
                let top = (cy - h / 2).max(ot);
                fx_win::window::show_with_placement(
                    hwnd,
                    &fx_win::window::Placement {
                        left,
                        top,
                        right: left + w,
                        bottom: top + h,
                        maximized: false,
                    },
                )
            });
        let _ = placed;
        // Comme pour la fenêtre principale : winit doit connaître l'état réel.
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }

    /// Applique le nouveau nom et les attributs ; vrai si tout s'est bien passé.
    fn apply(&mut self) -> bool {
        let (stat, editable) = {
            let d = self.data.lock().expect("données");
            (d.stat, self.editable())
        };
        if !editable {
            return true;
        }
        let path = self.path().to_path_buf();
        if let (Some(st), Some((ro, hidden))) = (stat, self.attrs) {
            let set = |a: u32, bit: u32, on: bool| if on { a | bit } else { a & !bit };
            let wanted = set(
                set(st.attributes, attrs::READONLY, ro),
                attrs::HIDDEN,
                hidden,
            );
            if wanted != st.attributes {
                if let Err(e) = fx_win::sys::set_attributes(&path, wanted) {
                    self.status = Some(format!("Attributs non modifiés : {}", e.message().trim()));
                    return false;
                }
                self.data.lock().expect("données").stat = fx_win::sys::stat(&path);
            }
        }
        let new_name = self.name.trim().to_owned();
        if !new_name.is_empty() && new_name != name_of(&path) {
            // Renommage par IFileOperation (corbeille, UAC, conflits), sur un thread STA.
            let item = path.clone();
            let name = new_name.clone();
            let result = std::thread::spawn(move || {
                let _com = fx_win::com::init_sta();
                fx_win::fileop::perform(
                    &fx_win::fileop::FileOp::Rename {
                        item,
                        new_name: name,
                    },
                    None,
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    |_| {},
                )
            })
            .join();
            match result {
                Ok(Ok(o)) if !o.aborted => {
                    if let Some(parent) = path.parent() {
                        self.args.paths[0] = parent.join(&new_name);
                    }
                }
                Ok(Err(e)) => {
                    self.status = Some(format!("Renommage impossible : {}", e.message().trim()));
                    return false;
                }
                _ => return false,
            }
        }
        true
    }

    fn editable(&self) -> bool {
        self.single() && self.args.recycled.is_none() && !self.is_root()
    }
}

impl egui_software_backend::App for PropsApp {
    fn ui(&mut self, ui: &mut Ui, _backend: &mut egui_software_backend::SoftwareBackend) {
        let ctx = ui.ctx().clone();
        let pal = self.pal;
        self.wheel.frame(&ctx);
        if self.icon.is_none()
            && let Some(img) = self.data.lock().expect("données").icon.take()
        {
            let image = egui::ColorImage::from_rgba_premultiplied(
                [img.width as usize, img.height as usize],
                &img.pixels,
            );
            self.icon = Some(ctx.load_texture("icon", image, egui::TextureOptions::LINEAR));
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        egui::Panel::bottom("buttons")
            .frame(
                Frame::new()
                    .fill(pal.base)
                    .inner_margin(Margin::symmetric(14, 12)),
            )
            .show_separator_line(false)
            .show(ui, |ui| self.buttons(ui));
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(pal.layer)
                    .inner_margin(Margin::symmetric(18, 10)),
            )
            .show(ui, |ui| {
                let tabs = {
                    let d = self.data.lock().expect("données");
                    self.tabs(&d)
                };
                if !tabs.contains(&self.tab) {
                    self.tab = Tab::General;
                }
                self.tab_bar(ui, &tabs);
                ui.add_space(10.0);
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        Tab::General => self.general(ui),
                        Tab::Sharing => self.sharing(ui),
                        Tab::Security => self.security(ui),
                        Tab::Details => self.details(ui),
                    });
            });
        self.show(&ctx);
    }

    fn on_exit(&mut self, _ctx: &Context) {
        self.cancel.cancel();
    }
}

// --------------------------------------------------------------------------------------
// Onglets

impl PropsApp {
    /// Onglets façon Windows 11 : texte, indicateur d'accent qui glisse sous l'onglet actif.
    fn tab_bar(&mut self, ui: &mut Ui, tabs: &[Tab]) {
        let pal = self.pal;
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
        let mut x = bar.left();
        let mut active_span = None;
        for &t in tabs {
            let font = if t == self.tab {
                semibold(14.0)
            } else {
                FontId::proportional(14.0)
            };
            let galley = ui
                .painter()
                .layout_no_wrap(t.label().to_owned(), font, pal.text);
            let w = galley.size().x + 24.0;
            let rect = egui::Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height()));
            let r = ui.interact(rect, egui::Id::unique(("ptab", t.label())), Sense::click());
            widgets::hover_fill(ui, &r, rect.shrink2(vec2(2.0, 4.0)), &pal, 6.0);
            let color = if t == self.tab { pal.text } else { pal.text2 };
            ui.painter().galley(
                pos2(
                    rect.center().x - galley.size().x / 2.0,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                color,
            );
            if r.clicked() {
                self.tab = t;
            }
            if t == self.tab {
                active_span = Some((rect.center().x, w - 28.0));
            }
            x += w;
        }
        ui.painter().hline(
            bar.x_range(),
            bar.bottom() - 0.5,
            egui::Stroke::new(1.0, pal.stroke),
        );
        if let Some((cx, w)) = active_span {
            let id = egui::Id::unique("ptab-indicator");
            let ax = ui.ctx().animate_value_with_time(id.with("x"), cx, 0.18);
            let aw = ui.ctx().animate_value_with_time(id.with("w"), w, 0.18);
            let line = egui::Rect::from_center_size(pos2(ax, bar.bottom() - 2.0), vec2(aw, 3.0));
            ui.painter().rect_filled(line, 1.5, pal.accent);
        }
    }

    fn general(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let (stat, totals, complete, open_with, drive) = {
            let d = self.data.lock().expect("données");
            (
                d.stat,
                d.totals,
                d.complete,
                d.open_with.clone(),
                d.drive.clone(),
            )
        };
        let single = self.single();
        let is_root = self.is_root();
        let is_dir = stat.is_some_and(|s| s.attributes & attrs::DIRECTORY != 0);
        let editable = self.editable();

        // Icône et nom (modifiable, comme dans l'Explorateur).
        ui.horizontal(|ui| {
            let (slot, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            match &self.icon {
                Some(tex) => {
                    ui.painter().image(
                        tex.id(),
                        egui::Rect::from_center_size(slot.center(), vec2(36.0, 36.0)),
                        egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    let g = if !single {
                        glyph::COPY
                    } else if is_dir || is_root {
                        glyph::FOLDER
                    } else {
                        glyph::FILE
                    };
                    ui.painter().text(
                        slot.center(),
                        egui::Align2::CENTER_CENTER,
                        g,
                        icon_font(28.0),
                        pal.folder,
                    );
                }
            }
            ui.add_space(8.0);
            if editable {
                ui.add(
                    egui::TextEdit::singleline(&mut self.name)
                        .font(FontId::proportional(15.0))
                        .desired_width(f32::INFINITY),
                );
            } else {
                let title = if single {
                    self.name.clone()
                } else {
                    format!("{} éléments", self.args.paths.len())
                };
                ui.label(RichText::new(title).font(semibold(16.0)));
            }
        });
        separator(ui, &pal);

        grid(ui, "general-1", |ui| {
            let kind = if !single {
                "Plusieurs éléments".to_owned()
            } else if is_root {
                "Disque local".to_owned()
            } else if stat.is_none() {
                "…".to_owned()
            } else {
                let shown = self
                    .args
                    .recycled
                    .as_ref()
                    .map_or_else(|| self.path().to_path_buf(), |(o, _)| o.clone());
                let ext = shown
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                crate::view::type_name(is_dir, ext.as_bytes())
            };
            field(ui, &pal, "Type", &kind);
            if single && !is_dir && !is_root && self.args.recycled.is_none() {
                ui.label(RichText::new("S'ouvre avec").color(pal.text2));
                ui.horizontal(|ui| {
                    ui.label(open_with.as_deref().unwrap_or("Aucune application"));
                    if ui.button("Modifier…").clicked() {
                        let path = self.path().to_path_buf();
                        let owner = self.hwnd;
                        std::thread::spawn(move || {
                            let _com = fx_win::com::init_sta();
                            let _ = fx_win::shell::open_with(owner, &path);
                        });
                    }
                });
                ui.end_row();
            }
        });
        separator(ui, &pal);

        grid(ui, "general-2", |ui| {
            if let Some((orig, deleted)) = &self.args.recycled {
                let origin = orig
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                field(ui, &pal, "Emplacement d'origine", &origin);
                field(ui, &pal, "Supprimé le", &date(*deleted));
            } else if !is_root {
                let parent = self
                    .path()
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                field(ui, &pal, "Emplacement", &parent);
            }
            if !is_root {
                ui.label(RichText::new("Taille").color(pal.text2));
                ui.horizontal(|ui| {
                    match totals {
                        Some(t) => ui.label(size_text(t.size)),
                        None => ui.label(RichText::new("Calcul…").color(pal.text2)),
                    };
                    if !complete {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                });
                ui.end_row();
                if let Some(t) = totals.filter(|t| t.folders > 0 || !single) {
                    let folders = if single && is_dir {
                        t.folders.saturating_sub(1)
                    } else {
                        t.folders
                    };
                    field(
                        ui,
                        &pal,
                        "Contenu",
                        &format!(
                            "{} fichier{}, {} dossier{}",
                            group(t.files),
                            plural(t.files),
                            group(folders),
                            plural(folders)
                        ),
                    );
                }
            }
        });

        if let Some(drive) = drive.filter(|d| d.total > 0) {
            separator(ui, &pal);
            let used = drive.total - drive.free;
            grid(ui, "drive", |ui| {
                field(ui, &pal, "Espace utilisé", &size_text(used));
                field(ui, &pal, "Espace libre", &size_text(drive.free));
                field(ui, &pal, "Capacité", &size_text(drive.total));
            });
            ui.add_space(10.0);
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
            widgets::progress(
                ui,
                bar,
                Some(used as f32 / drive.total as f32),
                pal.accent,
                pal.stroke_strong,
            );
        }

        if let Some(st) = stat.filter(|_| single) {
            separator(ui, &pal);
            grid(ui, "dates", |ui| {
                field(ui, &pal, "Créé le", &date(st.created));
                field(ui, &pal, "Modifié le", &date(st.modified));
            });
            if editable {
                separator(ui, &pal);
                let (mut ro, mut hidden) = self.attrs.unwrap_or((
                    st.attributes & attrs::READONLY != 0,
                    st.attributes & attrs::HIDDEN != 0,
                ));
                grid(ui, "attrs", |ui| {
                    ui.label(RichText::new("Attributs").color(pal.text2));
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut ro, "Lecture seule");
                        ui.add_space(8.0);
                        ui.checkbox(&mut hidden, "Caché");
                    });
                    ui.end_row();
                });
                self.attrs = Some((ro, hidden));
            }
        }
    }

    fn sharing(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let path = self.path().to_path_buf();
        let shares: Vec<Share> = self
            .data
            .lock()
            .expect("données")
            .shares
            .iter()
            .filter(|s| s.path.as_os_str().eq_ignore_ascii_case(path.as_os_str()))
            .cloned()
            .collect();
        section(ui, &pal, "Partage de fichiers et de dossiers en réseau");
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(glyph::NETWORK)
                    .font(icon_font(22.0))
                    .color(pal.text2),
            );
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(name_of(&path)).strong());
                ui.label(
                    RichText::new(if shares.is_empty() {
                        "Non partagé"
                    } else {
                        "Partagé"
                    })
                    .color(pal.text2),
                );
            });
        });
        ui.add_space(8.0);
        if !shares.is_empty() {
            let host = fx_win::sys::computer_name();
            grid(ui, "shares", |ui| {
                for s in &shares {
                    field(ui, &pal, "Chemin réseau", &format!(r"\\{host}\{}", s.name));
                }
            });
            ui.add_space(8.0);
        }
        ui.label(
            RichText::new(
                "Le partage avancé définit les autorisations réseau, le nombre d'utilisateurs \
                 simultanés et les noms de partage.",
            )
            .color(pal.text2),
        );
        ui.add_space(8.0);
        if ui.button("Partage avancé…").clicked() {
            self.native_page("Partage");
        }
    }

    fn security(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let security = self.data.lock().expect("données").security.clone();
        grid(ui, "sec-object", |ui| {
            field(
                ui,
                &pal,
                "Nom de l'objet",
                &self.path().display().to_string(),
            );
            if let Some(Ok(s)) = &security {
                field(
                    ui,
                    &pal,
                    "Propriétaire",
                    s.owner.as_deref().unwrap_or("Inconnu"),
                );
            }
        });
        ui.add_space(8.0);
        let entries = match &security {
            None => {
                ui.spinner();
                return;
            }
            Some(Err(e)) => {
                ui.label(RichText::new(format!("Droits illisibles : {e}")).color(pal.danger));
                return;
            }
            Some(Ok(s)) => s.entries.clone(),
        };
        section(ui, &pal, "Noms de groupes ou d'utilisateurs");
        Frame::new()
            .fill(pal.card)
            .stroke(egui::Stroke::new(1.0, pal.stroke))
            .corner_radius(6)
            .inner_margin(Margin::same(4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (i, e) in entries.iter().enumerate() {
                    let selected = i == self.account;
                    let (rect, r) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                    if selected {
                        ui.painter().rect_filled(rect, 4.0, pal.selection);
                    } else {
                        widgets::hover_fill(ui, &r, rect, &pal, 4.0);
                    }
                    ui.painter().text(
                        pos2(rect.left() + 14.0, rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        glyph::PERSON,
                        icon_font(13.0),
                        pal.text2,
                    );
                    ui.painter().text(
                        pos2(rect.left() + 30.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &e.account,
                        FontId::proportional(13.0),
                        pal.text,
                    );
                    if r.clicked() {
                        self.account = i;
                    }
                }
            });
        ui.add_space(10.0);
        let Some(e) = entries.get(self.account.min(entries.len().saturating_sub(1))) else {
            return;
        };
        section(
            ui,
            &pal,
            &format!("Autorisations pour {}", short_account(&e.account)),
        );
        egui::Grid::new("perms")
            .num_columns(3)
            .spacing([28.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(RichText::new("Autoriser").color(pal.text2));
                ui.label(RichText::new("Refuser").color(pal.text2));
                ui.end_row();
                let rows: [(&str, RightOf); 5] = [
                    ("Contrôle total", |r| r.full),
                    ("Modification", |r| r.modify),
                    ("Lecture et exécution", |r| r.read_execute),
                    ("Lecture", |r| r.read),
                    ("Écriture", |r| r.write),
                ];
                for (label, get) in rows {
                    ui.label(label);
                    for (on, color) in [(get(e.allow), pal.accent), (get(e.deny), pal.danger)] {
                        let g = if on { glyph::CHECK } else { "" };
                        ui.label(RichText::new(g).font(icon_font(13.0)).color(color));
                    }
                    ui.end_row();
                }
            });
        if e.inherited {
            ui.add_space(4.0);
            ui.label(
                RichText::new("Autorisations héritées du dossier parent.")
                    .size(12.0)
                    .color(pal.text3),
            );
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Modifier…").clicked() {
                self.native_page("Sécurité");
            }
            ui.label(
                RichText::new("Modification des droits et paramètres avancés (Windows).")
                    .size(12.0)
                    .color(pal.text3),
            );
        });
    }

    fn details(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let details = self.data.lock().expect("données").details.clone();
        match details {
            None => {
                ui.spinner();
            }
            Some(Err(e)) => {
                ui.label(RichText::new(format!("Détails indisponibles : {e}")).color(pal.text2));
            }
            Some(Ok(rows)) if rows.is_empty() => {
                ui.label(RichText::new("Aucun détail pour cet élément.").color(pal.text2));
            }
            Some(Ok(rows)) => {
                let mut group_rows: Vec<DetailGroup> = Vec::new();
                for r in rows {
                    match r {
                        Detail::Group(g) => group_rows.push((Some(g), Vec::new())),
                        Detail::Value { name, value } => {
                            if group_rows.is_empty() {
                                group_rows.push((None, Vec::new()));
                            }
                            if let Some(last) = group_rows.last_mut() {
                                last.1.push((name, value));
                            }
                        }
                    }
                }
                for (i, (title, values)) in group_rows.into_iter().enumerate() {
                    if let Some(t) = title {
                        section(ui, &pal, &t);
                    }
                    grid(ui, &format!("details-{i}"), |ui| {
                        for (n, v) in &values {
                            field(ui, &pal, n, v);
                        }
                    });
                    ui.add_space(8.0);
                }
            }
        }
    }

    fn native_page(&self, page: &str) {
        let path = self.path().to_path_buf();
        let owner = self.hwnd;
        let page = page.to_owned();
        std::thread::spawn(move || {
            let _com = fx_win::com::init_sta();
            let _ = fx_win::shell::properties_page(owner, &path, &page);
            // Fenêtre non modale : le thread doit vivre tant qu'elle est ouverte.
            std::thread::sleep(Duration::from_secs(3600));
        });
    }

    fn buttons(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        ui.horizontal(|ui| {
            if let Some(s) = &self.status {
                ui.label(RichText::new(s).size(12.0).color(pal.danger));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        self.editable(),
                        egui::Button::new("Appliquer").min_size(vec2(90.0, 30.0)),
                    )
                    .clicked()
                {
                    self.apply();
                }
                if ui
                    .add(egui::Button::new("Annuler").min_size(vec2(90.0, 30.0)))
                    .clicked()
                {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
                let ok = ui.add(
                    egui::Button::new(RichText::new("OK").color(Color32::WHITE))
                        .fill(pal.accent)
                        .min_size(vec2(90.0, 30.0)),
                );
                if ok.clicked() && self.apply() {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                }
            });
        });
    }
}

// --------------------------------------------------------------------------------------
// Lectures en arrière-plan

fn start_loading(ctx: &Context, args: &PropsArgs, data: &Arc<Mutex<Data>>, cancel: &CancelToken) {
    let paths = args.paths.clone();
    let single = paths.len() == 1 && args.recycled.is_none();
    // Lectures courtes et appels shell (icône, droits, détails) : un thread STA.
    {
        let (ctx, data, paths) = (ctx.clone(), Arc::clone(data), paths.clone());
        std::thread::spawn(move || {
            let _com = fx_win::com::init_sta();
            let path = &paths[0];
            let set = |f: &dyn Fn(&mut Data)| {
                if let Ok(mut d) = data.lock() {
                    f(&mut d);
                }
                ctx.request_repaint();
            };
            let stat = if paths.len() == 1 {
                fx_win::sys::stat(path)
            } else {
                None
            };
            set(&|d| d.stat = stat);
            if paths.len() == 1 {
                let icon =
                    fx_win::image::shell_image(path, 48, fx_win::image::ImageKind::Icon).ok();
                set(&|d| d.icon = icon.clone());
            }
            if !single {
                return;
            }
            if path.parent().is_none() {
                let drive = fx_win::sys::drives().into_iter().find(|d| d.root == *path);
                set(&|d| d.drive = drive.clone());
            }
            if let Some(ext) = path.extension() {
                let app = fx_win::shell::default_app_name(&ext.to_string_lossy());
                set(&|d| d.open_with = app.clone());
            }
            let shares = fx_win::sys::shares();
            set(&|d| d.shares = shares.clone());
            let security = fx_win::security::read(path).map_err(|e| e.message().trim().to_owned());
            set(&|d| d.security = Some(security.clone()));
            let details = fx_win::details::read(path).map_err(|e| e.message().trim().to_owned());
            set(&|d| d.details = Some(details.clone()));
        });
    }
    // Taille et contenu : parcours récursif, résultats partiels réguliers.
    {
        let (ctx, data, cancel) = (ctx.clone(), Arc::clone(data), cancel.clone());
        std::thread::spawn(move || {
            let c = ctx.clone();
            let d2 = Arc::clone(&data);
            let totals =
                fx_fs::measure::measure(&paths, &cancel, Duration::from_millis(150), |t| {
                    if let Ok(mut d) = d2.lock() {
                        d.totals = Some(t);
                    }
                    c.request_repaint();
                });
            if let (Some(t), Ok(mut d)) = (totals, data.lock()) {
                d.totals = Some(t);
                d.complete = true;
            }
            ctx.request_repaint();
        });
    }
}

// --------------------------------------------------------------------------------------
// Présentation

const LABEL_W: f32 = 150.0;

/// Lecture d'un droit dans [`Rights`] (ligne du tableau des autorisations).
type RightOf = fn(Rights) -> bool;

/// Section de l'onglet Détails : titre éventuel et couples (nom, valeur).
type DetailGroup = (Option<String>, Vec<(String, String)>);

fn grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .min_col_width(LABEL_W)
        .spacing([16.0, 9.0])
        .show(ui, add);
}

fn field(ui: &mut Ui, pal: &Palette, label: &str, value: &str) {
    ui.label(RichText::new(label).color(pal.text2));
    ui.add(egui::Label::new(value).wrap());
    ui.end_row();
}

fn section(ui: &mut Ui, pal: &Palette, title: &str) {
    ui.add_space(2.0);
    ui.label(RichText::new(title).font(semibold(13.5)).color(pal.text));
    ui.add_space(4.0);
}

fn separator(ui: &mut Ui, pal: &Palette) {
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        r.x_range(),
        r.center().y,
        egui::Stroke::new(1.0, pal.stroke),
    );
    ui.add_space(8.0);
}

fn short_account(a: &str) -> &str {
    a.rsplit('\\').next().unwrap_or(a)
}

fn name_of(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn plural(n: u64) -> &'static str {
    if n > 1 { "s" } else { "" }
}

/// Nombre avec séparateur de milliers (espace fine insécable).
fn group(n: u64) -> String {
    let mut s = String::new();
    let _ = fx_core::format::thousands(&mut s, n);
    s
}

/// « 12,4 Mo (13 012 345 octets) ».
fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} octet{}", plural(bytes));
    }
    let mut s = String::new();
    let _ = fx_core::format::size_human(&mut s, bytes);
    format!("{s} ({} octets)", group(bytes))
}

fn date(filetime: i64) -> String {
    fx_win::time::filetime_to_local(filetime).map_or_else(
        || "—".to_owned(),
        |t| {
            format!(
                "{:02}/{:02}/{} à {:02}:{:02}",
                t.day, t.month, t.year, t.hour, t.minute
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_line() {
        let a: Vec<String> = [
            "--owner",
            "1234",
            "--recycled",
            r"C:\a\b.txt",
            "99",
            "--",
            r"C:\x",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let p = parse_args(&a).expect("arguments");
        assert_eq!(p.owner, Some(1234));
        assert_eq!(p.recycled, Some((PathBuf::from(r"C:\a\b.txt"), 99)));
        assert_eq!(p.paths, vec![PathBuf::from(r"C:\x")]);
        assert!(parse_args(&[]).is_err());
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(size_text(6), "6 octets");
        assert_eq!(size_text(1536), "1,5 Ko (1\u{202F}536 octets)");
    }
}
