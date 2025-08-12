# tower-otel

[![Latest Version](https://img.shields.io/crates/v/tower-otel.svg)](https://crates.io/crates/tower-otel)
[![Latest Version](https://docs.rs/tower-otel/badge.svg)](https://docs.rs/tower-otel)
![Apache 2.0 OR MIT licensed](https://img.shields.io/badge/license-Apache2.0%2FMIT-blue.svg)

This crate provides an OpenTelemetry layer for HTTP and gRPC services built on top of [`tower`]. 
The implementation is compliant to the semantic conventions defined for [HTTP spans](https://opentelemetry.io/docs/specs/semconv/http/) and  [RPC](https://opentelemetry.io/docs/specs/semconv/rpc/).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or 
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

[`tower`]: https://crates.io/crates/tower
