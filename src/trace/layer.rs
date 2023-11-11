use tower_http::trace::TraceLayer;
use tower_layer::Layer;

use super::{
    OtelMakeSpan, OtelOnBodyChunk, OtelOnEos, OtelOnFailure, OtelOnRequest, OtelOnResponse,
    OtelTrace,
};

/// [`Layer`] that adds OpenTelemetry tracing to a [`Service`].
///
/// [`Service`]: tower_service::Service
#[derive(Clone, Debug)]
pub struct OtelTraceLayer<M>(
    TraceLayer<
        M,
        OtelMakeSpan,
        OtelOnRequest,
        OtelOnResponse,
        OtelOnBodyChunk,
        OtelOnEos,
        OtelOnFailure,
    >,
);

impl<M> OtelTraceLayer<M> {
    /// Customize how to make [`Span`]s.
    ///
    /// [`Span`]: tracing::Span
    pub fn make_span_with(self, make_span: OtelMakeSpan) -> Self {
        Self(self.0.make_span_with(make_span))
    }
}

type HttpClassifier =
    tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>;

impl OtelTraceLayer<HttpClassifier> {
    /// Create a new [`OtelTraceLayer`] using [`ServerErrorsAsFailures`].
    ///
    /// [`ServerErrorsAsFailures`]: tower_http::classify::ServerErrorsAsFailures
    pub fn new_for_http() -> Self {
        Self(
            TraceLayer::new_for_http()
                .make_span_with(OtelMakeSpan::default())
                .on_request(OtelOnRequest)
                .on_response(OtelOnResponse)
                .on_body_chunk(OtelOnBodyChunk)
                .on_eos(OtelOnEos)
                .on_failure(OtelOnFailure),
        )
    }
}

impl<S, M> Layer<S> for OtelTraceLayer<M>
where
    M: Clone,
{
    type Service = OtelTrace<S, M>;

    #[inline(always)]
    fn layer(&self, inner: S) -> Self::Service {
        self.0.layer(inner)
    }
}
