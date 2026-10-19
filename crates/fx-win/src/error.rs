use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};

/// Erreur Win32/COM avec le nom de l'opération qui a échoué.
#[derive(Debug, thiserror::Error)]
#[error("{op}: {source}")]
pub struct Error {
    op: &'static str,
    #[source]
    source: windows::core::Error,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn new(op: &'static str, source: windows::core::Error) -> Self {
        Self { op, source }
    }

    /// HRESULT brut, pour les journaux.
    pub fn hresult(&self) -> i32 {
        self.source.code().0
    }

    pub fn is_not_found(&self) -> bool {
        let code = self.source.code();
        code == ERROR_FILE_NOT_FOUND.to_hresult() || code == ERROR_PATH_NOT_FOUND.to_hresult()
    }

    pub fn is_access_denied(&self) -> bool {
        self.source.code() == ERROR_ACCESS_DENIED.to_hresult()
    }

    /// Message système localisé, sans le nom de l'opération (pour l'UI).
    pub fn message(&self) -> String {
        self.source.message()
    }
}

pub(crate) trait ResultExt<T> {
    fn op(self, op: &'static str) -> Result<T>;
}

impl<T> ResultExt<T> for windows::core::Result<T> {
    fn op(self, op: &'static str) -> Result<T> {
        self.map_err(|e| Error::new(op, e))
    }
}
