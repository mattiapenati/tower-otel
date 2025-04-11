//! Implementation of fields injector.

use std::str::FromStr;

use http::{HeaderMap, HeaderName, HeaderValue};

pub struct HeaderInjector<'a>(pub &'a mut HeaderMap);

impl opentelemetry::propagation::Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let Ok(header_name) = HeaderName::from_str(key) {
            if let Ok(header_value) = HeaderValue::from_str(&value) {
                self.0.insert(header_name, header_value);
            }
        }
    }
}
