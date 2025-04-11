//! Middleware that adds metrics to a [`Service`].
//!
//! These middlewares follow the conventions defined by OpenTelemetry for [HTTP protocol] and [gRPC
//! protocol].
//!
//! [`Service`]: tower_service::Service
//! [HTTP protocol]: https://opentelemetry.io/docs/specs/semconv/http/http-metrics/
//! [gRPC protocol]: https://opentelemetry.io/docs/specs/semconv/rpc/rpc-metrics/

#[doc(inline)]
pub use self::http::{Http, HttpLayer};

mod http;
