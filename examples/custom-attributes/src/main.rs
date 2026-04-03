use std::{future::IntoFuture, net::SocketAddr};

use axum::{routing::get, Router};
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tower::{Service, ServiceBuilder, ServiceExt};
use tower_otel::trace::*;
use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{
    filter::LevelFilter, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

struct ReqwestSpanHandler(http::DefaultSpanHandler);

impl ReqwestSpanHandler {
    fn new(level: Level) -> Self {
        Self(http::DefaultSpanHandler::client(level))
    }
}

impl OnRequest<reqwest::Request> for ReqwestSpanHandler {
    fn make_span(&self, request: &mut reqwest::Request) -> tracing::Span {
        let span = self.0.make_span(request);
        span.set_attribute("url.template", "/");
        span
    }
}

impl OnResponse<reqwest::Response> for ReqwestSpanHandler {
    fn record_response(&self, span: &tracing::Span, response: &reqwest::Response) {
        self.0.record_response(span, response);
    }
}

impl OnError<reqwest::Error> for ReqwestSpanHandler {
    fn record_error(&self, span: &tracing::Span, err: &reqwest::Error) {
        self.0.record_error(span, err);
    }
}

#[tokio::main]
async fn main() {
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    const PKG_NAME: &str = env!("CARGO_PKG_NAME");
    const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

    let resource = opentelemetry_sdk::Resource::builder()
        .with_service_name(PKG_NAME)
        .with_attribute(opentelemetry::KeyValue::new("service.version", PKG_VERSION))
        .build();

    let span_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .unwrap();

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
        .with_resource(resource.clone())
        .with_batch_exporter(span_exporter)
        .build();

    let telemetry = tracing_opentelemetry::layer()
        .with_tracer(tracer_provider.tracer("default_tracer"))
        .with_tracked_inactivity(true)
        .with_filter(LevelFilter::TRACE);

    let fmt = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(LevelFilter::from_level(Level::TRACE))
        .with(telemetry)
        .with(fmt)
        .init();

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .into_make_service_with_connect_info::<SocketAddr>();
    let listener = tokio::net::TcpListener::bind("[::1]:3000").await.unwrap();
    let server = axum::serve(listener, app).into_future();
    tokio::spawn(server);

    let mut client = ServiceBuilder::new()
        .layer(HttpLayer::new(ReqwestSpanHandler::new(Level::DEBUG)))
        .service(reqwest::Client::new());
    let req = reqwest::Request::new(
        reqwest::Method::GET,
        "http://[::1]:3000".try_into().unwrap(),
    );
    let res = client.ready().await.unwrap().call(req).await.unwrap();
    let body = res.text().await.unwrap();
    tracing::info!("received '{}'", body);

    tracer_provider.shutdown().unwrap();
}
