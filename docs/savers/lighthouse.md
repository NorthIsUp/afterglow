# `lighthouse`

![`lighthouse`](../media/lighthouse.gif)

A banded lighthouse on a heap of rocks at night. Its beam turns round the lantern, long when it crosses the frame and a flash when it faces us, lighting the haze and the waves beneath; surf bursts on the rocks.

One of ascii.rest's character pieces, drawn at whatever size and shape the panel is. See [the ascii.rest ports](../ascii-rest.md#character-pieces).

## At any size

The tower and its rocks are scaled to the panel's height and centred, and the sea, the horizon and the sky run out to every edge. On a wide panel the beam reaches far enough to sweep across the whole width and light the waves under it. On a panel too narrow for the rocks at that scale (square, portrait) the lighthouse scales to the width instead, and the rows left over become sky above and sea below, a little more sky.

The beam makes a full turn. On the half that faces us it swings across in front of the lantern, a little brighter, and when it points straight at us it blooms into a round flare over the tower's top, with a star of rays on the lamp. On the far half it goes behind the tower, narrower and dimmer, hidden by the lantern and the roof, and its far end draws smoothly in to the tower and back out rather than jumping across. Upstream's beam always passes behind.

In colour: the beam is amber, fading to bronze in the thin haze, and the lamp is pale gold. The tower is white, banded red, with an iron gallery and roof, on grey-brown rocks. The sea is two blues, with the beam's road on it in amber, and the surf is white. The stars are blue-white and the sky deep navy.

## Knobs

- `LIGHTHOUSE_BEAM_FRONT` (0..1, default 1): 0 sends the beam behind the tower all the way round, as upstream does.
- `LIGHTHOUSE_COLOR` (0..1, default 1): 0 draws it in upstream's one amber ink on black.
- `ASCII_REST_TEXT_CELL_W` / `ASCII_REST_TEXT_CELL_H` and `ASCII_REST_TITLE`; see [character pieces](../ascii-rest.md#character-pieces).

With both of its own knobs at 0, on upstream's 64x30 grid, it is upstream's picture cell for cell.

Source: [`src/ascii_rest/lighthouse.rs`](../../src/ascii_rest/lighthouse.rs).
