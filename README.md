# Another Epub Reader

A simple desktop EPUB reader for Windows. Open your books, follow the table of contents, adjust the
font size and theme, and continue reading where you left off.

## Features

- Opens DRM-free EPUB 2 and EPUB 3 books.
- Table of contents sidebar, resizable by dragging the divider.
- Clickable links inside chapters, including links to other chapters.
- Remembers recent books and the reading position of each one.
- Three themes: White, Sepia and Night.
- Font size from 14 to 30 px.
- Keyboard shortcuts: `←`/`→` previous/next chapter, `Ctrl+O` open a book, `Esc` close popups.
- Drag and drop an EPUB onto the window to open it.
- About dialog showing the app version, build toolchain and direct dependencies.

## Requirements

- Windows 10 or 11.
- Rust 1.97+ with the MSVC build tools (Visual Studio 2022/2026, Windows SDK).

## Build and run

```powershell
cargo run
cargo build --release
cargo test
```

The first build takes a few minutes. The release executable is
`target\release\another-epub-reader.exe`.

Prebuilt installers (`.msi`, setup `.exe`) and a portable `.exe` are attached to the Releases page.

## Usage

Open a book with the button, `Ctrl+O`, or by dragging an `.epub` file onto the window. Books you have
opened appear under **Recent books**; if a book file was moved or deleted, its row shows
"File not found" and can be removed with the × button. Reading position, theme, font size and other
settings are saved automatically.

Application data is stored in `%APPDATA%\epub-reader\state.json` with cover thumbnails under
`covers/`. If that folder is not writable, the app falls back to `epub-reader-data\` next to the
executable, then `%LOCALAPPDATA%\epub-reader`, then the temporary directory. The
`EPUB_READER_DATA_DIR` environment variable overrides the location.

## Known limitations

- Windows only.
- Some long Vietnamese words can be split across lines in narrow columns.
- HTML/CSS support covers common book styling; complex layouts such as floats, columns or fixed
  positioning are not rendered.

## License

Released under the [MIT License](LICENSE).
