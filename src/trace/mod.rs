//! Middleware that adds tracing to a [`Service`].
//!
//! [`Service`]: tower_service::Service

use tracing::Level;

pub use self::{
    http_client::{HttpClient, HttpClientLayer},
    layer::OtelTraceLayer,
    make_span::OtelMakeSpan,
    on_body_chunk::OtelOnBodyChunk,
    on_eos::OtelOnEos,
    on_failure::OtelOnFailure,
    on_request::OtelOnRequest,
    on_response::OtelOnResponse,
};

pub mod http_client;
mod layer;
mod make_span;
mod on_body_chunk;
mod on_eos;
mod on_failure;
mod on_request;
mod on_response;

const DEFAULT_MESSAGE_LEVEL: Level = Level::DEBUG;

/// Middleware that adds OpenTelemetry tracing to a [`Service`].
///
/// [`Service`]: tower_service::Service
pub type OtelTrace<S, M> = tower_http::trace::Trace<
    S,
    M,
    OtelMakeSpan,
    OtelOnRequest,
    OtelOnResponse,
    OtelOnBodyChunk,
    OtelOnEos,
    OtelOnFailure,
>;
