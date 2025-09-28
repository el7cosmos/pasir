FROM rust:alpine AS builder

# Install SPC dependencies
RUN apk update; \
    apk upgrade -a; \
    apk add --no-cache \
        # Required by SPC \
        autoconf \
        automake \
        binutils-gold \
        bison \
        cmake \
        curl \
        flex \
        g++ \
        gettext-dev \
        git \
        libtool \
        linux-headers \
        make \
        patch \
        patchelf \
        re2c \
        # Required by Rust ecosystem \
        clang-dev \
        clang-static \
        compiler-rt \
        libxml2-static \
        lld \
        llvm-dev \
        llvm-static \
        ncurses-static \
        zlib-static \
        zstd-static

# Install static-php-cli (spc)
RUN curl -fsSL https://dl.static-php.dev/static-php-cli/spc-bin/nightly/spc-linux-$(uname -m) \
    -o /usr/local/bin/spc && \
    chmod +x /usr/local/bin/spc

ENV CC=clang \
    CXX=clang++ \
    LLVM_CONFIG_PATH=/pasir/llvm-config \
    BUILD_ROOT_PATH=/spc/buildroot \
    PKG_ROOT_PATH=/spc/pkgroot \
    SOURCE_PATH=/spc/source \
    DOWNLOAD_PATH=/spc/downloads \
    SPC_BUILD_EXTENSIONS_JSON=/spc/buildroot/ \
    PATH=/spc/buildroot/bin:$PATH

WORKDIR /spc

COPY craft.yml /spc/craft.yml
COPY patches /spc/patches

RUN --mount=type=secret,id=github_token,env=GITHUB_TOKEN spc doctor

RUN --mount=type=secret,id=github_token,env=GITHUB_TOKEN spc craft

WORKDIR /pasir

COPY . .

RUN cargo build \
        --bins \
        --features clang_static,static \
        --no-default-features \
        --release \
        --target $(rustup target list --installed); \
    cp target/$(rustup target list --installed)/release/pasir /usr/local/bin/pasir

FROM busybox:stable

WORKDIR /pasir

COPY --from=builder /usr/local/bin/pasir /usr/local/bin/pasir
