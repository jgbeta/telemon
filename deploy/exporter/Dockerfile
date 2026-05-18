FROM rust:1.83-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=build /app/target/release/telemon-exporter /usr/local/bin/telemon-exporter
COPY deploy/exporter/entrypoint.sh /usr/local/bin/telemon-exporter-entrypoint
RUN chmod 0755 /usr/local/bin/telemon-exporter /usr/local/bin/telemon-exporter-entrypoint \
  && mkdir -p /config /host/sys
EXPOSE 9185
ENTRYPOINT ["telemon-exporter-entrypoint"]
CMD ["run", "--config", "/config/generated-exporter.yml"]
