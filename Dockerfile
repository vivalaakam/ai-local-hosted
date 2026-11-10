# Builds the alh-server binary from the workspace. Migrations are embedded
# into the binary via include_str!, so the migrations/ directory is needed
# for compilation but not at runtime.
FROM rust:1-slim-bookworm AS build

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY apps ./apps
COPY migrations ./migrations

RUN cargo build --release --locked -p alh-server

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --user-group --shell /usr/sbin/nologin alh

COPY --from=build /src/target/release/alh-server /usr/local/bin/alh-server

USER alh

# Required at runtime: DATABASE_URL (see .env.example).
ENV BIND=0.0.0.0:8080
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/alh-server"]
