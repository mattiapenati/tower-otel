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

mod extractor;
pub mod grpc;
pub mod http;
mod injector;
