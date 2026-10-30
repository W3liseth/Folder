use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{PCWSTR, w};

/// `true` si l'utilisateur a choisi le mode sombre pour les applications.
pub fn apps_prefer_dark() -> bool {
    read_dword(
        w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w!("AppsUseLightTheme"),
    )
    .is_some_and(|light| light == 0)
}

/// `true` si les « Effets de transparence » de Windows sont actifs (Personnalisation ›
/// Couleurs) ; actifs si la valeur manque, comme pour Windows.
pub fn transparency_enabled() -> bool {
    read_dword(
        w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
        w!("EnableTransparency"),
    )
    .is_none_or(|on| on != 0)
}

/// Couleur d'accentuation système en RGB, si disponible.
pub fn accent_color() -> Option<[u8; 3]> {
    // Stockée en ABGR.
    let abgr = read_dword(w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor"))?;
    Some([abgr as u8, (abgr >> 8) as u8, (abgr >> 16) as u8])
}

fn read_dword(subkey: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `subkey`/`value` sont des littéraux terminés par NUL ; `data` est un tampon
    // de `size` octets, conforme à RRF_RT_REG_DWORD.
    let err = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&raw mut size),
        )
    };
    err.is_ok().then_some(data)
}
