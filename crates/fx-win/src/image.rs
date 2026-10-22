//! Icônes et miniatures via `IShellItemImageFactory` (exploite le cache de miniatures de
//! Windows : aucun décodeur d'image embarqué).

use std::path::Path;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK,
    SIIGBF_ICONONLY, SIIGBF_THUMBNAILONLY,
};
use windows::core::PCWSTR;

use crate::error::{Error, Result, ResultExt};
use crate::util::wide;

/// Image RGBA8 **prémultipliée**, lignes de haut en bas.
#[derive(Debug, Clone)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// Icône associée (par type de fichier, ou propre au fichier : .exe, .lnk…).
    Icon,
    /// Miniature si le type en fournit une, sinon icône.
    Thumbnail,
    /// Miniature uniquement (échoue sinon).
    ThumbnailOnly,
}

/// Image shell de `path`, au plus `size`×`size` pixels.
///
/// Bloquant (peut solliciter des extensions tierces) : à appeler depuis un thread STA.
pub fn shell_image(path: &Path, size: u32, kind: ImageKind) -> Result<Rgba> {
    let w = wide(path);
    // SAFETY: chaîne terminée par NUL valide pendant l'appel.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }
            .op("SHCreateItemFromParsingName")?;
    let flags: SIIGBF = match kind {
        ImageKind::Icon => SIIGBF_ICONONLY,
        ImageKind::Thumbnail => SIIGBF_BIGGERSIZEOK,
        ImageKind::ThumbnailOnly => SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
    };
    let s = SIZE {
        cx: size as i32,
        cy: size as i32,
    };
    // SAFETY: fabrique valide ; le HBITMAP renvoyé nous appartient.
    let hbm = unsafe { factory.GetImage(s, flags) }.op("IShellItemImageFactory::GetImage")?;
    let bitmap = OwnedBitmap(hbm);
    read_bitmap(bitmap.0)
}

pub(crate) struct OwnedBitmap(pub(crate) HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        // SAFETY: bitmap possédé, supprimé une seule fois.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

pub(crate) fn read_bitmap(hbm: HBITMAP) -> Result<Rgba> {
    let mut bm = BITMAP::default();
    // SAFETY: `bm` est une BITMAP de la taille annoncée.
    let n = unsafe {
        GetObjectW(
            HGDIOBJ(hbm.0),
            size_of::<BITMAP>() as i32,
            Some((&raw mut bm).cast()),
        )
    };
    if n == 0 || bm.bmWidth <= 0 || bm.bmHeight == 0 {
        return Err(Error::new(
            "GetObjectW",
            windows::core::Error::from_thread(),
        ));
    }
    let (w, h) = (bm.bmWidth as u32, bm.bmHeight.unsigned_abs());
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32), // de haut en bas
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: DC écran temporaire ; `px` a la taille exacte demandée (32 bpp, w×h).
    let lines = unsafe {
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            hbm,
            0,
            h,
            Some(px.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        lines
    };
    if lines == 0 {
        return Err(Error::new("GetDIBits", windows::core::Error::from_thread()));
    }
    // BGRA -> RGBA ; certaines miniatures n'ont aucun alpha : on les rend opaques.
    let any_alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    for p in px.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        if !any_alpha {
            p[3] = 255;
        }
    }
    Ok(Rgba {
        width: w,
        height: h,
        pixels: px,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_of_notepad() {
        let _com = crate::com::init_sta();
        let windir = std::env::var_os("SystemRoot").expect("SystemRoot");
        let p = Path::new(&windir).join("notepad.exe");
        let img = shell_image(&p, 32, ImageKind::Icon).expect("icon");
        assert!(img.width >= 16 && img.width <= 64);
        assert_eq!(img.pixels.len(), (img.width * img.height * 4) as usize);
        assert!(img.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    }
}
