//! Raccourcis système (Windows + E, touche d'ouverture de folder) : règles communes au
//! scheduler, qui les applique, et aux paramètres, qui les valident.

use fx_commands::{HotKey, Key};
use fx_win::shortcuts::LaunchHotkey;

/// Touche de raccourci Windows correspondant à `hk`, si l'Explorateur l'accepte : lettre,
/// chiffre ou F1–F12, avec au moins deux modificateurs parmi Ctrl, Alt et Maj.
pub fn launch_hotkey(hk: &HotKey) -> Option<LaunchHotkey> {
    let mods = u8::from(hk.ctrl) + u8::from(hk.alt) + u8::from(hk.shift);
    if mods < 2 {
        return None;
    }
    let vk = match hk.key {
        Key::Char(c) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u8,
        // VK_F1 = 0x70.
        Key::F(n) if (1..=12).contains(&n) => 0x70 + n - 1,
        _ => return None,
    };
    Some(LaunchHotkey {
        vk,
        ctrl: hk.ctrl,
        alt: hk.alt,
        shift: hk.shift,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_hotkeys() {
        let p = |s: &str| HotKey::parse(s).and_then(|h| launch_hotkey(&h));
        assert_eq!(p("Ctrl+Alt+F").map(|h| h.vk), Some(b'F'));
        assert_eq!(p("Ctrl+Shift+3").map(|h| h.vk), Some(b'3'));
        assert_eq!(p("Alt+Shift+F5").map(|h| h.vk), Some(0x74));
        assert!(p("Ctrl+F").is_none(), "un seul modificateur");
        assert!(
            p("Ctrl+Alt+Enter").is_none(),
            "touche non gérée par Windows"
        );
        assert!(p("Ctrl+Alt+F13").is_none());
    }
}
