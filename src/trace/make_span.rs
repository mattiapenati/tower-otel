use tower_http::trace::MakeSpan;
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::DEFAULT_MESSAGE_LEVEL;

/// [`MakeSpan`] implementation used by [`OtelTrace`].
///
/// [`OtelTrace`]: super::OtelTrace
#[derive(Clone, Debug)]
pub struct OtelMakeSpan {
    level: Level,
}

impl Default for OtelMakeSpan {
    fn default() -> Self {
        Self::new(DEFAULT_MESSAGE_LEVEL)
    }
}

impl OtelMakeSpan {
    /// Set the [`Level`] used for [`Span`]s.
    pub fn new(level: Level) -> Self {
        Self { level }
    }
}

impl<B> MakeSpan<B> for OtelMakeSpan {
    fn make_span(&mut self, request: &http::Request<B>) -> Span {
        macro_rules! make_span {
        ($level:expr) => {{
            tracing::span!(
                $level,
                "request",
                "http.request.method" = %request.method(),
                "http.response.status_code" = tracing::field::Empty,
                "url.path" = request.uri().path(),
                "url.query" = request.uri().query(),
            )
        }};
    }

        let span = match self.level {
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

        span
    }
}
