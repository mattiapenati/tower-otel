//! Implementation of fields injector.

use http::HeaderMap;

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
