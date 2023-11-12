//! Middleware that adds tracing to a [`Service`].
//!
//! [`Service`]: tower_service::Service

#[doc(inline)]
pub use self::{
    http_client::{HttpClient, HttpClientLayer},
    http_server::{HttpServer, HttpServerLayer},
};

pub mod http_client;
pub mod http_server;
