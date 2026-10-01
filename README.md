# Image Labeler

Small native Rust app for labeling grayscale PNG and TIFF images.

## Features

- Loads grayscale PNG, TIFF, and DICOM/DICONDE files with `uint8` or `uint16` pixels.
- Decodes images at native bit depth and hashes the decoded grayscale pixel bytes, not the original file bytes.
- Flips grayscale TIFF values when the file uses the `WhiteIsZero` photometric interpretation.
- Flips grayscale DICOM/DICONDE values when the file uses `MONOCHROME1`.
- Computes a SHA-256 hash from the decoded grayscale raw pixel bytes at their native bit depth.
- Stores annotations in SQLite keyed by that hash, so renaming a file does not break label lookup.
- Loads editable schemas from `$XDG_CONFIG_HOME/image-labeler` or `$HOME/.config/image-labeler`.
- Supports schema labels of type `rectangle`, `polygon`, and `global`.
- Stores app-level keybinds in `$XDG_CONFIG_HOME/image-labeler/app-keybinds.toml` or `$HOME/.config/image-labeler/app-keybinds.toml`.
- Supports in-app rotate and mirror transforms and seamless PNG export of the transformed image.
- Includes an in-app file browser for selecting PNG or TIFF files.
- Supports non-persistent pan, zoom, brightness, and contrast controls.

## Run

```bash
~/.cargo/bin/cargo run
```

By default the app creates `labels.sqlite3` in the current working directory. Use `--db` to store labels in a different database (missing parent directories are created):

```bash
~/.cargo/bin/cargo run -- --db path/to/labels.sqlite3
image-labeler --db path/to/labels.sqlite3
```

Other startup options:

- `--schema my-schema` starts with an existing schema from the config directory. If there's no schema with that name, the available ones are listed.
- `--schema path/to/my-schema.toml` adds the schema in that file and starts with it. Its name is the normalized file name, here `my-schema`.
  - If no schema with that name is in the config directory, the file is validated and copied there.
  - If one exists with different contents, you're asked whether to overwrite it. Answering no, or launching without a terminal (e.g. from a shortcut), keeps the existing schema.
- `--input path/to/images` sets the directory the file browser starts in (default: the current directory).

```bash
image-labeler --db labels.sqlite3 --schema schemas/lesions.toml --input /data/scans
```

On first launch it also creates a default schema file at:

```bash
$XDG_CONFIG_HOME/image-labeler/default.toml
```

or, if `XDG_CONFIG_HOME` is not set:

```bash
$HOME/.config/image-labeler/default.toml
```

It also creates an app keybind config file at:

```bash
$XDG_CONFIG_HOME/image-labeler/app-keybinds.toml
```

or, if `XDG_CONFIG_HOME` is not set:

```bash
$HOME/.config/image-labeler/app-keybinds.toml
```

## Use

1. Start the app.
2. Use the browser panel to navigate to and select a PNG, TIFF, or DICOM/DICONDE image.
3. Pick, create, edit, or import a schema in the `Schema` section.
4. Select a rectangle or polygon label, or toggle a global label, from the `Labels` section.
5. Draw the shape type required by the selected non-global label.

For polygons, click to place points and use `Finish polygon` or `Enter` to save.

## Schema Format

Schemas now contain:

- `labels`: entries with `name`, `type`, `color_rgb`, and `keybind.chord`
- `type` may be `rectangle`, `polygon`, or `global`
- app-level action keybinds are stored separately in `app-keybinds.toml`

Example:

```toml
[[labels]]
name = "person-box"
type = "rectangle"
color_rgb = [255, 99, 71]

[labels.keybind]
chord = "p"

[[labels]]
name = "contains-person"
type = "global"
color_rgb = [255, 206, 84]

[labels.keybind]
chord = "shift+p"
```

App keybinds live in a separate file, for example:

```toml
[rotate_left]
chord = "shift+j"

[next_image]
chord = "shift+l"
```

## Decode And Hash Rules

- PNG: grayscale only, 8-bit or 16-bit only, hashed from decoded grayscale pixels at native bit depth.
- TIFF: grayscale only, 8-bit or 16-bit only, `WhiteIsZero` images are inverted before hashing.
- DICOM/DICONDE: grayscale single-sample only, 8-bit or 16-bit only, `MONOCHROME1` images are inverted before hashing.
- For 16-bit PNG, TIFF, and DICOM/DICONDE, the hash is computed from decoded `u16` pixel values in native-endian byte order.
- For 16-bit DICOM/DICONDE, the decoded values are masked to `BitsStored` before inversion, display conversion, and hashing.

## View Controls

- Mouse wheel zooms.
- Right or middle drag pans.
- Shift + right drag up/down adjusts brightness; Shift + right drag left/right adjusts contrast.
- `Brightness` and `Contrast` only affect display.
- `Hide segmentations` (default `shift+s`, app keybind `toggle_annotations`) toggles annotation overlays; hidden annotations cannot be selected or edited.
- `Reset view` restores zoom, pan, brightness, and contrast without touching saved labels.

## Editing

- Click an annotation to select it, then press `Delete` (app keybind `delete_annotation`) to remove it.
- `Ctrl+Z` undoes and `Ctrl+Shift+Z` redoes (app keybinds `undo` / `redo`). This covers creating, deleting, and editing annotations and toggling global labels.
- While drawing a polygon, `Enter` finishes it, `Escape` cancels it, and undo removes the last vertex.
- Undo history is kept in memory per image for the current session only (up to 200 steps); it is never written to the database.

## Transform Export

- `Rotate left`, `Rotate right`, `Mirror horizontal`, and `Mirror vertical` only affect the current in-app view and export result.
- `Save PNG` writes the transformed image to the editable output path without overwriting the original.
- The default output path is `<original_filename>_rotated.png` in the same directory.
- DICOM/DICONDE images can be viewed and transformed in-app, but persistent transformed export is intentionally disabled.
