_default:
    @just -l

# start collector
collector-run:
  docker run --rm --detach \
      --name opentelemetry-collector \
      --publish 127.0.0.1:4317:4317 \
      --publish 127.0.0.1:4318:4318 \
      --publish 127.0.0.1:55679:55679 \
      otel/opentelemetry-collector-contrib


# kill collector
collector-kill:
  docker kill opentelemetry-collector
