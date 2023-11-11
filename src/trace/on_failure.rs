use std::time::Duration;

use tower_http::trace::OnFailure;
use tracing::Span;

/// [`OnFailure`] implementation used by [`OtelTrace`].
#[derive(Clone, Debug)]
pub struct OtelOnFailure;

impl<FailureClass> OnFailure<FailureClass> for OtelOnFailure {
    fn on_failure(&mut self, _: FailureClass, _: Duration, _: &Span) {}
}
