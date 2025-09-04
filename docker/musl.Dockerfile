ARG PASIR_VERSION=0.0.0

FROM rust:alpine AS builder

# Install dependencies
RUN apk update; \
    apk upgrade -a; \
    apk add --no-cache \
        autoconf \
        automake \
        bash \
        binutils \
        binutils-gold \
        bison \
        build-base \
        clang-dev \
        clang-static \
        cmake \
        compiler-rt \
        curl \
        file \
        flex \
        g++ \
        gcc \
        gettext \
        gettext-dev \
        git \
        jq \
        libgcc \
        libstdc++ \
        libtool \
        libxml2-static \
        linux-headers \
        lld \
        llvm-dev \
        llvm-static \
        m4 \
        make \
        ncurses-static \
        patchelf \
        pkgconfig \
        re2c \
        wget \
        xz \
        zlib-static \
        zstd-static

# Install static-php-cli (spc)
RUN curl -fsSL https://dl.static-php.dev/static-php-cli/spc-bin/nightly/spc-linux-$(uname -m) \
    -o /usr/local/bin/spc && \
    chmod +x /usr/local/bin/spc

ENV CC=clang \
    CXX=clang++ \
    LLVM_CONFIG_PATH=/pasir/llvm-config \
    PATH=/pasir/buildroot/bin:$PATH

WORKDIR /pasir

COPY . .

RUN spc doctor

RUN --mount=type=secret,id=github_token,env=GITHUB_TOKEN \
    --mount=type=cache,target=/pasir/downloads \
    --mount=type=cache,target=/pasir/pkgroot \
    spc craft

RUN PASIR_VERSION=${PASIR_VERSION} cargo build --features clang_static,static --no-default-features --release --target $(uname -m)-unknown-linux-musl; \
    cp target/$(uname -m)-unknown-linux-musl/release/pasir /usr/local/bin/pasir

FROM alpine:latest

WORKDIR /pasir

COPY --from=builder /usr/local/bin/pasir /usr/local/bin/pasir
