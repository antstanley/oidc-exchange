// PyO3's `?` operator on PyResult triggers clippy::useless_conversion
// because From<PyErr> for PyErr is an identity conversion. This is a
// known interaction between PyO3 proc macros and clippy.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict};

fn required_string(request: &Bound<'_, PyDict>, field: &'static str) -> PyResult<String> {
    let value = request
        .get_item(field)?
        .ok_or_else(|| PyKeyError::new_err(field))?;
    value
        .extract::<String>()
        .map_err(|_| PyValueError::new_err(format!("request field '{field}' must be a string")))
}

fn optional_body(value: Option<Bound<'_, PyAny>>) -> PyResult<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Ok(bytes) = value.extract::<Vec<u8>>() {
        return Ok(bytes);
    }
    value
        .extract::<String>()
        .map(String::into_bytes)
        .map_err(|_| PyValueError::new_err("request field 'body' must be bytes or a string"))
}

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
        let method = required_string(request, "method")?;
        let path = required_string(request, "path")?;

        let headers: Vec<(String, String)> = if let Some(h) = request.get_item("headers")? {
            let hdict = h.downcast::<PyDict>().map_err(|_| {
                PyValueError::new_err("request field 'headers' must be a dictionary")
            })?;
            hdict
                .iter()
                .map(|(key, value)| {
                    let key = key.extract::<String>().map_err(|_| {
                        PyValueError::new_err("request header names must be strings")
                    })?;
                    let value = value.extract::<String>().map_err(|_| {
                        PyValueError::new_err("request header values must be strings")
                    })?;
                    Ok((key, value))
                })
                .collect::<PyResult<_>>()?
        } else {
            Vec::new()
        };

        let body = optional_body(request.get_item("body")?)?;

        // Release the GIL for the blocking FFI call so other Python threads —
        // including an asyncio event loop driving this call via an executor —
        // keep running while the request is serviced. All inputs are owned/Send
        // Rust values by this point, so the closure satisfies `allow_threads`'
        // `Send` bound with no restructuring.
        #[allow(deprecated)]
        let response = py
            .allow_threads(|| self.inner.handle_request(&method, &path, headers, body))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))?;

        // The GIL is re-held here; only now do we touch Python objects again.
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

        // Postcondition on the built response: the caller-facing contract
        // documented above promises a `status` key on every successful result.
        debug_assert!(
            result.contains("status")?,
            "result dict must carry a status key"
        );

        Ok(result.unbind())
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
