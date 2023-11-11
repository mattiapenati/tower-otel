use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};

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

impl<S, Req> Service<Req> for OtelTrace<S>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        let inner = self.inner.call(req);
        let span = make_span(self.level);

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

impl<F> Future for ResponseFuture<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.span.enter();

        this.inner.poll(cx)
    }
}

fn make_span(level: Level) -> Span {
    match level {
        Level::ERROR => tracing::error_span!("request"),
        Level::WARN => tracing::warn_span!("request"),
        Level::INFO => tracing::info_span!("request"),
        Level::DEBUG => tracing::debug_span!("request"),
        Level::TRACE => tracing::trace_span!("request"),
    }
}
