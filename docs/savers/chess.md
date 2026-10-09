# `chess`

![`chess`](../media/chess.gif)

Two engines playing each other, one game per board, as many boards as the
panel's shape holds. Each game opens with a named line from a small book of
32 openings, cut short at a random move, then the engines take over and play
to checkmate, stalemate, the fifty-move rule, threefold repetition or
insufficient material (200 moves is adjudicated a draw). The result sits over
the board for a few seconds and a new game starts on the next board colour.

Pieces glide from square to square, lifted slightly mid-flight. The last move's
squares are lit yellow and a king in check red. Beside each board are the eval
bar, the captured pieces with the material lead, the score sheet in SAN with
the opening's name and the engine's eval in figures, and each player's
strength as five pips. While an engine thinks, its depth so far and a clock bar
filling over its time budget show next to its name.

## Layout

The board is pixel art: every logical pixel is a solid cell, sized so a square
lands near 32 of them, and the 16x16 piece sprites are drawn at a whole
multiple. Text beside it is the repo's font at one cell per font pixel.

- **Pine's 3.2:1** (`SAVER_PIXEL_ASPECT=180`): two games side by side, each a
  full-height board with its score sheet to the right.
- **16:9 and 4:3**: one game, board on the left, side panel filling the rest.
- **Square and portrait**: the board on top, the side panel stacked under it;
  the score sheet flows into as many columns as fit.

## The engine

Written for this saver, in `src/chess/` (MIT like the rest): a mailbox move
generator checked against the standard perft positions, and an
iterative-deepening alpha-beta search with quiescence, a transposition table,
MVV-LVA and killer ordering, check extension, repetition detection, and
Michniewski's piece-square tables tapered into an endgame that pushes pawns and
drives a lone king to the edge. Each game's engine runs on its own thread; the
frame loop hands it a position and polls a mutex it only ever `try_lock`s, so
a search never costs a frame.

Strength is a level from 1 to 5: a depth cap, a share of `CHESS_THINK_MS`, and
noise in the evaluation (90 centipawns at level 1, none at 5). The noise is
seeded per game and per side, so the same position is misjudged differently in
the next game. By default two games in five pit equal engines (level 3 to 5)
against each other; the rest roll each side's level independently.

Source: [`src/chess/`](../../src/chess/).

## Knobs

- `CHESS_THINK_MS` (ms per move at full strength, 100..20000, default 1500)
- `CHESS_GAMES` (boards, 0..4, default 0 = fit the panel's shape)
- `CHESS_LEVEL` (both engines' strength, 0..5, default 0 = varied per game)
- `CHESS_RESULT_SECS` (seconds the result stays up, 1..60, default 6)
- `CHESS_SEED` (0 = random each build)
