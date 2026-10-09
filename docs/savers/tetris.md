# `tetris`

![`tetris`](../media/tetris.gif)

A falling-block game played by an AI, as many wells side by side as the
panel's shape holds. Each well is its own game: SRS rotation with wall kicks,
a 7-bag randomizer, a ghost showing where the piece will land, rows that wipe
out from the middle (the frame flashes for four at once), NES gravity and
scoring with a new level every ten lines, and the frame changing colour with
the level. Score sits above each well; the next piece, level and lines beside
it.

A well is 10x20 and its blocks are square on the glass, so its height sets the
block size and its width follows. Widening the well would change the game and
stretching the blocks would make them bricks, so the panel gets more games
instead: every arrangement is scored by the share of the panel it covers,
leaning towards bigger blocks. Pine's 3.2:1 glass gets four wells, 16:9 and
4:3 two, square and portrait one.

The AI scores every placement reachable by rotating at the spawn, sliding and
dropping, with El-Tetris's weights (landing height, rows cleared, row and
column transitions, holes, cumulative wells), and looks one piece ahead. It
then moves the piece like a player, at `TETRIS_MOVES` moves a second, while
gravity pulls. It never loses to the stack: a game ends at the kill screen,
level 29, where pieces fall two rows a frame and it cannot reach the edges in
time. In tests it averages about 290 lines a game, roughly fifteen minutes
each from level 0. Then the curtain comes down and a new game starts.

Source: [`src/tetris.rs`](../../src/tetris.rs) and
[`src/tetris/game.rs`](../../src/tetris/game.rs).

## Knobs

- `TETRIS_CELL` (px, 4..64, default 8)
- `TETRIS_WELLS` (wells side by side, 0..8, default 0 = as many as fit)
- `TETRIS_LEVEL` (starting level, 0..19, default 0)
- `TETRIS_MOVES` (the AI's moves a second, 2..60, default 15)
- `TETRIS_LOOKAHEAD` (weigh the next piece too, 0..1, default 1)
- `TETRIS_SEED` (0 rolls a new one each build)
