use std::{
    ffi::{CString, NulError},
    path::Path,
};

use pyo3::{exceptions::PyModuleNotFoundError, prelude::*, types::PyDict};

pub struct FandangoPythonModule {
    module: Py<PyModule>,
    generator: Py<PyAny>,
}

#[derive(Debug)]
pub enum FandangoPythonModuleInitError {
    ModuleNotFoundError(PyErr, String),
    PyErr(PyErr, String),
    FilePathError(String),
    ReadFileError(String),
    EncodingError(NulError),
}

impl FandangoPythonModule {
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

    fn map_py_init_error(py: Python<'_>, err: PyErr) -> FandangoPythonModuleInitError {
        let tb = Self::format_py_traceback(py, &err);
        if err
            .matches(py, py.get_type::<PyModuleNotFoundError>())
            .unwrap_or(false)
            && let Some(tb) = tb
        {
            FandangoPythonModuleInitError::ModuleNotFoundError(err, tb)
        } else {
            FandangoPythonModuleInitError::PyErr(
                err,
                tb.unwrap_or("No traceback available".to_string()),
            )
        }
    }

    pub fn new(
        fandango_file: &str,
        kwargs: &[(&str, &str)],
    ) -> Result<Self, FandangoPythonModuleInitError> {
        let path_of_default_interface = Path::new(file!())
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/run_fandango.py");

        Self::with_custom_python_interface(
            path_of_default_interface.to_str().unwrap(),
            fandango_file,
            kwargs,
        )
    }

    pub fn with_custom_python_interface(
        python_interface_path: &str,
        fandango_file: &str,
        kwargs: &[(&str, &str)],
    ) -> Result<Self, FandangoPythonModuleInitError> {
        let code = Self::read_code(python_interface_path)?;
        let (file_name, file_name_str) = Self::sanitize_file_name(python_interface_path)?;
        let module_name = Self::sanitize_module_name(python_interface_path, file_name_str)?;

        Python::with_gil(|py| {
            let module = PyModule::from_code(py, &code, &file_name, &module_name)
                .map_err(|err| Self::map_py_init_error(py, err))?;
            let module: Py<PyModule> = module.into();

            let wrapped_kwargs = PyDict::new(py);
            for (k, v) in kwargs {
                wrapped_kwargs.set_item(k, v).unwrap();
            }

            let generator = module
                .getattr(py, "setup")
                .map_err(|err| Self::map_py_init_error(py, err))?
                .call1(py, (fandango_file, wrapped_kwargs))
                .map_err(|err| Self::map_py_init_error(py, err))?;

            // check if next_input and parse_input are defined
            module
                .getattr(py, "next_input")
                .map_err(|err| Self::map_py_init_error(py, err))?;
            module
                .getattr(py, "parse_input")
                .map_err(|err| Self::map_py_init_error(py, err))?;

            Ok(Self { module, generator })
        })
    }

    fn read_code(path: &str) -> Result<CString, FandangoPythonModuleInitError> {
        let code = std::fs::read_to_string(path).map_err(|e| {
            FandangoPythonModuleInitError::ReadFileError(format!("Could not read file: {}", e))
        })?;
        CString::new(code).map_err(FandangoPythonModuleInitError::EncodingError)
    }

    fn sanitize_file_name(path: &str) -> Result<(CString, &str), FandangoPythonModuleInitError> {
        let path_as_path = Path::new(path);
        let file_name = path_as_path
            .file_name()
            .ok_or(FandangoPythonModuleInitError::FilePathError(format!(
                "Could not extract file name from path: {}",
                path
            )))?
            .to_str()
            .ok_or(FandangoPythonModuleInitError::FilePathError(format!(
                "Could not convert file name to string: {}",
                path
            )))?;
        Ok((
            CString::new(file_name).map_err(FandangoPythonModuleInitError::EncodingError)?,
            file_name,
        ))
    }

    fn sanitize_module_name(
        path: &str,
        file_name: &str,
    ) -> Result<CString, FandangoPythonModuleInitError> {
        let path_as_path = Path::new(path);
        let module_name = if file_name == "__init__.py" {
            path_as_path
                .parent()
                .ok_or(FandangoPythonModuleInitError::FilePathError(format!(
                    "Could not extract parent directory from path: {}",
                    path
                )))?
                .file_name()
                .ok_or(FandangoPythonModuleInitError::FilePathError(format!(
                    "No parent directory in path: {}",
                    path
                )))?
                .to_str()
                .ok_or(FandangoPythonModuleInitError::FilePathError(format!(
                    "Could not convert parent directory to string: {}",
                    path
                )))?
        } else {
            file_name
                .strip_suffix(".py")
                .ok_or(FandangoPythonModuleInitError::FilePathError(format!(
                    "File name does not end with .py: {}",
                    file_name
                )))?
        };
        CString::new(module_name).map_err(FandangoPythonModuleInitError::EncodingError)
    }

    pub fn next_input(&self) -> Result<Vec<u8>, PyErr> {
        Python::with_gil(|py| {
            let generator = self.generator.clone_ref(py);
            let input = self
                .module
                .getattr(py, "next_input")?
                .call1(py, (generator,))?
                .extract::<Vec<u8>>(py)?;
            Ok(input)
        })
    }

    pub fn parse_input(&self, input: &[u8]) -> Result<u32, PyErr> {
        Python::with_gil(|py| {
            let generator = self.generator.clone_ref(py);
            self.module
                .getattr(py, "parse_input")?
                .call1(py, (generator, input))?
                .extract::<u32>(py)
        })
    }
}
