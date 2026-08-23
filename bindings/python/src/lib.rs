#![allow(clippy::useless_conversion)]

use oidc_exchange_ffi::{TransportHints, WireRequest};
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PySequence};

fn required_string(request: &Bound<'_, PyDict>, field: &'static str) -> PyResult<String> {
    request
        .get_item(field)?
        .ok_or_else(|| PyKeyError::new_err(field))?
        .extract::<String>()
        .map_err(|_| PyValueError::new_err(format!("request field '{field}' must be a string")))
}

fn required_bytes(request: &Bound<'_, PyDict>, field: &'static str) -> PyResult<Vec<u8>> {
    request
        .get_item(field)?
        .ok_or_else(|| PyKeyError::new_err(field))?
        .extract::<Vec<u8>>()
        .map_err(|_| PyValueError::new_err(format!("request field '{field}' must be bytes")))
}

fn optional_bytes(request: &Bound<'_, PyDict>, field: &'static str) -> PyResult<Option<Vec<u8>>> {
    request
        .get_item(field)?
        .map(|value| {
            value.extract::<Vec<u8>>().map_err(|_| {
                PyValueError::new_err(format!("request field '{field}' must be bytes"))
            })
        })
        .transpose()
}

fn headers(request: &Bound<'_, PyDict>) -> PyResult<Vec<(String, String)>> {
    let Some(value) = request.get_item("headers")? else {
        return Ok(Vec::new());
    };
    let sequence = value.downcast::<PySequence>().map_err(|_| {
        PyValueError::new_err("request field 'headers' must be an ordered sequence of pairs")
    })?;
    sequence
        .iter()?
        .map(|item| {
            let item = item?;
            item.extract::<(String, String)>().map_err(|_| {
                PyValueError::new_err("request headers must be (name, value) string pairs")
            })
        })
        .collect()
}

#[pyclass]
struct OidcExchange {
    inner: oidc_exchange_ffi::OidcExchange,
}

#[pymethods]
impl OidcExchange {
    #[new]
    #[pyo3(signature = (*, config=None, config_string=None))]
    fn new(config: Option<&str>, config_string: Option<&str>) -> PyResult<Self> {
        let inner = match (config, config_string) {
            (Some(path), _) => oidc_exchange_ffi::OidcExchange::from_file(path),
            (_, Some(toml)) => oidc_exchange_ffi::OidcExchange::new(toml),
            _ => {
                return Err(PyValueError::new_err(
                    "Either 'config' or 'config_string' must be provided",
                ))
            }
        }
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        Ok(Self { inner })
    }

    fn handle_request_sync<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyDict>,
    ) -> PyResult<Py<PyDict>> {
        let wire = WireRequest {
            method: required_string(request, "method")?,
            raw_path: required_bytes(request, "raw_path")?,
            query: optional_bytes(request, "query")?,
            headers: headers(request)?,
            body: optional_bytes(request, "body")?.unwrap_or_default(),
            hints: TransportHints {
                path_is_raw: request
                    .get_item("path_is_raw")?
                    .ok_or_else(|| PyKeyError::new_err("path_is_raw"))?
                    .extract::<bool>()
                    .map_err(|_| {
                        PyValueError::new_err("request field 'path_is_raw' must be a bool")
                    })?,
            },
        };
        let response = py
            .allow_threads(|| self.inner.runtime_handle_for_test(wire))
            .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
        let result = PyDict::new_bound(py);
        result.set_item("status", response.status)?;
        let response_headers = PyList::empty_bound(py);
        for header in response.headers {
            response_headers.append(header)?;
        }
        result.set_item("headers", response_headers)?;
        result.set_item("body", PyBytes::new_bound(py, &response.body))?;
        Ok(result.unbind())
    }

    fn limits(&self) -> u64 {
        self.inner.limits().max_body_bytes
    }
    fn shutdown(&self) {}
}

#[pymodule]
fn _oidc_exchange(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<OidcExchange>()?;
    Ok(())
}
