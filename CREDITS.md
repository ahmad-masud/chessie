# Credits

## Chess pieces

The 3D chess pieces are from **[Chess Set](https://polyhaven.com/a/chess_set)**
by **Riley Queen**, published by [Poly Haven](https://polyhaven.com).

Licensed **CC0 1.0 Universal** (public domain dedication). Poly Haven's
[licence page](https://polyhaven.com/license) states that all assets on the
site are CC0, "which is effectively Public Domain even in jurisdictions that do
not support the Public Domain". No attribution is required; this file records
the provenance anyway.

Only the pieces are used. The board, its border and everything else in the
scene are generated in code — see `src/pieces.rs` for the turned shapes that
stand in until the model loads, and `src/world.rs` for the ground and sky.

Files live in `assets/models/chess_set/`, at 1K texture resolution (7.4 MB).
Other resolutions are available from Poly Haven if you want them.

## Engine

Play is provided by [Stockfish](https://stockfishchess.org), GPL-3.0, run as a
separate process over UCI. It is not bundled or linked — install it separately
(`brew install stockfish`).
