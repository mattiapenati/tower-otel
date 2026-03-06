use crate::util;

impl super::HttpRequest for reqwest::Request {}

impl super::sealed::HttpRequest for reqwest::Request {
    fn extract_metric_data<'r>(
        &'r self,
        side: super::sealed::MetricSide,
    ) -> super::sealed::RequestMetricData<'r> {
        debug_assert!(
            matches!(side, super::sealed::MetricSide::Client),
            "Http metrics middleware with reqwest::Request only supports client-side metrics"
        );

        super::sealed::RequestMetricData {
            method: self.method(),
            server_address: self.url().host_str(),
            server_port: self.url().port_or_known_default(),
            url_scheme: Some(self.url().scheme()),
            version: self.version(),
            http_route: None,
            body_size: util::http_body_size_from_headers(self.headers()),
        }
    }
}

impl super::HttpResponse for reqwest::Response {}

impl super::sealed::HttpResponse for reqwest::Response {
    fn status(&self) -> http::StatusCode {
        self.status()
    }

    fn body_size(&self) -> Option<u64> {
        util::http_body_size_from_headers(self.headers())
    }
}
