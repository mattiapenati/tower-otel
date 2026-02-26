use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use pin_project::pin_project;
use tower_service::Service;
use tracing::Span;

use super::{
    make_request_span, record_error, record_response, AnyUrl, Http, RequestSpanData, SpanKind,
};
use crate::util;

impl<'r, 'h> RequestSpanData<'r, 'h> {
    /// Build from a `reqwest::Request` acting as a **client**.
    ///
    /// Since `reqwest::Request` has no `into_parts()`, `req.url()` (shared borrow) and
    /// `req.headers_mut()` (exclusive borrow) cannot coexist. The URL is cloned once so
    /// that the shared borrow ends before the exclusive borrow begins. All string accessors
    /// then borrow from the owned `url::Url` — zero further allocations.
    pub(super) fn from_reqwest_request(kind: SpanKind, req: &'r mut reqwest::Request) -> Self
    where
        'r: 'h,
    {
        let method = util::http_method(req.method());
        let version = util::http_version(req.version());
        // Clone the whole URL once; all string borrows (path, host, scheme, …) come from
        // the owned value. The shared borrow of `req` ends here, before headers_mut().
        let url = AnyUrl::Url(req.url().clone());
        // All shared borrows of req released. Safe to take exclusive borrow:
        let headers = req.headers_mut();
        Self {
            kind,
            method,
            version,
            url,
            headers,
            server_address: None,
            server_port: None,
            url_scheme: None,
            http_route: None,
            client_address: None,
        }
    }
}

impl<S> Service<reqwest::Request> for Http<S>
where
    S: Service<reqwest::Request, Response = reqwest::Response>,
    S::Error: Display,
{
    type Response = reqwest::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: reqwest::Request) -> Self::Future {
        debug_assert!(
            matches!(self.kind, SpanKind::Client),
            "Http middleware with reqwest::Request only supports client-side spans"
        );
        let span = {
            let mut data = RequestSpanData::from_reqwest_request(self.kind, &mut req);
            make_request_span(self.level, &mut data)
        };
        let inner = {
            let _enter = span.enter();
            self.inner.call(req)
        };
        ResponseFuture {
            inner,
            span,
            kind: self.kind,
        }
    }
}

/// Response future for [`Http`] tracing middleware when wrapping a reqwest service.
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    span: Span,
    kind: SpanKind,
}

impl<F, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<reqwest::Response, E>>,
    E: Display,
{
    type Output = Result<reqwest::Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();

        match ready!(this.inner.poll(cx)) {
            Ok(response) => {
                record_response(this.span, response.status(), response.headers());
                Poll::Ready(Ok(response))
            }
            Err(err) => {
                record_error(this.span, &err);
                Poll::Ready(Err(err))
            }
        }
    }
}
