use opentelemetry_sdk::{propagation::TraceContextPropagator, runtime::Tokio};
use tonic::{
    transport::{Channel, Server},
    Request, Response, Status,
};

use hello_world::{
    greeter_client::GreeterClient,
    greeter_server::{Greeter, GreeterServer},
    {HelloReply, HelloRequest},
};
use tower_otel::trace::GrpcLayer;
use tracing::{level_filters::LevelFilter, Level};
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

mod hello_world {
    tonic::include_proto!("helloworld");
}

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

    let addr = "[::1]:50051".parse().unwrap();
    let greeter = MyGreeter;
    let server = Server::builder()
        .layer(GrpcLayer::server(Level::DEBUG))
        .add_service(GreeterServer::new(greeter))
        .serve(addr);
    tokio::spawn(server);

    let channel = Channel::from_static("http://[::1]:50051").connect_lazy();
    let channel = tower::ServiceBuilder::new()
        .layer(GrpcLayer::client(Level::DEBUG))
        .service(channel);
    let mut client = GreeterClient::new(channel);
    let request = tonic::Request::new(HelloRequest {
        name: "Tonic".into(),
    });
    let response = client.say_hello(request).await.unwrap();
    let body = response.into_inner().message;
    tracing::info!("received '{}'", body);

    opentelemetry::global::shutdown_tracer_provider();
}

#[derive(Debug)]
pub struct MyGreeter;

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let reply = hello_world::HelloReply {
            message: format!("Hello {}!", request.into_inner().name),
        };

        Ok(Response::new(reply))
    }
}
