use std::fmt;

use serde::{Deserialize, Serialize};

/// Touches gérées par les raccourcis (indépendantes de la bibliothèque d'UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Char(char),
    F(u8),
    Enter,
    Escape,
    Tab,
    Space,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    Plus,
    Minus,
    Comma,
    Backtick,
    BrowserBack,
    BrowserForward,
}

/// Combinaison touche + modificateurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HotKey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: Key,
}

impl HotKey {
    pub fn new(key: Key) -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            key,
        }
    }

    /// Analyse « Ctrl+Shift+N », « F2 », « Alt+Left »…
    pub fn parse(s: &str) -> Option<Self> {
        let mut hk = Self::new(Key::Escape);
        let mut key = None;
        for part in s.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" => hk.ctrl = true,
                "shift" => hk.shift = true,
                "alt" => hk.alt = true,
                other => key = Some(parse_key(other)?),
            }
        }
        hk.key = key?;
        Some(hk)
    }
}

fn parse_key(s: &str) -> Option<Key> {
    Some(match s {
        "enter" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "plus" => Key::Plus,
        "minus" => Key::Minus,
        "comma" => Key::Comma,
        "`" | "backtick" => Key::Backtick,
        "browserback" => Key::BrowserBack,
        "browserforward" => Key::BrowserForward,
        f if f.len() >= 2 && f.starts_with('f') && f[1..].chars().all(|c| c.is_ascii_digit()) => {
            Key::F(f[1..].parse().ok()?)
        }
        c if c.chars().count() == 1 => Key::Char(c.chars().next()?.to_ascii_uppercase()),
        _ => return None,
    })
}

impl fmt::Display for HotKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.shift {
            f.write_str("Maj+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        match self.key {
            Key::Char(c) => write!(f, "{c}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Enter => f.write_str("Entrée"),
            Key::Escape => f.write_str("Échap"),
            Key::Tab => f.write_str("Tab"),
            Key::Space => f.write_str("Espace"),
            Key::Backspace => f.write_str("Retour arrière"),
            Key::Delete => f.write_str("Suppr"),
            Key::Insert => f.write_str("Inser"),
            Key::Home => f.write_str("Début"),
            Key::End => f.write_str("Fin"),
            Key::PageUp => f.write_str("Pg préc."),
            Key::PageDown => f.write_str("Pg suiv."),
            Key::Left => f.write_str("←"),
            Key::Right => f.write_str("→"),
            Key::Up => f.write_str("↑"),
            Key::Down => f.write_str("↓"),
            Key::Plus => f.write_str("+"),
            Key::Minus => f.write_str("-"),
            Key::Comma => f.write_str(","),
            Key::Backtick => f.write_str("`"),
            Key::BrowserBack => f.write_str("Précédent (souris)"),
            Key::BrowserForward => f.write_str("Suivant (souris)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        let k = HotKey::parse("Ctrl+Shift+n").expect("parse");
        assert!(k.ctrl && k.shift && !k.alt);
        assert_eq!(k.key, Key::Char('N'));
        assert_eq!(k.to_string(), "Ctrl+Maj+N");
        assert_eq!(HotKey::parse("F12").expect("f12").key, Key::F(12));
        assert!(HotKey::parse("Ctrl+").is_none());
    }
}
