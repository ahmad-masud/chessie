# Chessie

Play chess against Stockfish at a rating you choose, on an oversized board in a
bare scene under an open sky, viewed from a camera you steer.

Built in Rust with [Bevy](https://bevyengine.org) 0.19 for the 3D world and
[shakmaty](https://docs.rs/shakmaty) for the rules.

## A packaged app

```sh
./scripts/bundle-macos.sh
open dist/Chessie.app
```

That produces a single `Chessie.app` (about 160 MB) with the engine and the
piece model inside it. Nothing needs installing, and it does not care where it
is moved to — it looks for its files in `Contents/Resources` first, then beside
the executable, then in the project directory, so the same binary works from a
bundle, a folder, or `cargo run`.

Two caveats for handing it to anyone else:

- The script signs the app **ad-hoc**, which is enough to run locally but not
  to pass Gatekeeper on another machine. Someone else opening it will need to
  right-click and choose Open the first time. Distributing it properly needs
  an Apple Developer ID and notarisation.
- It bundles Stockfish, which is GPL-3. That carries obligations — see
  `CREDITS.md` before publishing it anywhere.

## Requirements

- Rust (stable)
- Stockfish, for running from source: `brew install stockfish`. The packaged
  app carries its own copy and needs nothing installed.

## Run

```sh
cargo run --release
```

The first build compiles Bevy from scratch and takes a few minutes.

Run it through `cargo`, not by invoking the binary directly: Bevy looks for
`assets/` next to the executable otherwise. To run the built binary from
elsewhere, either copy `assets/` alongside it or set `BEVY_ASSET_ROOT` to the
project directory.

## Controls

| Key | Action |
| --- | --- |
| Left click | Select a piece, then its destination |
| Left drag | Or just drag a piece where you want it |
| Right-drag / `WASD` | Look around the board |
| Scroll / `[` `]` | Zoom in and out |
| `T` | Type the opponent's rating, then `Enter` (`Esc` cancels) |
| `-` / `=` | Nudge the rating; hold to repeat, faster the longer you hold |
| `←` `→` | Step back and forth through the game |
| `↑` / `Home` | Jump to the start |
| `↓` / `End` | Return to the live position |
| `F` | Swap which colour you play (starts a new game) |
| `Q` `R` `B` `N` | Choose the piece when a pawn promotes |
| `H` | Show or hide the controls |
| `R` | Restart the game |
| `F3` | Show frame rate, entity and mesh counts |

## Setting the rating

Press `T` and type the number — the range is 250 to 3190, and anything outside
it is clamped rather than refused. `-` and `=` still nudge, and holding either
repeats with a step that grows the longer you hold, so crossing the range takes
a moment rather than a hundred presses.

## How the rating works

Stockfish's own `UCI_Elo` only reaches down to **1320**, which is still a solid
club player. Above that floor the slider maps straight onto `UCI_LimitStrength`
and `UCI_Elo`.

Below 1320 the engine stays pinned at its floor and `src/strength.rs` mixes in
mistakes: with a probability that ramps to ~60% at the bottom of the range, the
engine's choice is swapped for a weaker legal move, biased toward the kind of
error a beginner actually makes (declining a capture the engine wanted) rather
than uniform random noise. Thinking time also scales with rating.

## Performance

Measured on an M5 in a release build, with the window focused.

Chessie draws only when something is happening, the way a conventional chess
app does. There is no character to walk and nothing animates on its own — the
scene is deliberately static — nothing animates on its own, so there is never
anything new to draw. Left alone it goes to sleep, and any
input wakes it instantly at full frame rate.

| | CPU | GPU |
| --- | --- | --- |
| Originally, drawing continuously | 2.02 cores | 89% |
| Idle now, untouched | **0.021 cores** | 17% |

(The machine's floor with nothing running is 9% GPU.) Idle settles to about four
frames a second, which is just a safety heartbeat; interacting returns it to the
display's full rate.

Getting there took three things:

- **There are no point lights.** The scene once had four shadow-casting ones,
  and each renders the scene six times, once per cubemap face — about 11,000
  extra draw calls a frame. Shrinking the shadow maps or their range changed
  nothing measurable: the cost is the passes, not the pixels. Only the sun
  casts shadows now.
- **Nothing animates at rest**, so there is never anything new to draw.
- **The frame rate is paced at runtime**, full speed while input, an animation
  or an engine search is in flight, asleep otherwise. The camera counts as busy
  until it has finished easing into place, so drawing never stops mid-swing.

Press `F3` in game for a live readout.

## Layout

| File | Role |
| --- | --- |
| `src/chess.rs` | Game state and rules, over `shakmaty` |
| `src/engine.rs` | Stockfish subprocess, UCI on a background thread |
| `src/strength.rs` | Rating model and sub-1320 weakening |
| `src/board.rs` | Board geometry, procedural piece meshes, square↔world mapping |
| `src/world.rs` | Ground, sky, lighting |
| `src/player.rs` | First-person controller and the board camera |
| `src/interaction.rs` | Click-to-move, highlights, engine turns |
| `src/ui.rs` | HUD and rating control |

The pieces are a model: Poly Haven's [Chess Set](https://polyhaven.com/a/chess_set)
by Riley Queen, CC0, loaded from `assets/models/chess_set/`. See `CREDITS.md`.
Only the pieces come from it — the board does real work (click targets, move
highlights, the border the captured pieces stand on), so it stays generated.

Loading is asynchronous and the generated pieces are drawn until it finishes,
so the game is playable immediately and still works with `assets/` deleted.
The set is scaled from its own king's height rather than by a magic number, so
it keeps its proportions whatever resolution you swap in. The target height is 1.45 times the square. A real
tournament set is 1.67 — a 95 mm king on a 57 mm square — but the camera here
sits closer than a player does, and at that ratio the pieces crowded the board.
The spacing of the captured piles is derived from the piece size rather than
set by hand, so changing how big the set is cannot quietly make them overlap. The generated stand-ins are measured at startup and scaled
to the same height, so nothing changes size when the model finishes loading.

Those fallback pieces are built the way real ones are turned: `src/pieces.rs` revolves a 2D
profile around the Y axis to get proper curved silhouettes, with the knight
extruded from a side-on outline since it has no axis of revolution.

The knight's outline is concave — the ears, the jaw, the muzzle — so its faces
are triangulated by ear clipping rather than fanned from a centroid, which
would lay triangles across those notches. `extrude` also normalises the
winding, because an outline given clockwise builds its faces inside-out and
backface culling then shows straight through into the piece.

## Choosing a side

You play White by default. `F` swaps you to the other colour and starts a fresh
game — changing sides mid-game would leave the engine playing against its own
position. The camera swings round to your end of the board, and as Black the
engine opens.

The swing orbits around the board rather than crossing it. The camera's angles
and distance are what ease toward their targets, not its position: interpolating
the position would draw a straight line between the two ends, and for a
half-turn that line passes through the middle of the board.

## The evaluation bar

A bar down the left edge shows who is better placed: White fills from the
bottom, Black from the top, with the score in pawns written on it. The status
panel repeats it as a percentage.

It is fed by a **second Stockfish running at full strength**, which never plays
a move. The opponent engine is deliberately weakened, so its opinion of a
position is worth as little as its play; asking it would make the bar wrong in
exactly the games where you most want to trust it.

Two details that are easy to get wrong. UCI reports scores from the side to
move's point of view, so the same winning position reads `+934` for White to
move and `-900` for Black to move — the score is flipped into White's terms or
the bar would swap ends every move. And centipawns are converted to a winning
chance through the usual logistic curve rather than shown linearly, because a
pawn up is a nudge while a queen up is all but decided.

The bar follows the position you are looking at, so stepping back through the
game shows how things stood then.

## Captured pieces and material

The status panel shows who is ahead and by how much, counting a pawn as 1, a
knight or bishop as 3, a rook as 5 and a queen as 9, along with what each side
has lost. Taken pieces are also stood in a pile on the board's own border, on their
owner's left as that player sees it, cheapest first, at the same size as the
pieces still in play. The border is sized from where those piles fall rather
than being a fixed width, so it always reaches past the outermost piece. They
rest a tile's thickness below the playing surface, because the border is the
plinth's top face and the squares sit proud of it.

Both are read from the game rather than from a running tally, so stepping back
through the history shows the material as it stood at that move. The balance
comes from the board and the piles from the move list: comparing against the
starting line-up would misreport promotions, since a side can end up with more
queens than it began with.

## Reviewing the game

If you miss the opponent's reply, step back through the game with the arrow
keys. The board shows the earlier position while the real game carries on
underneath: the engine is unaffected, clicks are ignored, and highlights are
hidden so it is clear you are looking at history. Stepping onto the newest move,
or pressing `↓`, returns you to live play. Single steps animate, so you can see
what actually moved.

Playing a move while looking at an earlier position **starts a new line from
there**: the moves that came after are discarded and the game continues down
the branch you just made, the way taking a move back over a real board does.
Stepping back and forward changes nothing until you actually move.

While you are reading back, the engine is paused, so the game cannot move on
underneath you. If it was already thinking when you branched, its answer is
for a position that no longer exists and is thrown away rather than played.

## Lighting

The scene is lit as a clear day, with values set against Bevy's reference
scale, where `OVERCAST_DAY` is 1 000 lux and `FULL_DAYLIGHT` is 20 000: a high
sun at 11 000, and a blue sky fill under it doing what the sky really does on
a clear day. The sun is placed high so the pieces cast short shadows rather
than long bars across the squares.

The sky is a gradient dome drawn unlit from the inside rather than a flat clear
colour, which otherwise reads as a void rather than a sky. The distance fog is
tinted to the horizon colour so the ground fades into it.

One thing worth knowing if you retune it: the board's square colours are
further apart than looks right in isolation, because light pulls them
together.

## The interface

Deliberately small: one low-contrast panel in the corner with three lines —
who you are playing, what is happening, and the material picture — plus the
evaluation bar down the edge. Everything else is behind `H`, which opens on the
right of the screen, clear of both the status panel and the board. The
performance readout sits in the opposite corner so the two never overlap.

## A note on clicking

Bevy's UI picking backend defaults to treating every UI node as pickable, and a
pickable node blocks picking of whatever is behind it. That made the pieces
underneath the HUD panels and the evaluation bar unclickable. None of the
interface here is meant to be clicked, so UI picking is switched off wholesale
by requiring an explicit marker that nothing sets.

The same default applies to meshes, which is why the highlight tiles and the
captured pieces carry `Pickable::IGNORE`: they sit over squares and would
otherwise intercept the click meant for the board.

## Notes on the board

Pieces slide between squares rather than snapping, and knights hop. A piece you
drag finishes from wherever you released it rather than snapping back to its old
square first, and the travel time scales with the distance, so releasing one a
hair off its square is a settle rather than a slow glide. Castling
animates the king and the rook together, and an en-passant capture removes the
pawn that is actually taken rather than the one on the destination square.

To castle, move the king two squares toward the rook, as in any other chess
interface. Internally `shakmaty` stores castling as king-takes-rook and reports
the rook's square as the destination, which is why the highlight used to appear
on the rook; the board now shows and accepts the king's square instead. Clicking
the rook still works if you prefer it. Either way the engine receives standard
UCI (`e1g1`), which it always did.

While playing you can lean around the board with a right-drag, the arrow keys,
or the scroll wheel. The orbit is deliberately clamped to White's half so the
view never swings behind Black.
