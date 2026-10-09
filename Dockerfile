# HDMI screensaver for whichever Talos Pi5 holds the monitor.
#
# Draws an animation (Doom fire, Matrix rain) straight into a DRM/KMS dumb
# buffer on /dev/dri/card0. It does NOT use /dev/fb0: Talos v1.14.0 builds its
# kernel with `# CONFIG_FB is not set`, so fbdev does not exist on any node and
# no device tree overlay can bring it back. See src/main.rs for the reasoning.
#
# arm64-only cluster; build with --platform=linux/arm64.
#
# The runtime stage is FROM scratch. This pod runs privileged with the host's
# /dev mounted, and the previous C version carried ~80 MB of Debian userland
# (plus libdrm and fbset) to host a 72 KB program. The `drm` crate issues raw
# ioctls, so a musl build links nothing but libc and the image is the binary
# alone — roughly 2 MB and no package surface at all.
#
# Trade-off worth knowing: there is no shell, so `kubectl exec … ls /dev/dri` is
# gone. That is how the missing CONFIG_FB was found in the first place. The
# binary prints its own device diagnostics on failure, and `kubectl debug` with
# an ephemeral container covers the rest.

# --- build stage: static musl binary ---
FROM rust:1-alpine AS build
# musl-dev for the C runtime musl-gcc needs; no libdrm, the crate does ioctls.
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
# --locked so the committed Cargo.lock is authoritative; a drifting dependency
# should fail the build rather than silently ship something else.
RUN cargo build --release --locked

# --- the -doom variant: doomgeneric compiled in, so GPL as a whole ---
# `docker build --target doom`. The default target below never copies doom/,
# so the MIT image cannot pick up a byte of it.
FROM rust:1-alpine AS build-doom
# gcc and binutils (nm) for build.rs, which compiles the engine four times.
RUN apk add --no-cache musl-dev gcc binutils
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY doom ./doom
RUN cargo build --release --locked --features doom

FROM alpine:3 AS freedoom
RUN apk add --no-cache curl
COPY tools/freedoom.sh /freedoom.sh
RUN /freedoom.sh /out

FROM scratch AS doom
COPY --from=build-doom /src/target/release/screensaver /screensaver
COPY --from=freedoom /out/freedoom1.wad /freedoom1.wad
COPY --from=freedoom /out/COPYING.txt /licenses/freedoom-COPYING.txt
COPY doom/doomgeneric/LICENSE /licenses/doomgeneric-GPL-2.0.txt
COPY LICENSE /licenses/afterglow-MIT.txt
ENTRYPOINT ["/screensaver"]

# --- runtime stage: just the binary. Last, so it is the default target ---
FROM scratch
COPY --from=build /src/target/release/screensaver /screensaver
ENTRYPOINT ["/screensaver"]
