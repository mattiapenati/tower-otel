/*!

# Crate features

* **propagate** -
  Enables the distributed tracing context propagation, linking spans across service boundaries.
  When enabled the trace context is injected into outgoing requests, and extracted from incoming
  requests. Enabled by default.

* **axum** -
  Enables the [`axum`] integration. Trace and metrics will contain the
  `http.route` attribute, populated with the path in the router that matches
  the request, as well as the `client.address` and `client.port` attributes.

* **reqwest_013** -
  Enables support for [`reqwest`] v0.13. Services [`trace::Http`] and [`metrics::Http`] can be
  used with `reqwest::Client`. Applying a server side service results in a panic.

[`axum`]: https://docs.rs/axum
[`reqwest`]: https://docs.rs/reqwest

*/

pub mod metrics;
pub mod trace;
mod util;
