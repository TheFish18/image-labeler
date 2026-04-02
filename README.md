# Image Labeler

Small native Rust app for labeling grayscale PNG and TIFF images.

## Features

- Loads grayscale PNG and TIFF files with `uint8` or `uint16` pixels.
- Flips grayscale TIFF values when the file uses the `WhiteIsZero` photometric interpretation.
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

The app creates `labels.sqlite3` in the project directory.

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
2. Use the browser panel to navigate to and select a PNG or TIFF.
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

## View Controls

- Mouse wheel zooms.
- Right or middle drag pans.
- `Brightness` and `Contrast` only affect display.
- `Reset view` restores zoom, pan, brightness, and contrast without touching saved labels.

## Transform Export

- `Rotate left`, `Rotate right`, `Mirror horizontal`, and `Mirror vertical` only affect the current in-app view and export result.
- `Save PNG` writes the transformed image to the editable output path without overwriting the original.
- The default output path is `<original_filename>_rotated.png` in the same directory.
- DICOM/DICONDE transforms are intentionally disallowed. DICOM loading is not implemented.
