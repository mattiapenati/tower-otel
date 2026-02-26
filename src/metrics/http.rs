//! Middleware that adds metrics to a [`Service`] that handles HTTP requests.

#[cfg(feature = "reqwest_013")]
mod reqwest;

use std::{
    fmt::Display,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
    time::Instant,
};

use http::{Request, Response};
use http_body::Body;
use opentelemetry::{
    metrics::{Histogram, Meter, UpDownCounter},
    KeyValue,
};
use pin_project::pin_project;
use tower_layer::Layer;
use tower_service::Service;

use crate::util;

/// The side from which metrics are recorded.
#[derive(Clone, Copy, Debug)]
enum MetricSide {
    /// The span describes a request sent to some remote service.
    Client,
    /// The span describes the server-side handling of a request.
    Server,
}

#[derive(Debug)]
struct MetricsRecord {
    side: MetricSide,
    request_duration: Histogram<f64>,
    active_requests: UpDownCounter<i64>,
    request_body_size: Histogram<u64>,
    response_body_size: Histogram<u64>,
}

impl MetricsRecord {
    fn server(meter: &Meter) -> Self {
        Self {
            side: MetricSide::Server,
            request_duration: meter
                .f64_histogram("http.server.request.duration")
                .with_description("Duration of HTTP server requests")
                .with_unit("s")
                .with_boundaries(vec![
                    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
                ])
                .build(),
            active_requests: meter
                .i64_up_down_counter("http.server.active_requests")
                .with_description("Number of active HTTP server requests")
                .with_unit("{request}")
                .build(),
            request_body_size: meter
                .u64_histogram("http.server.request.body.size")
                .with_description("Size of HTTP server request body")
                .with_unit("By")
                .build(),
            response_body_size: meter
                .u64_histogram("http.server.response.body.size")
                .with_description("Size of HTTP server response body")
                .with_unit("By")
                .build(),
        }
    }

    fn client(meter: &Meter) -> Self {
        Self {
            side: MetricSide::Client,
            request_duration: meter
                .f64_histogram("http.client.request.duration")
                .with_description("Duration of HTTP client requests")
                .with_unit("s")
                .with_boundaries(vec![
                    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
                ])
                .build(),
            request_body_size: meter
                .u64_histogram("http.client.request.body.size")
                .with_description("Size of HTTP client request body")
                .with_unit("By")
                .build(),
            response_body_size: meter
                .u64_histogram("http.client.response.body.size")
                .with_description("Size of HTTP client response body")
                .with_unit("By")
                .build(),
            active_requests: meter
                .i64_up_down_counter("http.client.active_requests")
                .with_description("Number of active HTTP client requests")
                .with_unit("{request}")
                .build(),
        }
    }
}

/// [`Layer`] that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct HttpLayer {
    record: Arc<MetricsRecord>,
}

impl HttpLayer {
    /// Metrics are recorded from server side.
    pub fn server(meter: &Meter) -> Self {
        let record = MetricsRecord::server(meter);
        Self {
            record: Arc::new(record),
        }
    }

    /// Metrics are recorded from client side.
    pub fn client(meter: &Meter) -> Self {
        let record = MetricsRecord::client(meter);
        Self {
            record: Arc::new(record),
        }
    }
}

impl<S> Layer<S> for HttpLayer {
    type Service = Http<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Http {
            inner,
            record: Arc::clone(&self.record),
        }
    }
}

/// Middleware that adds tracing to a [`Service`] that handles HTTP requests.
#[derive(Clone, Debug)]
pub struct Http<S> {
    inner: S,
    record: Arc<MetricsRecord>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for Http<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    S::Error: Display,
    ReqBody: Body,
    ResBody: Body,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let data = RequestMetricData::from_http(self.record.side, &req);
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

/// Response future for [`Http`].
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    inner: F,
    record: Arc<MetricsRecord>,
    state: ResponseMetricState,
}

impl<F, ResBody, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<ResBody>, E>>,
    ResBody: Body,
    E: Display,
{
    type Output = Result<Response<ResBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        let inner_response = ready!(this.inner.poll(cx));

        this.state
            .push_response_attributes(inner_response.as_ref().map(|r| r.status()));

        let response_body_size = inner_response
            .as_ref()
            .ok()
            .and_then(util::http_response_size);
        this.state.record_metrics(this.record, response_body_size);

        Poll::Ready(inner_response)
    }
}

/// Data extracted from an HTTP request used to build metric attributes.
struct RequestMetricData<'a> {
    method: &'a http::Method,
    server_address: Option<&'a str>,
    server_port: Option<u16>,
    url_scheme: Option<&'a str>,
    version: http::Version,
    http_route: Option<&'a str>,
    body_size: Option<u64>,
}

impl<'a> RequestMetricData<'a> {
    fn from_http<B: Body>(side: MetricSide, req: &'a Request<B>) -> Self {
        let util::HttpRequestAttributes {
            url_scheme,
            server_address,
            server_port,
        } = match side {
            MetricSide::Client => util::HttpRequestAttributes::from_sent_request(req),
            MetricSide::Server => util::HttpRequestAttributes::from_recv_request(req),
        };

        Self {
            method: req.method(),
            server_address,
            server_port,
            url_scheme,
            version: req.version(),
            http_route: util::http_route(req),
            body_size: util::http_request_size(req),
        }
    }
}

struct ResponseMetricState {
    start: Instant,
    /// The size of the request body.
    request_body_size: Option<u64>,
    /// Attributes to add to the metrics.
    attributes: Vec<KeyValue>,
    /// The number of attributes that are used for only for active requests counter.
    active_requests_attributes: usize,
}

impl ResponseMetricState {
    fn new(data: RequestMetricData<'_>) -> Self {
        let start = Instant::now();
        let request_body_size = data.body_size;

        let active_requests_attributes;
        let attributes = {
            let mut attributes = vec![];

            attributes.push(KeyValue::new(
                "http.request.method",
                util::http_method(data.method),
            ));

            if let Some(server_address) = data.server_address {
                attributes.push(KeyValue::new("server.address", server_address.to_string()));
            }
            if let Some(server_port) = data.server_port {
                attributes.push(KeyValue::new("server.port", server_port as i64));
            }
            if let Some(url_scheme) = data.url_scheme {
                attributes.push(KeyValue::new("url.scheme", url_scheme.to_string()));
            }

            active_requests_attributes = attributes.len();

            attributes.push(KeyValue::new("network.protocol.name", "http"));

            if let Some(version) = util::http_version(data.version) {
                attributes.push(KeyValue::new("network.protocol.version", version));
            }

            if let Some(http_route) = data.http_route {
                attributes.push(KeyValue::new("http.route", http_route.to_string()));
            }

            attributes
        };

        Self {
            start,
            request_body_size,
            attributes,
            active_requests_attributes,
        }
    }

    fn push_response_attributes<E>(&mut self, res: Result<http::StatusCode, &E>)
    where
        E: Display,
    {
        match res {
            Ok(status) => {
                self.attributes.push(KeyValue::new(
                    "http.response.status_code",
                    status.as_u16() as i64,
                ));
            }
            Err(err) => {
                self.attributes
                    .push(KeyValue::new("error.type", err.to_string()));
            }
        }
    }

    fn record_metrics(&self, record: &MetricsRecord, response_body_size: Option<u64>) {
        let duration = self.start.elapsed().as_secs_f64();

        record.request_duration.record(duration, self.attributes());

        record
            .active_requests
            .add(-1, self.active_requests_attributes());

        if let Some(request_body_size) = self.request_body_size {
            record
                .request_body_size
                .record(request_body_size, self.attributes());
        }

        if let Some(response_size) = response_body_size {
            record
                .response_body_size
                .record(response_size, self.attributes());
        }
    }

    /// Return the attributes for each metric.
    fn attributes(&self) -> &[KeyValue] {
        &self.attributes[..]
    }

    /// Returns the attributes used for active requests counter.
    fn active_requests_attributes(&self) -> &[KeyValue] {
        &self.attributes[..self.active_requests_attributes]
    }
}
