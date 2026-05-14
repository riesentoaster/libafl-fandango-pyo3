use std::ffi::NulError;

use pyo3::PyErr;
use pyo3::prelude::*;

pub(crate) mod inprocess;
pub(crate) mod subprocess;

pub use inprocess::FandangoInprocessModule;
pub use subprocess::{FandangoSubprocessInitIpc, FandangoSubprocessModule};

#[deprecated(
    since = "0.4.0",
    note = "Explicitly use FandangoInprocessModule or FandangoSubprocessModule instead"
)]
pub type FandangoPythonModule = FandangoInprocessModule;
#[deprecated(since = "0.4.0", note = "Renamed to FandangoModuleInitError")]
pub type FandangoPythonModuleInitError = FandangoModuleInitError;

pub trait FandangoClient {
    fn next_input(&mut self) -> Result<Vec<u8>, String>;
    fn parse_input(&mut self, input: &[u8]) -> Result<u32, String>;
}

#[derive(Debug)]
pub enum FandangoModuleInitError {
    ModuleNotFoundError(PyErr, String),
    PyErr(PyErr, String),
    FilePathError(String),
    ReadFileError(String),
    EncodingError(NulError),
    /// Only [`FandangoSubprocessModule`](subprocess::FandangoSubprocessModule); see [`FandangoSubprocessInitIpc`].
    SubprocessIpc(FandangoSubprocessInitIpc),
}

impl FandangoModuleInitError {
    /// Full human-readable report for logging or IPC.
    ///
    /// For Python failures this combines the exception's usual string form with the formatted
    /// traceback when one is available (see [`FandangoInprocessModule`](inprocess::FandangoInprocessModule)’s use of `traceback.format_exception`).
    pub fn format_report(&self) -> String {
        match self {
            Self::ModuleNotFoundError(err, tb) => {
                Self::format_py_err_pair(err, tb, "Module not found")
            }
            Self::PyErr(err, tb) => Self::format_py_err_pair(err, tb, "Python error"),
            Self::FilePathError(s) => s.clone(),
            Self::ReadFileError(s) => s.clone(),
            Self::EncodingError(e) => format!("Invalid string data (embedded NUL): {e}"),
            Self::SubprocessIpc(e) => e.to_string(),
        }
    }

    fn format_py_err_pair(err: &PyErr, tb: &str, label: &str) -> String {
        Python::with_gil(|_| {
            let head = err.to_string();
            let tb = tb.trim();
            let tb_useful = !tb.is_empty() && tb != "No traceback available";
            if tb_useful {
                format!("{label}: {head}\n\n{tb}")
            } else {
                format!("{label}: {head}")
            }
        })
    }
}

impl std::fmt::Display for FandangoModuleInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.format_report())
    }
}

impl std::error::Error for FandangoModuleInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EncodingError(e) => Some(e),
            Self::PyErr(e, _) | Self::ModuleNotFoundError(e, _) => e.source(),
            Self::FilePathError(_) | Self::ReadFileError(_) => None,
            Self::SubprocessIpc(e) => std::error::Error::source(e),
        }
    }
}
