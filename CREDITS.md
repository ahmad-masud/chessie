# Credits

Chessie's own code is MIT licensed, as in `LICENSE`. That covers the code in
`src/` and the packaging scripts. The two things it carries with it — the
piece model and, in a packaged build, the Stockfish binary — have their own
terms, set out below. The MIT licence on this repository does not extend to
them.

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

Play is provided by [Stockfish](https://stockfishchess.org), licensed
**GPL-3.0**. Chessie runs it as a separate process and talks to it over UCI;
it does not link against it.

`scripts/bundle-macos.sh` copies the Stockfish binary into the app so that
nothing has to be installed. **If you distribute that bundle, this brings
obligations**, and they are worth understanding before you publish it:

- Stockfish's licence and authors travel with the binary, in
  `Contents/Resources/stockfish-Copying.txt` and `stockfish-AUTHORS`.
- GPL-3 requires that anyone you give the binary to can get its **source**.
  Stockfish's source is at <https://github.com/official-stockfish/Stockfish>,
  and the version bundled is whatever `stockfish --version` reports for the
  binary you packaged. Point people at the matching tag, or include the
  source alongside the download.
- Running a GPL program as a separate process is normally treated as mere
  aggregation, so Chessie's own code is not itself forced to be GPL by
  shipping next to it. That is the common reading, not legal advice; if you
  intend to sell this, take advice.

If you would rather not carry those obligations, the alternative is a chess
engine under a permissive licence compiled into the binary. It would be
weaker than Stockfish, and the rating model in `src/strength.rs` assumes
Stockfish's `UCI_Elo`, so it is not a drop-in change.
