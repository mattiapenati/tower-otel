//! Middleware that adds tracing to a [`Service`].
//!
//! [`Service`]: tower_service::Service

#[doc(inline)]
pub use self::{
    grpc::{Grpc, GrpcLayer},
    http::{Http, HttpLayer},
};

mod extractor;
pub mod grpc;
pub mod http;
mod injector;
