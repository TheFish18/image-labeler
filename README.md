# Image Labeler

Small native Rust app for labeling grayscale PNG images.

## Features

- Loads grayscale PNG files with `uint8` or `uint16` pixels.
- Computes a BLAKE3 hash from the decoded raw pixel bytes.
- Stores labels in SQLite keyed by that hash, so renaming a file does not break label lookup.
- Supports per-class rectangle and polygon annotations.
- Includes an in-app file browser for selecting PNG files.
- Supports non-persistent pan, zoom, brightness, and contrast controls.

## Run

```bash
~/.cargo/bin/cargo run
```

The app creates `labels.sqlite3` in the project directory.

## Use

1. Start the app.
2. Use the browser panel to navigate to and select a PNG.
3. Select or create a class.
4. Choose `Rectangle` or `Polygon`.
5. Draw on the image.

For polygons, click to place points and use `Finish polygon` or `Enter` to save.

## View Controls

- Mouse wheel zooms.
- Right or middle drag pans.
- `Brightness` and `Contrast` only affect display.
- `Reset view` restores zoom, pan, brightness, and contrast without touching saved labels.
