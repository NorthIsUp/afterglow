# `lighthouse`

A banded lighthouse on a heap of rocks at night. Its beam turns round the lantern, long when it crosses the frame and a flash when it faces us, lighting the haze and the waves beneath; surf bursts on the rocks.

One of ascii.rest's character pieces, drawn in one ink. The beam makes the twin's full turn (below); upstream's beam always passes behind the tower, which `LIGHTHOUSE_BEAM_FRONT=0` restores. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

Source: [`src/ascii_rest/lighthouse.rs`](../../src/ascii_rest/lighthouse.rs).

## `lighthouse-wide`

The same night scaled to the panel's height, with the tower centred. The sea, the horizon and the sky run out to both edges, and the beam reaches far enough to sweep across the whole width and light the waves under it.

The beam makes a full turn. On the half that faces us it swings across in front of the lantern, a little brighter, and when it points straight at us it blooms into a round flare over the tower's top, with a star of rays on the lamp. On the far half it goes behind the tower, narrower and dimmer, hidden by the lantern and the roof.

In colour: the beam is amber, fading to bronze in the thin haze, and the lamp is pale gold. The tower is white, banded red, with an iron gallery and roof, on grey-brown rocks. The sea is two blues, with the beam's road on it in amber, and the surf is white. The stars are blue-white and the sky deep navy.

Knobs: `LIGHTHOUSE_BEAM_FRONT` (0..1, default 1): 0 sends the beam behind the tower all the way round, as upstream does. Also `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [full-screen twins](../ascii-rest.md#full-screen-twins).

Source: [`src/ascii_rest/lighthouse.rs`](../../src/ascii_rest/lighthouse.rs), the original's own module.
