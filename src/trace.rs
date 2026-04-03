//! Middleware that adds tracing to a [`Service`].
//!
//! These middlewares follow the conventions defined by OpenTelemetry for [HTTP protocol] and [gRPC
//! protocol].
//!
//! [`Service`]: tower_service::Service
//! [HTTP protocol]: https://opentelemetry.io/docs/specs/semconv/http/http-spans/
//! [gRPC protocol]: https://opentelemetry.io/docs/specs/semconv/rpc/grpc/

#[doc(inline)]
pub use self::{
    grpc::{Grpc, GrpcLayer},
    http::{Http, HttpLayer},
};

pub mod grpc;
pub mod http;

#[cfg(feature = "propagate")]
mod propagate;

/// Trait used to create a [`Span`] from new request.
///
/// [`Span`]: tracing::Span
pub trait OnRequest<Req> {
    /// Creates a new [`Span`] for the given request.
    ///
    /// [`Span`]: tracing::Span
    fn make_span(&self, request: &mut Req) -> tracing::Span;
}

/// Trait used to update a [`Span`] when a response has been created.
///
/// [`Span`]: tracing::Span
pub trait OnResponse<Res> {
    /// Records fields associated to the response.
    fn record_response(&self, span: &tracing::Span, response: &Res);
}

/// Trait used to update a [`Span`] when a request failed.
///
/// [`Span`]: tracing::Span
pub trait OnError<E> {
    /// Records the error message.
    fn record_error(&self, span: &tracing::Span, err: &E);
}

/// Trait used to handle a [`Span`] while handling a new request and the following response.
///
/// [`Span`]: tracing::Span
pub trait SpanHandler<Req, Res, E>: OnRequest<Req> + OnResponse<Res> + OnError<E> {}

impl<T, Req, Res, E> SpanHandler<Req, Res, E> for T where
    T: OnRequest<Req> + OnResponse<Res> + OnError<E>
{
}
