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

## Fights

A capture is fought out, in the spirit of Interplay's Battle Chess (1988) with
art of our own. The board zooms in three times on the two squares, the pieces
come alive as 32x32 fighters drawn at twice the board sprites' resolution (the
same colours, outline and shading), the attacker walks over, they fight, the
victim is defeated, the attacker takes the square and the board zooms back out.
Three seconds by default.

Each attacker has its own fight, and three pairings have their own:

| Attacker  | Fight                                                                |
| --------- | -------------------------------------------------------------------- |
| Pawn      | Two spear jabs; the victim topples and fades                         |
| Knight    | Backs off and charges with a lance; the victim goes flying, spinning |
| Bishop    | A fireball from the staff; the victim burns to ash                   |
| Rook      | Leaps and lands on the victim, flattening it; a shock ring           |
| Queen     | Lightning from the wand; the victim dissolves into sparks            |
| King      | Sceptre blows; the victim shatters                                   |
| Q takes Q | A duel: two bolts meet and the attacker's pushes through             |
| N takes R | The tower crumbles to a pile of bricks                               |
| P takes Q | A poke, a startled hop, a faint under circling stars                 |

En passant is fought beside the pawn being taken, then the attacker steps to
its square; a promotion turns into its new piece in a burst of sparks; a capture
that gives check or mate ends with the word over the victor.

The move is played the moment the fight starts, so the next search runs during
it; only showing the answer waits for the fight to finish. With two boards each
fights on its own.

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
- `CHESS_FIGHTS` (captures fought out, 0..1, default 1)
- `CHESS_FIGHT_SECS` (seconds a fight lasts, 1..10, default 3)
- `CHESS_SEED` (0 = random each build)
