//! Middleware that adds tracing to a [`Service`] that handles gRPC requests.

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use http::{Request, Response};
use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::util;

use super::{OnError, OnRequest, OnResponse, SpanHandler};

/// Describes the relationship between the [`Span`] and the service producing the span.
#[derive(Clone, Copy, Debug)]
enum SpanKind {
    /// The span describes a request sent to some remote service.
    Client,
    /// The span describes the server-side handling of a request.
    Server,
}

/// [`Layer`] that adds tracing to a [`Service`] that handles gRRC requests.
#[derive(Clone, Debug)]
pub struct GrpcLayer<H = DefaultSpanHandler> {
    handler: Arc<H>,
}

impl GrpcLayer {
    /// [`Span`]s are constructed at the given level from server side.
    pub fn server(level: Level) -> Self {
        Self::new(DefaultSpanHandler::server(level))
    }

    /// [`Span`]s are constructed at the given level from client side.
    pub fn client(level: Level) -> Self {
        Self::new(DefaultSpanHandler::client(level))
    }
}

impl<H> GrpcLayer<H> {
    /// Customize how to handle [`Span`] from request to response.
    pub fn new(handler: H) -> Self {
        Self {
            handler: Arc::new(handler),
        }
    }
}

impl<S, H> Layer<S> for GrpcLayer<H> {
    type Service = Grpc<S, H>;

    fn layer(&self, inner: S) -> Self::Service {
        Grpc {
            inner,
            handler: self.handler.clone(),
        }
    }
}

/// Middleware that adds tracing to a [`Service`] that handles gRPC requests.
#[derive(Clone, Debug)]
pub struct Grpc<S, H> {
    inner: S,
    handler: Arc<H>,
}

impl<S, H, ReqBody, ResBody> Service<Request<ReqBody>> for Grpc<S, H>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    H: SpanHandler<Request<ReqBody>, S::Response, S::Error>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, H>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let span = self.handler.make_span(&mut req);
        let inner = {
            let _enter = span.enter();
            self.inner.call(req)
        };

        ResponseFuture {
            inner,
            span,
            handler: self.handler.clone(),
        }
    }
}

/// Response future for [`Grpc`].
#[pin_project]
pub struct ResponseFuture<F, H> {
    #[pin]
    inner: F,
    span: Span,
    handler: Arc<H>,
}

impl<F, H, ResBody, E> Future for ResponseFuture<F, H>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    H: OnResponse<Response<ResBody>> + OnError<E>,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();

        match ready!(this.inner.poll(cx)) {
            Ok(response) => {
                this.handler.record_response(this.span, &response);
                Poll::Ready(Ok(response))
            }
            Err(err) => {
                this.handler.record_error(this.span, &err);
                Poll::Ready(Err(err))
            }
        }
    }
}

/// String representation of span kind
fn span_kind(kind: SpanKind) -> &'static str {
    match kind {
        SpanKind::Client => "client",
        SpanKind::Server => "server",
    }
}

/// Default implementation of [`SpanHandler`] trait for HTTP services.
#[derive(Clone, Debug)]
pub struct DefaultSpanHandler {
    level: Level,
    kind: SpanKind,
}

impl DefaultSpanHandler {
    /// [`Span`] are constructed at the given level from server side.
    pub fn server(level: Level) -> Self {
        Self {
            level,
            kind: SpanKind::Server,
        }
    }

    /// [`Span`] are constructed at the given level from client side.
    pub fn client(level: Level) -> Self {
        Self {
            level,
            kind: SpanKind::Client,
        }
    }
}

impl<B> OnRequest<Request<B>> for DefaultSpanHandler {
    fn make_span(&self, request: &mut Request<B>) -> tracing::Span {
        let Self { level, kind } = *self;

        macro_rules! make_span {
            ($level:expr) => {{
                use tracing::field::Empty;

                tracing::span!(
                    $level,
                    "GRPC",
                    "client.address" = Empty,
                    "client.port" = Empty,
                    "error.message" = Empty,
                    "otel.kind" = span_kind(kind),
                    "otel.name" = Empty,
                    "otel.status_code" = Empty,
                    "rpc.grpc.status_code" = Empty,
                    "rpc.method" = Empty,
                    "rpc.service" = Empty,
                    "rpc.system" = "grpc",
                    "server.address" = Empty,
                    "server.port" = Empty,
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

        for (header_name, header_value) in request.headers().iter() {
            if let Ok(attribute_value) = header_value.to_str() {
                let attribute_name = format!("rpc.grpc.request.metadata.{}", header_name);
                span.set_attribute(attribute_name, attribute_value.to_owned());
            }
        }

        let path = request.uri().path();
        let name = path.trim_start_matches('/');
        span.record("otel.name", name);
        if let Some((service, method)) = name.split_once('/') {
            span.record("rpc.service", service);
            span.record("rpc.method", method);
        }

        match kind {
            SpanKind::Client => {
                let util::HttpRequestAttributes {
                    server_address,
                    server_port,
                    ..
                } = util::HttpRequestAttributes::from_sent_request(request);

                if let Some(server_address) = server_address {
                    span.record("server.address", server_address);
                }
                if let Some(server_port) = server_port {
                    span.record("server.port", server_port);
                }

                #[cfg(feature = "propagate")]
                {
                    use crate::trace::propagate::HeaderInjector;

                    let context = span.context();
                    opentelemetry::global::get_text_map_propagator(|injector| {
                        injector
                            .inject_context(&context, &mut HeaderInjector(request.headers_mut()));
                    });
                }
            }
            SpanKind::Server => {
                if let Some(client_address) = util::client_address(request) {
                    let ip = client_address.ip();
                    span.record("client.address", tracing::field::display(ip));
                    span.record("client.port", client_address.port());
                }

                let util::HttpRequestAttributes {
                    server_address,
                    server_port,
                    ..
                } = util::HttpRequestAttributes::from_recv_request(request);

                if let Some(server_address) = server_address {
                    span.record("server.address", server_address);
                }
                if let Some(server_port) = server_port {
                    span.record("server.port", server_port);
                }

                #[cfg(feature = "propagate")]
                {
                    use crate::trace::propagate::HeaderExtractor;

                    let context = opentelemetry::global::get_text_map_propagator(|extractor| {
                        extractor.extract(&HeaderExtractor(request.headers_mut()))
                    });
                    if let Err(err) = span.set_parent(context) {
                        tracing::warn!("Failed to set parent span: {err}");
                    }
                }
            }
        }

        span
    }
}

impl<B> OnResponse<Response<B>> for DefaultSpanHandler {
    fn record_response(&self, span: &Span, response: &Response<B>) {
        for (header_name, header_value) in response.headers().iter() {
            if let Ok(attribute_value) = header_value.to_str() {
                let attribute_name = format!("rpc.grpc.response.metadata.{}", header_name);
                span.set_attribute(attribute_name, attribute_value.to_owned());
            }
        }

        if let Some(header_value) = response.headers().get("grpc-status") {
            if let Ok(header_value) = header_value.to_str() {
                if let Ok(status_code) = header_value.parse::<i32>() {
                    span.record("rpc.grpc.status_code", status_code);
                }
            }
        } else {
            span.record("rpc.grpc.status_code", 0);
        }
    }
}

impl<E> OnError<E> for DefaultSpanHandler
where
    E: Display,
{
    fn record_error(&self, span: &Span, err: &E) {
        span.record("otel.status_code", "ERROR");
        span.record("error.message", err.to_string());
    }
}
