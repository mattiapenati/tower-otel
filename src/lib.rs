use std::{
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// [`Layer`] that adds OpenTelemetry tracing to a [`Service`].
///
/// [`Layer`]: tower_layer::Layer
/// [`Service`]: tower_service::Service
#[derive(Clone)]
pub struct OtelTraceLayer {
    level: Level,
}

impl OtelTraceLayer {
    /// Set the [`Level`] used for tracing [`Span`].
    ///
    /// [`Level`]: tracing::Level
    /// [`Span`]: tracing::Span
    pub fn new(level: Level) -> Self {
        Self { level }
    }
}

impl<S> Layer<S> for OtelTraceLayer {
    type Service = OtelTrace<S>;

    fn layer(&self, inner: S) -> Self::Service {
        let level = self.level;
        OtelTrace { inner, level }
    }
}

/// Middleware that adds OpenTelemetry tracing to a [`Service`].
///
/// [`Service`]: tower_service::Service
#[derive(Clone)]
pub struct OtelTrace<S> {
    inner: S,
    level: Level,
}

impl<S, ReqBody, ResBody> Service<http::Request<ReqBody>> for OtelTrace<S>
where
    S: Service<http::Request<ReqBody>, Response = http::Response<ResBody>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: http::Request<ReqBody>) -> Self::Future {
        let span = make_span(self.level, &req);
        let inner = self.inner.call(req);

        ResponseFuture { inner, span }
    }
}

pin_project! {
    /// Response future for [`OtelTrace`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        span: Span,
    }
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<http::Response<ResBody>, E>>,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();

        match ready!(this.inner.poll(cx)) {
            Ok(response) => {
                update_span(this.span, &response);
                Poll::Ready(Ok(response))
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

fn make_span<B>(level: Level, req: &http::Request<B>) -> Span {
    macro_rules! make_span {
        ($level:expr) => {{
            tracing::span!(
                $level,
                "request",
                "http.request.method" = %req.method(),
                "http.response.status_code" = tracing::field::Empty,
                "url.path" = req.uri().path(),
                "url.query" = req.uri().query(),
            )
        }};
    }

    let span = match level {
        Level::ERROR => make_span!(Level::ERROR),
        Level::WARN => make_span!(Level::WARN),
        Level::INFO => make_span!(Level::INFO),
        Level::DEBUG => make_span!(Level::DEBUG),
        Level::TRACE => make_span!(Level::TRACE),
    };

    for (header_name, header_value) in req.headers().iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.request.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    span
}

fn update_span<B>(span: &Span, res: &http::Response<B>) {
    // `u16` values are recorded as string
    span.record("http.response.status_code", res.status().as_u16() as i64);

    for (header_name, header_value) in res.headers().iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.response.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }
}
