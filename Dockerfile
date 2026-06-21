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
  && apt-get install -y --no-install-recommends gcc libc6-dev \
  && rm -rf /var/lib/apt/lists/*
COPY fbfire.c /src/fbfire.c
RUN gcc -O2 -Wall -Wextra -o /usr/local/bin/fbfire /src/fbfire.c

# --- runtime stage: just the binary + entrypoint + fbset for diagnostics ---
FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends fbset \
  && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/fbfire /usr/local/bin/fbfire
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
