FROM ubuntu:24.04

ARG NODE_VERSION=22.14.0
ARG PNPM_VERSION=10.28.0
ARG RUST_VERSION=1.97.1

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/opt/cargo \
    RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:/usr/local/bin:$PATH

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        curl \
        file \
        git \
        build-essential \
        pkg-config \
        perl \
        python3 \
        libgtk-3-dev \
        libwebkit2gtk-4.1-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libssl-dev \
        libsqlite3-dev \
        libxdo-dev \
        libdbus-1-dev \
        libudev-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --fail --silent --show-error --location \
        "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
        | tar --extract --xz --directory /usr/local --strip-components=1 \
    && corepack enable \
    && corepack prepare "pnpm@${PNPM_VERSION}" --activate

RUN curl --proto '=https' --tlsv1.2 --fail --silent --show-error https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain none \
    && rustup toolchain install "${RUST_VERSION}" --profile minimal \
    && rustup component add rustfmt clippy --toolchain "${RUST_VERSION}"

WORKDIR /workspace
COPY . .

RUN pnpm install --frozen-lockfile

# The image is a reproducible, headless build/test environment. A GUI build
# still needs a macOS or Linux desktop runner outside the container.
CMD ["pnpm", "test"]
