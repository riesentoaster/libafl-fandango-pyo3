use std::{
    ffi::CString,
    path::{Path, PathBuf},
};

use pyo3::{exceptions::PyModuleNotFoundError, prelude::*, types::PyDict};

use crate::fandango::{FandangoClient, FandangoModuleInitError};

/// A module for running Fandango in process.
///
/// Fast. Kills the entire process if Fandango e.g. OOMs.
pub struct FandangoInprocessModule {
    module: Py<PyModule>,
    generator: Py<PyAny>,
}

impl FandangoInprocessModule {
    fn format_py_traceback(py: Python<'_>, err: &PyErr) -> Option<String> {
        let traceback_module = py.import("traceback").ok()?;
        let formatted = traceback_module
            .call_method1(
                "format_exception",
                (err.get_type(py), err.value(py), err.traceback(py)),
            )
            .ok()?;
        let lines = formatted.extract::<Vec<String>>().ok()?;
        Some(lines.concat())
    }

    fn map_py_init_error(py: Python<'_>, err: PyErr) -> FandangoModuleInitError {
        let tb = Self::format_py_traceback(py, &err);
        if err
            .matches(py, py.get_type::<PyModuleNotFoundError>())
            .unwrap_or(false)
            && let Some(tb) = tb
        {
            FandangoModuleInitError::ModuleNotFoundError(err, tb)
        } else {
            FandangoModuleInitError::PyErr(err, tb.unwrap_or("No traceback available".to_string()))
        }
    }

    pub fn new(
        fandango_file: &str,
        kwargs: &[(&str, &str)],
    ) -> Result<Self, FandangoModuleInitError> {
        let path_of_default_interface =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/run_fandango.py");
        let iface = path_of_default_interface.to_str().ok_or_else(|| {
            FandangoModuleInitError::FilePathError(
                "default interface path is not UTF-8".to_string(),
            )
        })?;
        Self::with_custom_python_interface(iface, fandango_file, kwargs)
    }

    pub fn with_custom_python_interface(
        python_interface_path: &str,
        fandango_file: &str,
        kwargs: &[(&str, &str)],
    ) -> Result<Self, FandangoModuleInitError> {
        Python::with_gil(|py| {
            let wrapped_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                wrapped_kwargs
                    .set_item(k, v)
                    .map_err(|err| Self::map_py_init_error(py, err))?;
            }

            let (module, generator) = Self::load_interface_and_setup(
                py,
                python_interface_path,
                fandango_file,
                &wrapped_kwargs,
            )?;

            Ok(Self { module, generator })
        })
    }

    /// Shared by in-process use and the out-of-process IPC worker.
    pub(crate) fn load_interface_and_setup(
        py: Python<'_>,
        python_interface_path: &str,
        fandango_file: &str,
        kwargs: &Bound<'_, PyDict>,
    ) -> Result<(Py<PyModule>, Py<PyAny>), FandangoModuleInitError> {
        let code = Self::read_code(python_interface_path)?;
        let (file_name, file_name_str) = Self::sanitize_file_name(python_interface_path)?;
        let module_name = Self::sanitize_module_name(python_interface_path, file_name_str)?;

        let module = PyModule::from_code(py, &code, &file_name, &module_name)
            .map_err(|err| Self::map_py_init_error(py, err))?;
        let module: Py<PyModule> = module.into();

        let generator = module
            .getattr(py, "setup")
            .map_err(|err| Self::map_py_init_error(py, err))?
            .call1(py, (fandango_file, kwargs))
            .map_err(|err| Self::map_py_init_error(py, err))?;

        module
            .getattr(py, "next_input")
            .map_err(|err| Self::map_py_init_error(py, err))?;
        module
            .getattr(py, "parse_input")
            .map_err(|err| Self::map_py_init_error(py, err))?;

        Ok((module, generator))
    }

    fn read_code(path: &str) -> Result<CString, FandangoModuleInitError> {
        let code = std::fs::read_to_string(path).map_err(|e| {
            FandangoModuleInitError::ReadFileError(format!("Could not read file: {}", e))
        })?;
        CString::new(code).map_err(FandangoModuleInitError::EncodingError)
    }

    fn sanitize_file_name(path: &str) -> Result<(CString, &str), FandangoModuleInitError> {
        let path_as_path = Path::new(path);
        let file_name = path_as_path
            .file_name()
            .ok_or(FandangoModuleInitError::FilePathError(format!(
                "Could not extract file name from path: {}",
                path
            )))?
            .to_str()
            .ok_or(FandangoModuleInitError::FilePathError(format!(
                "Could not convert file name to string: {}",
                path
            )))?;
        Ok((
            CString::new(file_name).map_err(FandangoModuleInitError::EncodingError)?,
            file_name,
        ))
    }

    fn sanitize_module_name(
        path: &str,
        file_name: &str,
    ) -> Result<CString, FandangoModuleInitError> {
        let path_as_path = Path::new(path);
        let module_name = if file_name == "__init__.py" {
            path_as_path
                .parent()
                .ok_or(FandangoModuleInitError::FilePathError(format!(
                    "Could not extract parent directory from path: {}",
                    path
                )))?
                .file_name()
                .ok_or(FandangoModuleInitError::FilePathError(format!(
                    "No parent directory in path: {}",
                    path
                )))?
                .to_str()
                .ok_or(FandangoModuleInitError::FilePathError(format!(
                    "Could not convert parent directory to string: {}",
                    path
                )))?
        } else {
            file_name
                .strip_suffix(".py")
                .ok_or(FandangoModuleInitError::FilePathError(format!(
                    "File name does not end with .py: {}",
                    file_name
                )))?
        };
        CString::new(module_name).map_err(FandangoModuleInitError::EncodingError)
    }
}

impl FandangoClient for FandangoInprocessModule {
    fn next_input(&mut self) -> Result<Vec<u8>, String> {
        Python::with_gil(|py| {
            let generator = self.generator.clone_ref(py);
            self.module
                .getattr(py, "next_input")?
                .call1(py, (generator,))?
                .extract::<Vec<u8>>(py)
        })
        .map_err(|e| e.to_string())
    }

    fn parse_input(&mut self, input: &[u8]) -> Result<u32, String> {
        Python::with_gil(|py| {
            let generator = self.generator.clone_ref(py);
            self.module
                .getattr(py, "parse_input")?
                .call1(py, (generator, input))?
                .extract::<u32>(py)
        })
        .map_err(|e| e.to_string())
    }
}
