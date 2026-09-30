PrintCraft
==========

From the artcraft team

A clean-room, open-source PDF application written in Rust, aiming at Adobe Acrobat Pro parity. It runs natively on macOS, Windows and Linux, and in the browser via WebAssembly. It is a sibling project of PhotoCraft.

**Status:** early. The viewer slice works; the core PDF object layer (M1) is next. The plan lives in `plan/` (local) and the rules for contributors and agents are in `CLAUDE.md`.

```sh
cargo run -p printcraft -- some.pdf        # desktop app
cargo xtask demo-pdf                       # build dist/demo/printcraft-showcase.pdf (needs Chrome)
cargo run -p printcraft -- dist/demo/printcraft-showcase.pdf --panel bookmarks
```

Licence: MIT OR Apache-2.0 (proposed). Third-party material is listed in `NOTICE`.
