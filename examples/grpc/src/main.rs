use opentelemetry::trace::TracerProvider;
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

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
        .unwrap();

    let tracer_provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
        .with_resource(opentelemetry_sdk::Resource::new(resources))
        .with_batch_exporter(exporter, Tokio)
        .build();

    let telemetry = tracing_opentelemetry::layer()
        .with_tracer(tracer_provider.tracer("default_tracer"))
        .with_tracked_inactivity(true)
        .with_filter(LevelFilter::INFO);

    let fmt = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE);

    tracing_subscriber::registry()
        .with(LevelFilter::from_level(Level::INFO))
        .with(telemetry)
        .with(fmt)
        .init();

    let addr = "[::1]:50051".parse().unwrap();
    let greeter = MyGreeter;
    let server = Server::builder()
        .layer(GrpcLayer::server(Level::INFO))
        .add_service(GreeterServer::new(greeter))
        .serve(addr);
    tokio::spawn(server);

    let channel = Channel::from_static("http://[::1]:50051").connect_lazy();
    let channel = tower::ServiceBuilder::new()
        .layer(GrpcLayer::client(Level::INFO))
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
