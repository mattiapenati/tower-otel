_default:
    @just -l

# start jaeger
jaeger-run:
    docker run --rm --detach \
        --name tower-otel-jaeger \
        --env COLLECTOR_OTLP_ENABLED=true \
        --publish 4317:4317 \
        --publish 4318:4318 \
        --publish 16686:16686 \
        jaegertracing/all-in-one:1.51

# kill jaeger
jaeger-kill:
    docker kill tower-otel-jaeger
