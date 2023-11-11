use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;
use tower_layer::Layer;
use tower_service::Service;

/// [`Layer`] that adds OpenTelemetry tracing to a [`Service`].
///
/// [`Layer`]: tower_layer::Layer
/// [`Service`]: tower_service::Service
pub struct OtelTraceLayer;

impl<S> Layer<S> for OtelTraceLayer {
    type Service = OtelTrace<S>;

    fn layer(&self, inner: S) -> Self::Service {
        OtelTrace { inner }
    }
}

/// Middleware that adds OpenTelemetry tracing to a [`Service`].
///
/// [`Service`]: tower_service::Service
#[derive(Clone)]
pub struct OtelTrace<S> {
    inner: S,
}

impl<S, Req> Service<Req> for OtelTrace<S>
where
    S: Service<Req>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        self.inner.call(req)
    }
}

pin_project! {
    /// Response future for [`OtelTrace`].
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
    }
}

impl<F> Future for ResponseFuture<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        this.inner.poll(cx)
    }
}
