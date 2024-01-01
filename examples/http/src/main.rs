use axum::{http::Request, routing::get, Router};
use hyper::Client;
use opentelemetry_sdk::{propagation::TraceContextPropagator, runtime::Tokio};
use tower::{Service, ServiceBuilder, ServiceExt};
use tower_otel::trace::HttpLayer;
use tracing::Level;
use tracing_subscriber::{
    filter::LevelFilter, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

#[tokio::main]
async fn main() {
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    const PKG_NAME: &str = env!("CARGO_PKG_NAME");
    const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

    let resources = vec![
        opentelemetry::KeyValue::new("service.name", PKG_NAME),
        opentelemetry::KeyValue::new("service.version", PKG_VERSION),
    ];

    let trace_config = opentelemetry_sdk::trace::config()
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
        .with_resource(opentelemetry_sdk::Resource::new(resources));

    let exporter = opentelemetry_otlp::new_exporter().tonic();
    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_trace_config(trace_config)
        .with_exporter(exporter)
        .install_batch(Tokio)
        .unwrap();

    let telemetry = tracing_opentelemetry::layer()
        .with_tracer(tracer)
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
        .layer(HttpLayer::server(Level::DEBUG));
    let server = axum::Server::bind(&"[::1]:3000".parse().unwrap()).serve(app.into_make_service());
    tokio::spawn(server);

    let mut client = ServiceBuilder::new()
        .layer(HttpLayer::client(Level::DEBUG))
        .service(Client::new());

    let req = Request::get("http://[::1]:3000")
        .body(Default::default())
        .unwrap();
    let res = client.ready().await.unwrap().call(req).await.unwrap();
    let body = hyper::body::to_bytes(res.into_body()).await.unwrap();
    let body = std::str::from_utf8(&body).unwrap();
    tracing::info!("received '{}'", body);

    opentelemetry::global::shutdown_tracer_provider();
}
