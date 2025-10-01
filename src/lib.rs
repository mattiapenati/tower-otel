/*!

# Crate features

* **axum** -
  Enables the [`axum`] integration. Trace and metrics will contain the
  `http.route` attribute, populated with the path in the router that matches
  the request, as well as the `client.address` and `client.port` attributes.

[`axum`]: https://docs.rs/axum

*/

pub mod metrics;
pub mod trace;
mod util;
