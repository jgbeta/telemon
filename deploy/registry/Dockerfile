FROM rust:1.83-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=build /app/target/release/telemon-registry /usr/local/bin/telemon-registry
ENTRYPOINT ["telemon-registry"]
