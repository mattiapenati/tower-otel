use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use pin_project::pin_project;
use tower_service::Service;

use crate::util::http_body_size_from_headers;

use super::{Http, MetricSide, MetricsRecord, RequestMetricData, ResponseMetricState};

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
        let data = RequestMetricData::from_reqwest(&req);
        let state = ResponseMetricState::new(data);
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

        this.state
            .push_response_attributes(inner_response.as_ref().map(|r| r.status()));

        let response_body_size = inner_response
            .as_ref()
            .ok()
            .and_then(|r| http_body_size_from_headers(r.headers()));
        this.state.record_metrics(this.record, response_body_size);

        Poll::Ready(inner_response)
    }
}

impl<'a> RequestMetricData<'a> {
    pub(super) fn from_reqwest(req: &'a reqwest::Request) -> Self {
        Self {
            method: req.method(),
            server_address: req.url().host_str(),
            server_port: req.url().port_or_known_default(),
            url_scheme: Some(req.url().scheme()),
            version: req.version(),
            http_route: None,
            body_size: http_body_size_from_headers(req.headers()),
        }
    }
}
