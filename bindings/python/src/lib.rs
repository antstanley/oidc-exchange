use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

/// Python wrapper around the FFI OidcExchange instance.
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
            (None, None) => {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "Either 'config' (file path) or 'config_string' (TOML string) must be provided",
                ));
            }
        }
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        Ok(Self { inner })
    }

    /// Send an HTTP request through the router and return a response dict.
    ///
    /// The `request` dict must contain:
    ///   - method: str
    ///   - path: str
    ///   - headers: Optional[dict[str, str]]
    ///   - body: Optional[bytes | str]
    ///
    /// Returns a dict with:
    ///   - status: int
    ///   - headers: dict[str, str]
    ///   - body: bytes
    fn handle_request_sync<'py>(
        &self,
        py: Python<'py>,
        request: &Bound<'py, PyDict>,
    ) -> PyResult<Py<PyDict>> {
        let method: String = request
            .get_item("method")?
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("method"))?
            .extract()?;

        let path: String = request
            .get_item("path")?
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("path"))?
            .extract()?;

        let headers: Vec<(String, String)> = if let Some(h) = request.get_item("headers")? {
            let hdict: &Bound<'py, PyDict> = h.downcast()?;
            let mut vec = Vec::new();
            for (k, v) in hdict.iter() {
                vec.push((k.extract::<String>()?, v.extract::<String>()?));
            }
            vec
        } else {
            Vec::new()
        };

        let body: Vec<u8> = if let Some(b) = request.get_item("body")? {
            // Try bytes first, then string, default empty
            if let Ok(bytes) = b.extract::<Vec<u8>>() {
                bytes
            } else if let Ok(s) = b.extract::<String>() {
                s.into_bytes()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let response = self
            .inner
            .handle_request(&method, &path, headers, body)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        let result = PyDict::new_bound(py);
        result.set_item("status", response.status)?;

        // Use a dict for headers. Note: duplicate header names (e.g. Set-Cookie)
        // will be collapsed. For full multi-value header support, consumers should
        // check the raw response. This covers the common case.
        let resp_headers = PyDict::new_bound(py);
        for (k, v) in &response.headers {
            resp_headers.set_item(k, v)?;
        }
        result.set_item("headers", resp_headers)?;
        result.set_item("body", PyBytes::new_bound(py, &response.body))?;

        Ok(result.into())
    }

    /// Shutdown the instance (no-op, reserved for future use).
    fn shutdown(&self) {}
}

/// Python module definition.
#[pymodule]
fn _oidc_exchange(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<OidcExchange>()?;
    Ok(())
}
