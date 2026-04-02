use anyhow::{bail, Context, Result};
use eframe::egui::{Color32, ColorImage};
use png::{BitDepth, ColorType, Decoder, Encoder};
use sha2::{Digest, Sha256};
use std::{fs::File, io::BufReader, path::Path};
use tiff::{
    decoder::{Decoder as TiffDecoder, DecodingResult},
    tags::Tag,
    ColorType as TiffColorType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceFormat {
    Png,
    Tiff,
}

impl SourceFormat {
    pub fn allows_transform_export(self) -> bool {
        matches!(self, Self::Png | Self::Tiff)
    }
}

#[derive(Clone, Debug)]
pub enum RawPixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageTransform {
    pub rotation_quadrants: u8,
    pub mirror_horizontal: bool,
    pub mirror_vertical: bool,
}

impl ImageTransform {
    pub fn rotate_left(&mut self) {
        self.rotation_quadrants = (self.rotation_quadrants + 3) % 4;
    }

    pub fn rotate_right(&mut self) {
        self.rotation_quadrants = (self.rotation_quadrants + 1) % 4;
    }

    pub fn toggle_mirror_horizontal(&mut self) {
        self.mirror_horizontal = !self.mirror_horizontal;
    }

    pub fn toggle_mirror_vertical(&mut self) {
        self.mirror_vertical = !self.mirror_vertical;
    }

    pub fn transformed_dimensions(self, width: usize, height: usize) -> (usize, usize) {
        if self.rotation_quadrants % 2 == 0 {
            (width, height)
        } else {
            (height, width)
        }
    }

    pub fn apply_to_point(self, point: (f32, f32), width: f32, height: f32) -> (f32, f32) {
        let (mut x, mut y) = match self.rotation_quadrants % 4 {
            0 => point,
            1 => (height - point.1, point.0),
            2 => (width - point.0, height - point.1),
            _ => (point.1, width - point.0),
        };

        let (display_width, display_height) =
            self.transformed_dimensions(width as usize, height as usize);
        if self.mirror_horizontal {
            x = display_width as f32 - x;
        }
        if self.mirror_vertical {
            y = display_height as f32 - y;
        }

        (x, y)
    }

    pub fn invert_point(self, point: (f32, f32), width: f32, height: f32) -> (f32, f32) {
        let (display_width, display_height) =
            self.transformed_dimensions(width as usize, height as usize);
        let mut x = point.0;
        let mut y = point.1;

        if self.mirror_horizontal {
            x = display_width as f32 - x;
        }
        if self.mirror_vertical {
            y = display_height as f32 - y;
        }

        match self.rotation_quadrants % 4 {
            0 => (x, y),
            1 => (y, height - x),
            2 => (width - x, height - y),
            _ => (width - y, x),
        }
    }
}

pub struct LoadedImage {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub bit_depth: u8,
    pub hash: String,
    pub display: ColorImage,
    pub raw_pixels: RawPixels,
    pub source_format: SourceFormat,
}

impl LoadedImage {
    pub fn transformed_display(&self, transform: ImageTransform) -> ColorImage {
        transform_color_image(&self.display, transform)
    }

    pub fn transformed_adjusted_display(
        &self,
        transform: ImageTransform,
        brightness: f32,
        contrast: f32,
    ) -> ColorImage {
        let transformed = self.transformed_display(transform);
        let pixels = transformed
            .pixels
            .iter()
            .map(|pixel| {
                let gray = pixel.r() as f32 / 255.0;
                let adjusted = ((gray - 0.5) * contrast + 0.5 + brightness).clamp(0.0, 1.0);
                Color32::from_gray((adjusted * 255.0).round() as u8)
            })
            .collect();

        ColorImage {
            size: transformed.size,
            pixels,
        }
    }

    pub fn transformed_dimensions(&self, transform: ImageTransform) -> (usize, usize) {
        transform.transformed_dimensions(self.width, self.height)
    }

    pub fn export_transformed_png(
        &self,
        transform: ImageTransform,
        output_path: &Path,
    ) -> Result<()> {
        let file = File::create(output_path)
            .with_context(|| format!("failed to create {}", output_path.display()))?;

        match &self.raw_pixels {
            RawPixels::U8(values) => {
                let transformed = transform_u8_pixels(values, self.width, self.height, transform);
                let (width, height) = transform.transformed_dimensions(self.width, self.height);
                let mut encoder = Encoder::new(file, width as u32, height as u32);
                encoder.set_color(ColorType::Grayscale);
                encoder.set_depth(BitDepth::Eight);
                let mut writer = encoder
                    .write_header()
                    .context("failed to write PNG header")?;
                writer
                    .write_image_data(&transformed)
                    .context("failed to write transformed PNG data")?;
            }
            RawPixels::U16(values) => {
                let transformed = transform_u16_pixels(values, self.width, self.height, transform);
                let (width, height) = transform.transformed_dimensions(self.width, self.height);
                let mut encoder = Encoder::new(file, width as u32, height as u32);
                encoder.set_color(ColorType::Grayscale);
                encoder.set_depth(BitDepth::Sixteen);
                let mut writer = encoder
                    .write_header()
                    .context("failed to write PNG header")?;
                let mut raw = Vec::with_capacity(transformed.len() * 2);
                for value in transformed {
                    raw.extend_from_slice(&value.to_be_bytes());
                }
                writer
                    .write_image_data(&raw)
                    .context("failed to write transformed PNG data")?;
            }
        }

        Ok(())
    }
}

pub fn load_image(path: &Path) -> Result<LoadedImage> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase());

    match extension.as_deref() {
        Some("png") => load_png(path),
        Some("tif") | Some("tiff") => load_tiff(path),
        Some("dcm") | Some("dicom") | Some("diconde") => {
            bail!("DICOM/DICONDE loading is not implemented")
        }
        _ => bail!("only PNG and TIFF files are supported"),
    }
}

fn load_png(path: &Path) -> Result<LoadedImage> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let decoder = Decoder::new(file);
    let mut reader = decoder.read_info().context("failed to read PNG header")?;
    let info = reader.info();
    if info.color_type != ColorType::Grayscale {
        bail!("only grayscale PNG files are supported");
    }
    let bit_depth = match info.bit_depth {
        BitDepth::Eight => 8,
        BitDepth::Sixteen => 16,
        _ => bail!("only uint8 and uint16 grayscale PNG files are supported"),
    };

    let mut buffer = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut buffer)
        .context("failed to decode PNG frame")?;
    if frame.color_type != ColorType::Grayscale {
        bail!("expected grayscale data in PNG frame");
    }

    let raw = &buffer[..frame.buffer_size()];
    let (hash, display, raw_pixels) = match bit_depth {
        8 => (
            hash_raw_pixels(raw),
            make_display_image_u8(frame.width as usize, frame.height as usize, raw)?,
            RawPixels::U8(raw.to_vec()),
        ),
        16 => {
            let values = decode_png_u16(raw)?;
            let native = u16s_to_native_bytes(&values);
            (
                hash_raw_pixels(&native),
                make_display_image_u16(frame.width as usize, frame.height as usize, &values)?,
                RawPixels::U16(values),
            )
        }
        _ => bail!("unsupported bit depth"),
    };

    Ok(LoadedImage {
        path: path.display().to_string(),
        width: frame.width as usize,
        height: frame.height as usize,
        bit_depth,
        hash,
        display,
        raw_pixels,
        source_format: SourceFormat::Png,
    })
}

fn load_tiff(path: &Path) -> Result<LoadedImage> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut decoder =
        TiffDecoder::new(BufReader::new(file)).context("failed to read TIFF header")?;
    let (width, height) = decoder
        .dimensions()
        .context("failed to read TIFF dimensions")?;
    let color_type = decoder
        .colortype()
        .context("failed to read TIFF color type")?;
    let photometric = decoder
        .find_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
        .ok()
        .flatten()
        .unwrap_or(1);

    let (bit_depth, hash, display, raw_pixels) = match decoder
        .read_image()
        .context("failed to decode TIFF image")?
    {
        DecodingResult::U8(mut values) => {
            match color_type {
                TiffColorType::Gray(8) => {}
                _ => bail!("only grayscale 8-bit or 16-bit TIFF files are supported"),
            }
            if photometric == 0 {
                for value in &mut values {
                    *value = u8::MAX - *value;
                }
            }
            let hash = hash_raw_pixels(&values);
            let display = make_display_image_u8(width as usize, height as usize, &values)?;
            (8, hash, display, RawPixels::U8(values))
        }
        DecodingResult::U16(mut values) => {
            match color_type {
                TiffColorType::Gray(16) => {}
                _ => bail!("only grayscale 8-bit or 16-bit TIFF files are supported"),
            }
            if photometric == 0 {
                for value in &mut values {
                    *value = u16::MAX - *value;
                }
            }
            let native = u16s_to_native_bytes(&values);
            let hash = hash_raw_pixels(&native);
            let display = make_display_image_u16(width as usize, height as usize, &values)?;
            (16, hash, display, RawPixels::U16(values))
        }
        _ => bail!("only grayscale 8-bit or 16-bit TIFF files are supported"),
    };

    Ok(LoadedImage {
        path: path.display().to_string(),
        width: width as usize,
        height: height as usize,
        bit_depth,
        hash,
        display,
        raw_pixels,
        source_format: SourceFormat::Tiff,
    })
}

fn hash_raw_pixels(raw: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw);
    format!("{:x}", hasher.finalize())
}

fn decode_png_u16(raw: &[u8]) -> Result<Vec<u16>> {
    if raw.len() % 2 != 0 {
        bail!("decoded 16-bit PNG byte count was invalid");
    }
    Ok(raw
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn u16s_to_native_bytes(values: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 2);
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn make_display_image_u8(width: usize, height: usize, raw: &[u8]) -> Result<ColorImage> {
    let pixels = raw
        .iter()
        .map(|value| Color32::from_gray(*value))
        .collect::<Vec<_>>();

    if pixels.len() != width * height {
        bail!("decoded grayscale size did not match image dimensions");
    }

    Ok(ColorImage {
        size: [width, height],
        pixels,
    })
}

fn make_display_image_u16(width: usize, height: usize, values: &[u16]) -> Result<ColorImage> {
    let pixels = values
        .iter()
        .map(|value| Color32::from_gray((value >> 8) as u8))
        .collect::<Vec<_>>();

    if pixels.len() != width * height {
        bail!("decoded grayscale size did not match image dimensions");
    }

    Ok(ColorImage {
        size: [width, height],
        pixels,
    })
}

fn transform_color_image(image: &ColorImage, transform: ImageTransform) -> ColorImage {
    let width = image.size[0];
    let height = image.size[1];
    let (out_width, out_height) = transform.transformed_dimensions(width, height);
    let mut pixels = vec![Color32::BLACK; out_width * out_height];

    for y in 0..out_height {
        for x in 0..out_width {
            let (source_x, source_y) = inverse_transform_index(x, y, width, height, transform);
            pixels[y * out_width + x] = image.pixels[source_y * width + source_x];
        }
    }

    ColorImage {
        size: [out_width, out_height],
        pixels,
    }
}

fn transform_u8_pixels(
    pixels: &[u8],
    width: usize,
    height: usize,
    transform: ImageTransform,
) -> Vec<u8> {
    let (out_width, out_height) = transform.transformed_dimensions(width, height);
    let mut transformed = vec![0; out_width * out_height];

    for y in 0..out_height {
        for x in 0..out_width {
            let (source_x, source_y) = inverse_transform_index(x, y, width, height, transform);
            transformed[y * out_width + x] = pixels[source_y * width + source_x];
        }
    }

    transformed
}

fn transform_u16_pixels(
    pixels: &[u16],
    width: usize,
    height: usize,
    transform: ImageTransform,
) -> Vec<u16> {
    let (out_width, out_height) = transform.transformed_dimensions(width, height);
    let mut transformed = vec![0; out_width * out_height];

    for y in 0..out_height {
        for x in 0..out_width {
            let (source_x, source_y) = inverse_transform_index(x, y, width, height, transform);
            transformed[y * out_width + x] = pixels[source_y * width + source_x];
        }
    }

    transformed
}

fn inverse_transform_index(
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    transform: ImageTransform,
) -> (usize, usize) {
    let (display_width, display_height) = transform.transformed_dimensions(width, height);
    let mut x = x;
    let mut y = y;

    if transform.mirror_horizontal {
        x = display_width - 1 - x;
    }
    if transform.mirror_vertical {
        y = display_height - 1 - y;
    }

    match transform.rotation_quadrants % 4 {
        0 => (x, y),
        1 => (y, height - 1 - x),
        2 => (width - 1 - x, height - 1 - y),
        _ => (width - 1 - y, x),
    }
}
