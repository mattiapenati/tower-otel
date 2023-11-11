use tower_http::trace::OnRequest;
use tracing::Span;

/// [`OnRequest`] implementation used by [`OtelTrace`].
#[derive(Clone, Debug)]
pub struct OtelOnRequest;

impl<B> OnRequest<B> for OtelOnRequest {
    fn on_request(&mut self, _: &http::Request<B>, _: &Span) {}
}
