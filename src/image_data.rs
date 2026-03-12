use anyhow::{anyhow, bail, Context, Result};
use eframe::egui::{Color32, ColorImage};
use png::{BitDepth, ColorType, Decoder};
use std::{fs::File, path::Path};

pub struct LoadedImage {
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub bit_depth: u8,
    pub hash: String,
    pub display: ColorImage,
}

impl LoadedImage {
    pub fn adjusted_display(&self, brightness: f32, contrast: f32) -> ColorImage {
        let pixels = self
            .display
            .pixels
            .iter()
            .map(|pixel| {
                let gray = pixel.r() as f32 / 255.0;
                let adjusted = ((gray - 0.5) * contrast + 0.5 + brightness).clamp(0.0, 1.0);
                Color32::from_gray((adjusted * 255.0).round() as u8)
            })
            .collect();

        ColorImage {
            size: self.display.size,
            pixels,
        }
    }
}

pub fn load_png(path: &Path) -> Result<LoadedImage> {
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
    let hash = blake3::hash(raw).to_hex().to_string();
    let display = make_display_image(frame.width as usize, frame.height as usize, bit_depth, raw)?;

    Ok(LoadedImage {
        path: path.display().to_string(),
        width: frame.width as usize,
        height: frame.height as usize,
        bit_depth,
        hash,
        display,
    })
}

fn make_display_image(width: usize, height: usize, bit_depth: u8, raw: &[u8]) -> Result<ColorImage> {
    let pixels = match bit_depth {
        8 => raw
            .iter()
            .map(|value| Color32::from_gray(*value))
            .collect::<Vec<_>>(),
        16 => raw
            .chunks_exact(2)
            .map(|chunk| {
                let value = u16::from_be_bytes([chunk[0], chunk[1]]);
                Color32::from_gray((value >> 8) as u8)
            })
            .collect::<Vec<_>>(),
        _ => return Err(anyhow!("unsupported bit depth")),
    };

    if pixels.len() != width * height {
        bail!("decoded PNG size did not match its dimensions");
    }

    Ok(ColorImage {
        size: [width, height],
        pixels,
    })
}
