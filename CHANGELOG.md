## v0.7.0

- Add `url.scheme` attribute to HTTP spans/metrics. For client request the
  value is extracted from the request URL. For server request the value is
  extracted from `X-Forwarded-Proto` and `Forwarded` headers.

- Add `server.address` and `server.port` attributes to HTTP spans/metrics and
  gRPC spans. For client request the value is extracted from the request URL.
  For server request the value is extracted from request following [these rules].

- Add `client.address` attributes to server spans when `axum` feature is
  enabled.

## v0.6.2

- Update [`prost`] and [`tonic`] to v0.14 in gRPC example
- Update lock file, this includes the update of [`slab`] to v0.4.11 (see [#153](https://github.com/tokio-rs/slab/pull/153))

## v0.6.0

- Updated OpenTelemetry to v0.30.0

## v0.5.0

- Add [axum](https://docs.rs/axum) support

## v0.4.0

- Add metrics for HTTP services


[`prost`]: https://crates.io/crates/prost
[`slab`]: https://crates.io/crates/slab
[`tonic`]: https://crates.io/crates/tonic

[these rules]: https://opentelemetry.io/docs/specs/semconv/http/http-spans/#setting-serveraddress-and-serverport-attributes
