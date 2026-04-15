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

pub struct HeaderExtractor<'a>(pub &'a HeaderMap);

impl opentelemetry::propagation::Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .and_then(|header_value| header_value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(|header_name| header_name.as_str())
            .collect()
    }
}
