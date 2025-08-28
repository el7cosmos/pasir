ARG PASIR_VERSION=0.1.0
ARG PHP_VERSION=8.1
ARG RUST_VERSION=1
ARG VARIANT=bookworm

FROM rust:${RUST_VERSION}-slim-${VARIANT} AS rust-builder

FROM php:${PHP_VERSION}-zts-${VARIANT} AS php-builder

ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH

COPY --from=rust-builder /usr/local/cargo /usr/local/cargo
COPY --from=rust-builder /usr/local/rustup /usr/local/rustup

RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
        libclang-dev \
    ;

WORKDIR /app

COPY . .

RUN PASIR_VERSION=${PASIR_VERSION} cargo build --release

FROM php:${PHP_VERSION}-zts-${VARIANT}

ENV PASIR_PORT=8080

COPY --from=php-builder /app/target/release/pasir /usr/local/bin/pasir

RUN cp "$PHP_INI_DIR/php.ini-development" "$PHP_INI_DIR/php.ini"

WORKDIR /app

EXPOSE ${PASIR_PORT}

CMD ["pasir"]