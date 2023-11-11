use std::time::Duration;

use http::HeaderMap;
use tower_http::trace::OnEos;
use tracing::Span;

/// [`OnEos`] implementation used by [`OtelTrace`].
#[derive(Clone, Debug)]
pub struct OtelOnEos;

impl OnEos for OtelOnEos {
    fn on_eos(self, _: Option<&HeaderMap>, _: Duration, _: &Span) {}
}
