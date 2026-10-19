//! Traduction des événements clavier egui en raccourcis `fx-commands`.

use eframe::egui::{self, Event, Key as EKey, Modifiers};
use fx_commands::{HotKey, Key};

fn map_key(k: EKey) -> Option<Key> {
    use EKey::*;
    Some(match k {
        Enter => Key::Enter,
        Escape => Key::Escape,
        Tab => Key::Tab,
        Space => Key::Space,
        Backspace => Key::Backspace,
        Delete => Key::Delete,
        Insert => Key::Insert,
        Home => Key::Home,
        End => Key::End,
        PageUp => Key::PageUp,
        PageDown => Key::PageDown,
        ArrowLeft => Key::Left,
        ArrowRight => Key::Right,
        ArrowUp => Key::Up,
        ArrowDown => Key::Down,
        Plus | Equals => Key::Plus,
        Minus => Key::Minus,
        Comma => Key::Comma,
        Backtick => Key::Backtick,
        BrowserBack => Key::BrowserBack,
        F1 => Key::F(1),
        F2 => Key::F(2),
        F3 => Key::F(3),
        F4 => Key::F(4),
        F5 => Key::F(5),
        F6 => Key::F(6),
        F7 => Key::F(7),
        F8 => Key::F(8),
        F9 => Key::F(9),
        F10 => Key::F(10),
        F11 => Key::F(11),
        F12 => Key::F(12),
        Num0 => Key::Char('0'),
        Num1 => Key::Char('1'),
        Num2 => Key::Char('2'),
        Num3 => Key::Char('3'),
        Num4 => Key::Char('4'),
        Num5 => Key::Char('5'),
        Num6 => Key::Char('6'),
        Num7 => Key::Char('7'),
        Num8 => Key::Char('8'),
        Num9 => Key::Char('9'),
        other => {
            let name = other.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Key::Char(c.to_ascii_uppercase()),
                _ => return None,
            }
        }
    })
}

pub fn hotkey(key: EKey, m: Modifiers) -> Option<HotKey> {
    Some(HotKey {
        ctrl: m.ctrl || m.command,
        shift: m.shift,
        alt: m.alt,
        key: map_key(key)?,
    })
}

/// Raccourcis pressés pendant cette image (touches avec modificateur, ou touches de fonction,
/// navigation, Entrée, Suppr…). Les lettres sans modificateur sont laissées à la saisie
/// rapide (« type-ahead ») et aux champs de texte.
pub fn pressed(ctx: &egui::Context) -> Vec<HotKey> {
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                // Rangée des chiffres : touche physique (AZERTY : « 3 » sans Maj donne « " »).
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    let k = match physical_key {
                        Some(p) if is_digit(*p) => *p,
                        _ => *key,
                    };
                    hotkey(k, *modifiers)
                }
                _ => None,
            })
            .collect()
    })
}

fn is_digit(k: EKey) -> bool {
    use EKey::*;
    matches!(
        k,
        Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9
    )
}

/// Le raccourci doit-il être ignoré pendant la saisie dans un champ de texte ?
pub fn conflicts_with_typing(h: &HotKey) -> bool {
    if h.alt || (h.ctrl && !matches!(h.key, Key::Char('A' | 'C' | 'V' | 'X' | 'Z' | 'Y'))) {
        return false;
    }
    !matches!(h.key, Key::F(_) | Key::Escape)
}
