# Thin HDMI ASCII screensaver for the Talos Pi5 node "spruce".
#
# Renders cacafire (libcaca colour fire demo) onto the Linux framebuffer via
# fbterm. libcaca, like aalib, draws to a *terminal* (it has no framebuffer/KMS
# output driver), so fbterm provides a terminal painted onto /dev/fb0 and we run
# cacafire inside it with the ncurses driver.
#   - caca-utils : provides /usr/bin/cacafire (+ cacademo) — colour ASCII
#   - libaa-bin  : provides /usr/bin/aafire (kept as a mono fallback)
#   - fbterm     : framebuffer terminal emulator that paints to /dev/fb0
#   - fbset      : (fbset pkg) handy for fb diagnostics
#
# arm64-only cluster; build with --platform=linux/arm64.
FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
       caca-utils \
       libaa-bin \
       fbterm \
       fbset \
       ncurses-base \
  && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENV TERM=linux
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
