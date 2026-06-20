# Thin HDMI ASCII screensaver for the Talos Pi5 node "spruce".
#
# Renders aafire (aalib fire demo) onto the Linux framebuffer via fbterm.
#   - libaa-bin  : provides /usr/bin/aafire
#   - fbterm     : framebuffer terminal emulator that paints to /dev/fb0
#   - fbset      : (fbset pkg) handy for fb diagnostics
#
# arm64-only cluster; build with --platform=linux/arm64.
FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
       libaa-bin \
       fbterm \
       fbset \
       ncurses-base \
  && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENV TERM=linux
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
