PrintCraft
==========

From the artcraft team

A clean-room, open-source PDF application written in Rust, aiming at Adobe Acrobat Pro parity. It runs natively on macOS, Windows and Linux, and in the browser via WebAssembly.

<p align="center">
  <img src="docs/images/printcraft-viewer.png" alt="PrintCraft showing the typography showcase PDF, with the All tools panel on the left and threaded comments on the right" width="100%">
</p>

- **Engine first.** Parsing, rendering and editing live in library crates. The egui interface is one swappable crate on top.
- **Faithful to the file.** The PDF object graph is the document model. Unknown data is preserved. Saves are incremental: the original bytes are kept and your edits are appended.
- **Tested against the real world.** The pdf.js test corpus (983 files) is opened, rendered and round-tripped on every change, and qpdf and poppler are used as independent checks.

<p align="center">
  <img src="docs/images/printcraft-organize.png" alt="The Organize pages view: a grid of page thumbnails with rotate, delete, insert and move actions" width="100%">
</p>

## Status

**Early.** Work so far:
- **Viewer:** rendering, text find, select and copy, bookmarks, comments, layers, attachments and form-field highlighting, on desktop and the web.
- **Editing:**
  - Page organizing: rotate, delete, insert and reorder pages.
  - Document properties.
  - Undo and redo.
  - Save and Save As.
  - A prompt to save unsaved changes before closing.

See [ROADMAP.md](ROADMAP.md) for the milestones, progress and estimates to full parity.

```sh
cargo run -p printcraft -- some.pdf        # desktop app
cargo xtask demo-pdf                       # build dist/demo/printcraft-showcase.pdf (needs Chrome)
cargo run -p printcraft -- dist/demo/printcraft-showcase.pdf --panel comments
cargo run -p printcraft-cli -- edit in.pdf --rotate 1:90 --delete 3 --title "Report" --out out.pdf
```

The rules for contributors and agents are in [CLAUDE.md](CLAUDE.md).

## The Craft family

Open-source, clean-room, pure-Rust creative applications. Each is native on macOS, Windows and Linux, runs on the web, and can be driven by agents.

<table>
  <tr>
    <td width="20%" align="center" valign="top">
      <a href="https://github.com/storytold/photocraft"><b>PhotoCraft</b></a><br>
      <sub>Image editing</sub>
    </td>
    <td width="20%" align="center" valign="top">
      <a href="https://github.com/storytold/drawcraft"><b>DrawCraft</b></a><br>
      <sub>Vector illustration</sub>
    </td>
    <td width="20%" align="center" valign="top">
      <a href="https://github.com/storytold/filmcraft"><b>FilmCraft</b></a><br>
      <sub>Video editing</sub>
    </td>
    <td width="20%" align="center" valign="top">
      <a href="https://github.com/storytold/lightcraft"><b>LightCraft</b></a><br>
      <sub>Photo library &amp; raw</sub>
    </td>
    <td width="20%" align="center" valign="top">
      <a href="https://github.com/storytold/printcraft"><b>PrintCraft</b></a><br>
      <sub>PDF documents</sub>
    </td>
  </tr>
  <tr>
    <td valign="top"><sub>Layered raster editor with byte-exact PSD/PSB round trips and 13 formats at 8/16/32-bit. A Photoshop-class workflow.</sub></td>
    <td valign="top"><sub>Vector illustration with the Illustrator-style tools, panels, menus and shortcuts people already know.</sub></td>
    <td valign="top"><sub>Non-linear video editor with frame-exact timing and its own codecs. Aims at Premiere Pro parity.</sub></td>
    <td valign="top"><sub>Photo library and non-destructive raw developer: its own raw decoders, a wide-gamut float pipeline and a local-first catalog.</sub></td>
    <td valign="top"><sub>View, organize, annotate, fill, sign and edit PDFs. Aims at Acrobat Pro parity. <i>You are here.</i></sub></td>
  </tr>
</table>

## Licence

MIT OR Apache-2.0 (proposed). Third-party material is listed in [NOTICE](NOTICE).
