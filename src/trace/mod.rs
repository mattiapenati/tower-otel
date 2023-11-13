//! Middleware that adds tracing to a [`Service`].
//!
//! [`Service`]: tower_service::Service

#[doc(inline)]
pub use self::http::{Http, HttpLayer};

pub mod http;
