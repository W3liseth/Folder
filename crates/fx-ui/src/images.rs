//! Cache de textures d'icônes et de miniatures.
//!
//! Les images sont produites par le thread shell (`fx-shell`) et converties en textures egui
//! sur le thread UI. Le cache est borné en octets (LRU) : avec le rendu CPU, les textures
//! vivent en mémoire système et comptent dans le budget.

use std::collections::HashMap;
use std::path::PathBuf;

use crossbeam_channel::Receiver;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use fx_shell::{ImageKey, ImageRequest, Rgba, ShellWorker};

/// Budget des icônes (petites, nombreuses) et des miniatures (grandes).
const ICON_BUDGET: usize = 8 * 1024 * 1024;
const THUMB_BUDGET: usize = 32 * 1024 * 1024;

enum Slot {
    Pending,
    Ready {
        tex: TextureHandle,
        bytes: usize,
        used: u64,
    },
    Failed,
}

pub struct ImageCache {
    slots: HashMap<ImageKey, Slot>,
    rx: Receiver<(ImageKey, Option<Rgba>)>,
    frame: u64,
    icon_bytes: usize,
    thumb_bytes: usize,
}

impl ImageCache {
    pub fn new(rx: Receiver<(ImageKey, Option<Rgba>)>) -> Self {
        Self {
            slots: HashMap::new(),
            rx,
            frame: 0,
            icon_bytes: 0,
            thumb_bytes: 0,
        }
    }

    /// Intègre les images arrivées depuis la dernière image ; à appeler une fois par image.
    pub fn pump(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let mut got_any = false;
        while let Ok((key, img)) = self.rx.try_recv() {
            got_any = true;
            let slot = match img {
                Some(img) if img.width > 0 && img.height > 0 => {
                    let size = [img.width as usize, img.height as usize];
                    let color = ColorImage::from_rgba_premultiplied(size, &img.pixels);
                    let bytes = img.pixels.len();
                    let tex = ctx.load_texture(
                        format!("img{}", self.frame),
                        color,
                        TextureOptions::LINEAR,
                    );
                    if matches!(key, ImageKey::Thumb { .. }) {
                        self.thumb_bytes += bytes;
                    } else {
                        self.icon_bytes += bytes;
                    }
                    Slot::Ready {
                        tex,
                        bytes,
                        used: self.frame,
                    }
                }
                _ => Slot::Failed,
            };
            self.slots.insert(key, slot);
        }
        if got_any {
            self.evict();
        }
    }

    /// Texture pour `key`, demandée au thread shell si absente.
    pub fn get(
        &mut self,
        shell: &ShellWorker,
        key: &ImageKey,
        source: &std::path::Path,
    ) -> Option<&TextureHandle> {
        let frame = self.frame;
        match self.slots.get_mut(key) {
            Some(Slot::Ready { used, .. }) => *used = frame,
            Some(Slot::Pending | Slot::Failed) => return None,
            None => {
                self.slots.insert(key.clone(), Slot::Pending);
                shell.request_image(ImageRequest {
                    key: key.clone(),
                    path: PathBuf::from(source),
                    size: u32::from(key.size()),
                    kind: key.kind(),
                });
                return None;
            }
        }
        match self.slots.get(key) {
            Some(Slot::Ready { tex, .. }) => Some(tex),
            _ => None,
        }
    }

    /// Une miniature a-t-elle échoué (on affiche alors l'icône) ?
    pub fn failed(&self, key: &ImageKey) -> bool {
        matches!(self.slots.get(key), Some(Slot::Failed))
    }

    /// Oublie les demandes en attente (changement de dossier) : elles seront refaites si besoin.
    pub fn forget_pending(&mut self, shell: &ShellWorker) {
        shell.clear_images();
        self.slots.retain(|_, s| !matches!(s, Slot::Pending));
    }

    pub fn bytes(&self) -> (usize, usize) {
        (self.icon_bytes, self.thumb_bytes)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    fn evict(&mut self) {
        for thumbs in [false, true] {
            let budget = if thumbs { THUMB_BUDGET } else { ICON_BUDGET };
            let total = if thumbs {
                self.thumb_bytes
            } else {
                self.icon_bytes
            };
            if total <= budget {
                continue;
            }
            let mut candidates: Vec<(u64, ImageKey, usize)> = self
                .slots
                .iter()
                .filter(|(k, _)| matches!(k, ImageKey::Thumb { .. }) == thumbs)
                .filter_map(|(k, s)| match s {
                    Slot::Ready { used, bytes, .. } => Some((*used, k.clone(), *bytes)),
                    _ => None,
                })
                .collect();
            candidates.sort_by_key(|c| c.0);
            let mut freed = 0;
            for (used, key, bytes) in candidates {
                // Ne jamais évincer ce qui a servi à l'image courante.
                if total - freed <= budget * 3 / 4 || used >= self.frame {
                    break;
                }
                self.slots.remove(&key);
                freed += bytes;
            }
            if thumbs {
                self.thumb_bytes -= freed;
            } else {
                self.icon_bytes -= freed;
            }
        }
    }
}
