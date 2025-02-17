use std::{
    future::{Future, IntoFuture},
    pin::Pin,
    task::{ready, Context, Poll},
};

use axum::{http::Request, routing::get, Router};
use http_body_util::{BodyExt, Empty};
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::{propagation::TraceContextPropagator, runtime::Tokio};
use pin_project::pin_project;
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
    let listener = tokio::net::TcpListener::bind("[::1]:3000").await.unwrap();
    let server = axum::serve(listener, app).into_future();
    tokio::spawn(server);

    let tcp = tokio::net::TcpStream::connect("[::1]:3000").await.unwrap();
    let io = TokioIo(tcp);
    let (request_sender, connection) = hyper::client::conn::http1::handshake(io).await.unwrap();
    tokio::spawn(connection);
    let mut client = ServiceBuilder::new()
        .layer(HttpLayer::client(Level::DEBUG))
        .service(Client(request_sender));

    let req = Request::get("http://[::1]:3000")
        .body(Empty::<&[u8]>::new())
        .unwrap();
    let res = client.ready().await.unwrap().call(req).await.unwrap();
    let body = res.collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&body).unwrap();
    tracing::info!("received '{}'", body);

    opentelemetry::global::shutdown_tracer_provider();
}

struct Client<B>(hyper::client::conn::http1::SendRequest<B>);

impl<B> tower::Service<hyper::Request<B>> for Client<B>
where
    B: hyper::body::Body + 'static,
{
    type Response = hyper::Response<hyper::body::Incoming>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(cx)
    }

    fn call(&mut self, req: hyper::Request<B>) -> Self::Future {
        Box::pin(self.0.send_request(req))
    }
}

#[pin_project]
struct TokioIo<T>(#[pin] T);

impl<T> hyper::rt::Read for TokioIo<T>
where
    T: tokio::io::AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let n = {
            let this = self.project();
            let mut buf = tokio::io::ReadBuf::uninit(unsafe { buf.as_mut() });
            ready!(this.0.poll_read(cx, &mut buf))?;
            buf.filled().len()
        };
        unsafe { buf.advance(n) };
        Poll::Ready(Ok(()))
    }
}

impl<T> hyper::rt::Write for TokioIo<T>
where
    T: tokio::io::AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.project();
        this.0.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        this.0.poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.project();
        this.0.poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.project();
        this.0.poll_write_vectored(cx, bufs)
    }
}
