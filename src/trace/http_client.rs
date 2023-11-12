use std::{
    error::Error as StdError,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
};

use http::{Request, Response, Version};
use opentelemetry_http::HeaderInjector;
use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// [`Layer`] that adds tracing on a [`Service`] making outgoing HTTP requests.
#[derive(Clone, Debug)]
pub struct HttpClientLayer {
    level: Level,
}

impl HttpClientLayer {
    /// [`Span`]s are constructed at the given level
    pub fn new(level: Level) -> Self {
        Self { level }
    }
}

impl<S> Layer<S> for HttpClientLayer {
    type Service = HttpClient<S>;

    fn layer(&self, inner: S) -> Self::Service {
        let level = self.level;

        HttpClient { inner, level }
    }
}

/// Middleware that adds on a [`Service`] making outgoing HTTP requests.
#[derive(Clone, Debug)]
pub struct HttpClient<S> {
    inner: S,
    level: Level,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for HttpClient<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Error: StdError,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let span = make_request_span(self.level, &mut req);
        let inner = {
            let _enter = span.enter();
            self.inner.call(req)
        };

        ResponseFuture { inner, span }
    }
}

#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    span: Span,
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    E: StdError,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _enter = this.span.enter();

        match ready!(this.inner.poll(cx)) {
            Ok(response) => {
                record_response(this.span, &response);
                Poll::Ready(Ok(response))
            }
            Err(err) => {
                record_error(this.span, &err);
                Poll::Ready(Err(err))
            }
        }
    }
}

/// String representing the network protocol version and injects `traceparent` header
fn network_protocol_version(version: Version) -> Option<&'static str> {
    match version {
        Version::HTTP_09 => Some("0.9"),
        Version::HTTP_10 => Some("1.0"),
        Version::HTTP_11 => Some("1.1"),
        Version::HTTP_2 => Some("2"),
        Version::HTTP_3 => Some("3"),
        _ => None,
    }
}

/// Creates a new [`Span`] for the given request.
fn make_request_span<B>(level: Level, request: &mut Request<B>) -> Span {
    macro_rules! make_span {
        ($level:expr) => {
            tracing::span!(
                $level,
                "HTTP request",
                "error.message" = tracing::field::Empty,
                "http.request.method" = %request.method(),
                "http.response.status_code" = tracing::field::Empty,
                "network.protocol.name" = "http",
                "network.protocol.version" = network_protocol_version(request.version()),
                "otel.status_code" = tracing::field::Empty,
                "url.full" = %request.uri(),
            )
        };
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
            let attribute_name = format!("http.request.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    let context = span.context();
    opentelemetry::global::get_text_map_propagator(|injector| {
        injector.inject_context(&context, &mut HeaderInjector(request.headers_mut()));
    });

    span
}

/// Records fields associated to the response.
fn record_response<B>(span: &Span, response: &Response<B>) {
    span.record(
        "http.response.status_code",
        response.status().as_u16() as i64,
    );

    for (header_name, header_value) in response.headers().iter() {
        if let Ok(attribute_value) = header_value.to_str() {
            let attribute_name = format!("http.response.header.{}", header_name);
            span.set_attribute(attribute_name, attribute_value.to_owned());
        }
    }

    // client errors are marked as errors
    if response.status().is_client_error() {
        span.record("otel.status_code", "ERROR");
    }
}

/// Records the error message.
fn record_error<E: StdError>(span: &Span, err: &E) {
    span.record("otel.status_code", "ERROR");
    span.record("error.message", err.to_string());
}
