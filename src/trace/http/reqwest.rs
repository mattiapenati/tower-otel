use crate::util;

impl super::HttpRequest for reqwest::Request {}

impl super::sealed::HttpRequest for reqwest::Request {
    #[inline(always)]
    fn extract_span_data<'r>(
        &'r mut self,
        kind: super::sealed::SpanKind,
    ) -> super::sealed::RequestSpanData<'r> {
        assert!(
            matches!(kind, super::sealed::SpanKind::Client),
            "Http middleware with reqwest::Request only supports client-side spans"
        );
        super::sealed::RequestSpanData {
            method: util::http_method(self.method()),
            version: util::http_version(self.version()),
            url: util::Uri::Reqwest(self.url().clone()),
            headers: self.headers(),
            server_address: None,
            server_port: None,
            url_scheme: None,
            http_route: None,
            client_address: None,
        }
    }

    #[inline(always)]
    fn headers_mut(&mut self) -> &mut http::HeaderMap {
        reqwest::Request::headers_mut(self)
    }
}

impl super::HttpResponse for reqwest::Response {}

impl super::sealed::HttpResponse for reqwest::Response {
    #[inline(always)]
    fn status(&self) -> http::StatusCode {
        reqwest::Response::status(self)
    }

    #[inline(always)]
    fn headers(&self) -> &http::HeaderMap {
        reqwest::Response::headers(self)
    }
}
