//! Requêtes HTTPS minimales (WinHTTP, synchrones) : seul accès réseau de folder, pour sa
//! mise à jour (ARCHITECTURE.md §7.12). WinHTTP est fourni par Windows (pas de pile TLS
//! embarquée) et suit le proxy du système. Appelé uniquement dans le processus éphémère
//! `folder.exe --update-check` : winhttp.dll et ses threads ne sont jamais chargés dans une
//! fenêtre folder.

use std::ffi::c_void;
use std::io::Write;
use std::path::Path;

use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE,
    WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{PCWSTR, w};

use crate::error::{Error, Result, ResultExt};
use crate::util::wide_str;

/// Réponse sans corps : code HTTP et en-tête `Location` (redirections).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub status: u32,
    pub location: Option<String>,
}

/// Échec d'un téléchargement.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error(transparent)]
    Http(#[from] Error),
    #[error("réponse HTTP {0}")]
    Status(u32),
    #[error("fichier trop volumineux")]
    TooLarge,
    #[error("téléchargement incomplet")]
    Truncated,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// GET sans suivre les redirections : code et `Location`. Sert à lire la dernière version
/// publiée, que GitHub donne par une redirection vers la page de sa release.
pub fn head(url: &str, agent: &str) -> Result<Head> {
    let r = Request::send(url, agent, false)?;
    Ok(Head {
        status: r.status()?,
        location: r.header(WINHTTP_QUERY_LOCATION),
    })
}

/// Télécharge `url` (redirections suivies, HTTPS seulement) dans `dest`, au plus
/// `max_bytes` octets ; renvoie la taille écrite. `dest` est créé ou remplacé ; laissé
/// incomplet en cas d'erreur (à l'appelant de le supprimer).
pub fn download(
    url: &str,
    agent: &str,
    dest: &Path,
    max_bytes: u64,
) -> std::result::Result<u64, DownloadError> {
    let r = Request::send(url, agent, true)?;
    let status = r.status()?;
    if status != 200 {
        return Err(DownloadError::Status(status));
    }
    let expected = r.number(WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
    if expected.is_some_and(|n| n > max_bytes) {
        return Err(DownloadError::TooLarge);
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(dest)?);
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let mut n = 0u32;
        // SAFETY: requête valide ; `buf` est accessible en écriture sur toute sa longueur.
        unsafe { WinHttpReadData(r.req.0, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n) }
            .op("WinHttpReadData")?;
        if n == 0 {
            break;
        }
        total += u64::from(n);
        if total > max_bytes {
            return Err(DownloadError::TooLarge);
        }
        file.write_all(&buf[..n as usize])?;
    }
    file.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    if expected.is_some_and(|n| n != total) {
        return Err(DownloadError::Truncated);
    }
    Ok(total)
}

/// Handle WinHTTP fermé au `Drop`.
struct Handle(*mut c_void);

impl Handle {
    fn new(p: *mut c_void, op: &'static str) -> Result<Self> {
        if p.is_null() {
            Err(Error::new(op, windows::core::Error::from_thread()))
        } else {
            Ok(Self(p))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: handle WinHTTP valide, fermé une seule fois.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// Requête envoyée, réponse reçue (en-têtes lus, corps à lire). Champs dans l'ordre de
/// fermeture : requête, connexion, session.
struct Request {
    req: Handle,
    _connect: Handle,
    _session: Handle,
}

impl Request {
    fn send(url: &str, agent: &str, follow: bool) -> Result<Self> {
        let (host, path) = split_https(url).ok_or_else(|| {
            Error::new(
                "URL",
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG),
            )
        })?;
        let agent = wide_str(agent);
        // SAFETY: chaîne terminée par NUL ; pas de proxy imposé (celui du système).
        let session = Handle::new(
            unsafe {
                WinHttpOpen(
                    PCWSTR(agent.as_ptr()),
                    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                    PCWSTR::null(),
                    PCWSTR::null(),
                    0,
                )
            },
            "WinHttpOpen",
        )?;
        // Résolution, connexion, envoi, réception (ms) : un réseau absent échoue vite.
        // SAFETY: session valide.
        unsafe { WinHttpSetTimeouts(session.0, 10_000, 15_000, 15_000, 30_000) }
            .op("WinHttpSetTimeouts")?;
        let host = wide_str(host);
        // SAFETY: session valide, nom d'hôte terminé par NUL.
        let connect = Handle::new(
            unsafe { WinHttpConnect(session.0, PCWSTR(host.as_ptr()), 443, 0) },
            "WinHttpConnect",
        )?;
        let path = wide_str(path);
        // SAFETY: connexion valide, chaînes terminées par NUL ; pas de référent ni de types
        // acceptés (pointeur nul).
        let req = Handle::new(
            unsafe {
                WinHttpOpenRequest(
                    connect.0,
                    w!("GET"),
                    PCWSTR(path.as_ptr()),
                    PCWSTR::null(),
                    PCWSTR::null(),
                    std::ptr::null(),
                    WINHTTP_FLAG_SECURE,
                )
            },
            "WinHttpOpenRequest",
        )?;
        if !follow {
            let v = WINHTTP_DISABLE_REDIRECTS.to_ne_bytes();
            // SAFETY: requête valide ; l'option attend un DWORD.
            unsafe { WinHttpSetOption(Some(req.0), WINHTTP_OPTION_DISABLE_FEATURE, Some(&v)) }
                .op("WinHttpSetOption")?;
        }
        // SAFETY: requête valide, ni en-têtes ni corps.
        unsafe { WinHttpSendRequest(req.0, None, None, 0, 0, 0) }.op("WinHttpSendRequest")?;
        // SAFETY: requête envoyée ; paramètre réservé nul.
        unsafe { WinHttpReceiveResponse(req.0, std::ptr::null_mut()) }
            .op("WinHttpReceiveResponse")?;
        Ok(Self {
            req,
            _connect: connect,
            _session: session,
        })
    }

    fn status(&self) -> Result<u32> {
        self.number(WINHTTP_QUERY_STATUS_CODE)
            .ok_or_else(|| Error::new("WinHttpQueryHeaders", windows::core::Error::from_thread()))
    }

    fn number(&self, query: u32) -> Option<u32> {
        let mut v = 0u32;
        let mut len = 4u32;
        // SAFETY: réponse reçue ; `v` reçoit un DWORD (`WINHTTP_QUERY_FLAG_NUMBER`).
        unsafe {
            WinHttpQueryHeaders(
                self.req.0,
                query | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&raw mut v).cast()),
                &mut len,
                std::ptr::null_mut(),
            )
        }
        .ok()
        .map(|()| v)
    }

    fn header(&self, query: u32) -> Option<String> {
        let mut buf = vec![0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        // SAFETY: réponse reçue ; `buf` fait `len` octets ; un en-tête plus long échoue
        // (ERROR_INSUFFICIENT_BUFFER) et est ignoré.
        unsafe {
            WinHttpQueryHeaders(
                self.req.0,
                query,
                PCWSTR::null(),
                Some(buf.as_mut_ptr().cast()),
                &mut len,
                std::ptr::null_mut(),
            )
        }
        .ok()?;
        buf.truncate(len as usize / 2);
        Some(String::from_utf16_lossy(&buf))
    }
}

/// `https://hôte/chemin` → (`hôte`, `/chemin`). Rien d'autre n'est accepté (ni HTTP, ni
/// port, ni identifiants).
fn split_https(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    valid.then_some((host, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_only() {
        assert_eq!(
            split_https("https://github.com/a/b?x=1"),
            Some(("github.com", "/a/b?x=1"))
        );
        assert_eq!(
            split_https("https://example.org"),
            Some(("example.org", "/"))
        );
        assert_eq!(split_https("http://github.com/a"), None);
        assert_eq!(split_https("https://user@github.com/a"), None);
        assert_eq!(split_https("https://github.com:8443/a"), None);
        assert_eq!(split_https("https:///a"), None);
    }
}
