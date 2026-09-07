# HDMI fire screensaver for the Talos Pi5 node "fir".
#
# Renders a Doom-fire animation by writing pixels DIRECTLY into the mmap'd
# framebuffer (/dev/fb0). This replaces the old cacafire+fbterm+ncurses path,
# which rendered ASCII fire into a terminal that fbterm repainted onto the
# framebuffer — an indirect path that burned ~0.5 core for ~10 fps. Direct fb
# writes are the fast path (hundreds of MB/s on fir) and need no VT/terminal.
#
# arm64-only cluster; build with --platform=linux/arm64.

# --- build stage: compile the tiny C renderer ---
FROM debian:bookworm-slim AS build
RUN apt-get update \
  && apt-get install -y --no-install-recommends gcc libc6-dev libdrm-dev pkg-config \
  && rm -rf /var/lib/apt/lists/*
COPY fbfire.c /src/fbfire.c
# libdrm is needed because Talos v1.14.0 ships `# CONFIG_FB is not set`, so
# /dev/fbN does not exist on any node and DRM/KMS is the only output path.
#
# CFLAGS_EXTRA lets CI pass -Werror without making local iteration painful; see
# .github/workflows/screensaver-image.yml.
ARG CFLAGS_EXTRA=""
RUN gcc -O2 -Wall -Wextra ${CFLAGS_EXTRA} -o /usr/local/bin/fbfire /src/fbfire.c \
      $(pkg-config --cflags --libs libdrm)

# --- runtime stage: binary + entrypoint + diagnostics ---
# libdrm2 for the renderer; fbset and libdrm-tests(modetest) to make a broken
# display debuggable from inside the pod rather than by guesswork.
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends fbset libdrm2 \
  && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/fbfire /usr/local/bin/fbfire
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
