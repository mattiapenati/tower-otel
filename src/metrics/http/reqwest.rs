use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
    time::Instant,
};

use opentelemetry::KeyValue;
use pin_project::pin_project;
use tower_service::Service;

use super::{Http, MetricSide, MetricsRecord, ResponseMetricState};
use crate::util;

impl<S> Service<reqwest::Request> for Http<S>
where
    S: Service<reqwest::Request, Response = reqwest::Response>,
    S::Error: Display,
{
    type Response = reqwest::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: reqwest::Request) -> Self::Future {
        debug_assert!(
            matches!(self.record.side, MetricSide::Client),
            "Http metrics middleware with reqwest::Request only supports client-side metrics"
        );
        let state = reqwest_metric_state(&req);
        let record = Arc::clone(&self.record);
        let inner = self.inner.call(req);

        record
            .active_requests
            .add(1, state.active_requests_attributes());

        ResponseFuture {
            inner,
            record,
            state,
        }
    }
}

/// Response future for [`Http`] metrics middleware when wrapping a reqwest service.
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    record: Arc<MetricsRecord>,
    state: ResponseMetricState,
}

impl<F, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<reqwest::Response, E>>,
    E: Display,
{
    type Output = Result<reqwest::Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let inner_response = ready!(this.inner.poll(cx));
        let duration = this.state.elapsed_seconds();

        // Push response attributes (can't use push_response_attributes since it expects
        // http::Response<B>).
        match &inner_response {
            Ok(response) => {
                this.state.attributes.push(KeyValue::new(
                    "http.response.status_code",
                    response.status().as_u16() as i64,
                ));
            }
            Err(err) => {
                this.state
                    .attributes
                    .push(KeyValue::new("error.type", err.to_string()));
            }
        }

        this.record
            .request_duration
            .record(duration, this.state.attributes());

        this.record
            .active_requests
            .add(-1, this.state.active_requests_attributes());

        if let Some(request_body_size) = this.state.request_body_size {
            this.record
                .request_body_size
                .record(request_body_size, this.state.attributes());
        }

        if let Ok(response) = inner_response.as_ref() {
            if let Some(response_size) = response_content_length(response) {
                this.record
                    .response_body_size
                    .record(response_size, this.state.attributes());
            }
        }

        Poll::Ready(inner_response)
    }
}

/// Build a [`ResponseMetricState`] from a reqwest client request.
fn reqwest_metric_state(req: &reqwest::Request) -> ResponseMetricState {
    let start = Instant::now();

    let request_body_size = req
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let active_requests_attributes;
    let attributes = {
        let mut attributes = vec![];

        let http_method = util::http_method(req.method());
        attributes.push(KeyValue::new("http.request.method", http_method));

        if let Some(server_address) = req.url().host_str() {
            attributes.push(KeyValue::new("server.address", server_address.to_string()));
        }

        if let Some(server_port) = req.url().port_or_known_default() {
            attributes.push(KeyValue::new("server.port", server_port as i64));
        }

        attributes.push(KeyValue::new("url.scheme", req.url().scheme().to_string()));

        active_requests_attributes = attributes.len();

        attributes.push(KeyValue::new("network.protocol.name", "http"));

        if let Some(http_version) = util::http_version(req.version()) {
            attributes.push(KeyValue::new("network.protocol.version", http_version));
        }

        attributes
    };

    ResponseMetricState {
        start,
        request_body_size,
        attributes,
        active_requests_attributes,
    }
}

/// Read response body size from the `content-length` header.
fn response_content_length(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}
