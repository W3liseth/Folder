use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// Date et heure locales, pour l'affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Heure courante en FILETIME UTC brut.
pub fn now_filetime() -> i64 {
    // SAFETY: pas de précondition ; renvoie la structure par valeur.
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)) as i64
}

/// Convertit un FILETIME UTC brut en heure locale (règles DST de la date concernée).
pub fn filetime_to_local(ft: i64) -> Option<LocalTime> {
    if ft <= 0 {
        return None;
    }
    let ft = FILETIME {
        dwLowDateTime: ft as u32,
        dwHighDateTime: (ft >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: pointeurs vers des structures locales valides.
    unsafe {
        FileTimeToSystemTime(&raw const ft, &raw mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &raw const utc, &raw mut local).ok()?;
    }
    Some(LocalTime {
        year: local.wYear,
        month: local.wMonth as u8,
        day: local.wDay as u8,
        hour: local.wHour as u8,
        minute: local.wMinute as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_a_known_date() {
        // 2024-06-15 12:00 UTC ; l'heure locale dépend du fuseau, la date presque jamais.
        let t = filetime_to_local(133_628_976_000_000_000).expect("time");
        assert_eq!((t.year, t.month), (2024, 6));
        assert!((14..=16).contains(&t.day));
    }

    #[test]
    fn zero_is_none() {
        assert_eq!(filetime_to_local(0), None);
    }
}
