use std::time::Duration;

use tower_http::trace::OnBodyChunk;
use tracing::Span;

/// [`OnBodyChunk`] implementation used by [`OtelTrace`].
#[derive(Clone, Debug)]
pub struct OtelOnBodyChunk;

impl<B> OnBodyChunk<B> for OtelOnBodyChunk {
    fn on_body_chunk(&mut self, _: &B, _: Duration, _: &Span) {}
}
