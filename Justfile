_default:
    @just -l

# start jaeger
jaeger-run:
    docker run --rm --detach \
        --name tower-otel-jaeger \
        jaegertracing/all-in-one:1.51

# kill jaeger
jaeger-kill:
    docker kill tower-otel-jaeger
