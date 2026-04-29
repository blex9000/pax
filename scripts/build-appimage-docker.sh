#!/usr/bin/env bash
set -euo pipefail

# Build a portable Pax AppImage inside an Ubuntu 22.04 Docker container.
#
# Why not build directly on the host: AppImages don't bundle glibc /
# libstdc++, so the binary inherits the host's symbol versions. Building
# on Ubuntu 24.04 (glibc 2.39) produces an AppImage that fails on
# anything older with errors like
#   "GLIBC_2.39 not found", "GLIBCXX_3.4.31 not found".
#
# Ubuntu 22.04 ships glibc 2.35 — old enough for Debian 12, RHEL 9,
# Ubuntu 22.04+, Mint 21+. If you need to support older still, switch
# the base image to debian:11 (glibc 2.31).
#
# Requirements: docker (or podman aliased to docker).
# Output: target/Pax-x86_64.AppImage

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Pinned to 24.04 because Cargo.toml requires gtk4 v4_10 (GTK ≥ 4.10)
# and libadwaita v1_4 (≥ 1.4) feature flags — Ubuntu 22.04 only ships
# GTK 4.6 / libadwaita 1.0-1.2. Lowering the floor below 24.04 would
# require pulling GTK from a PPA or compiling it from source inside
# the container (slow). Override with PAX_APPIMAGE_BASE if you have
# a custom base that ships ≥ GTK 4.10 with older glibc.
BASE_IMAGE="${PAX_APPIMAGE_BASE:-ubuntu:24.04}"
IMAGE_TAG="pax-appimage-builder:$(echo "$BASE_IMAGE" | tr ':/' '--')"

if ! command -v docker &>/dev/null; then
    echo "Error: docker not found. Install docker (or podman aliased)." >&2
    exit 1
fi

# Build the builder image once. Subsequent runs reuse the layer cache.
echo "==> Ensuring builder image exists ($IMAGE_TAG, base $BASE_IMAGE)..."
docker build -t "$IMAGE_TAG" - <<DOCKERFILE
FROM $BASE_IMAGE

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        file \
        git \
        libgtk-4-dev \
        libadwaita-1-dev \
        libgtksourceview-5-dev \
        libvte-2.91-gtk4-dev \
        pkg-config \
        squashfs-tools \
        wget \
        zsync \
    && rm -rf /var/lib/apt/lists/*

# Rust toolchain — match local dev (cargo's MSRV check at build time
# will fail loudly if the host project bumps past what we install).
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --profile minimal
ENV PATH="/root/.cargo/bin:\$PATH"

WORKDIR /work
DOCKERFILE

# Mount the repo into /work and run build-appimage.sh inside.
echo "==> Running build-appimage.sh inside the container..."
docker run --rm \
    -v "$ROOT_DIR:/work" \
    -v pax-cargo-cache:/root/.cargo/registry \
    -v pax-target-cache:/work/target \
    -e CARGO_TARGET_DIR=/work/target \
    "$IMAGE_TAG" \
    bash -lc 'cd /work && scripts/build-appimage.sh'

echo ""
echo "==> Done. AppImage at: $ROOT_DIR/target/Pax-$(uname -m).AppImage"
echo "    Built against $BASE_IMAGE (glibc-2.35) — runs on Debian 12,"
echo "    Ubuntu 22.04+, RHEL 9+, Mint 21+, and any newer system."
