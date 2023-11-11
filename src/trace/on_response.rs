use std::time::Duration;

use tower_http::trace::OnResponse;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// [`OnResponse`] implementation used by [`OtelTrace`].
///
/// [`OtelTrace`]: super::OtelTrace
#[derive(Clone, Debug)]
pub struct OtelOnResponse;

impl<B> OnResponse<B> for OtelOnResponse {
    fn on_response(self, response: &http::Response<B>, _latency: Duration, span: &Span) {
        span.record(
            "http.response.status_code",
            response.status().as_u16() as i64,
        );

        for (header_name, header_value) in response.headers().iter() {
            if let Ok(attribute_value) = header_value.to_str() {
                let attribute_name = format!("http.response.header.{}", header_name);
                span.set_attribute(attribute_name, attribute_value.to_owned());
            }
        }
    }
}
