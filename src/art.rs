//! Album-art processing for the animated background and accent colours.
//!
//! Follows Kawarp's pipeline (MIT, Better Lyrics, used by better-lyrics-shaders):
//! tint the dark areas, then blur heavily. Kawarp does this on the GPU every frame;
//! here it runs once per track on a 64×64 image, so the per-frame shader is cheap.

use egui::Color32;

pub const SIZE: u32 = 64;

#[derive(Clone)]
pub struct Art {
    /// SIZE×SIZE RGBA, already tinted and blurred.
    pub rgba: Vec<u8>,
    /// Most vivid colour, lifted for use on dark backgrounds (progress bar, highlights).
    pub accent: Color32,
    /// Most used colour across the artwork, managed so it is neither too dark nor too white.
    pub dominant: Color32,
}

pub fn process(bytes: &[u8]) -> anyhow::Result<Art> {
    let img = image::load_from_memory(bytes)?.to_rgb8();
    let small = image::imageops::resize(&img, SIZE, SIZE, image::imageops::FilterType::Triangle);

    let (accent, dominant) = palette(&small);
    let mut px: Vec<[f32; 3]> = small.pixels().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();

    // Kawarp's tint pass: pull near-black regions toward the accent so dark covers
    // still produce a coloured backdrop instead of a black one.
    let tint = [accent.r() as f32, accent.g() as f32, accent.b() as f32];
    for p in &mut px {
        let luma = (0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2]) / 255.0;
        let dark = 1.0 - smoothstep(0.0, 0.5, luma);
        for c in 0..3 {
            p[c] += (tint[c] - p[c]) * dark * 0.45;
        }
    }
    // Four wide box-blur passes produce an ultra-smooth, creamy Gaussian blur effect.
    for _ in 0..4 {
        px = box_blur(&px, SIZE as usize, 8);
    }
    let rgba = px.iter().flat_map(|p| [p[0] as u8, p[1] as u8, p[2] as u8, 255]).collect();
    Ok(Art { rgba, accent, dominant })
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn box_blur(src: &[[f32; 3]], n: usize, r: usize) -> Vec<[f32; 3]> {
    let pass = |src: &[[f32; 3]], horizontal: bool| -> Vec<[f32; 3]> {
        let mut out = vec![[0.0; 3]; src.len()];
        for y in 0..n {
            for x in 0..n {
                let mut acc = [0.0; 3];
                for k in 0..=2 * r {
                    let o = k as isize - r as isize;
                    let (sx, sy) = if horizontal {
                        ((x as isize + o).clamp(0, n as isize - 1) as usize, y)
                    } else {
                        (x, (y as isize + o).clamp(0, n as isize - 1) as usize)
                    };
                    let p = src[sy * n + sx];
                    for c in 0..3 {
                        acc[c] += p[c];
                    }
                }
                let d = (2 * r + 1) as f32;
                out[y * n + x] = [acc[0] / d, acc[1] / d, acc[2] / d];
            }
        }
        out
    };
    pass(&pass(src, true), false)
}

/// Extracts the accent colour (vivid) and the dominant/most-used colour in the artwork,
/// managed so it is neither too dark nor too white.
fn palette(img: &image::RgbImage) -> (Color32, Color32) {
    let mut sum = [0.0f32; 3];
    let mut total_pixels = 0.0f32;

    #[derive(Default, Clone, Copy)]
    struct Bin {
        count: u32,
        sum: [f32; 3],
    }
    let mut bins = [Bin { count: 0, sum: [0.0; 3] }; 4096];

    for p in img.pixels() {
        let rgb = [p[0] as f32, p[1] as f32, p[2] as f32];
        for c in 0..3 {
            sum[c] += rgb[c];
        }
        total_pixels += 1.0;

        let r_bin = (p[0] >> 4) as usize;
        let g_bin = (p[1] >> 4) as usize;
        let b_bin = (p[2] >> 4) as usize;
        let idx = (r_bin << 8) | (g_bin << 4) | b_bin;

        bins[idx].count += 1;
        bins[idx].sum[0] += rgb[0];
        bins[idx].sum[1] += rgb[1];
        bins[idx].sum[2] += rgb[2];
    }

    // 1. Most used color across the artwork (highest bin pixel count)
    let mut max_count = 0;
    let mut most_used = [128.0f32, 128.0f32, 128.0f32];
    for bin in &bins {
        if bin.count > max_count {
            max_count = bin.count;
            let c = bin.count as f32;
            most_used = [bin.sum[0] / c, bin.sum[1] / c, bin.sum[2] / c];
        }
    }

    // Manage dominant color: don't make it too dark or too white, keeping the artwork's hue/tint
    let (mut dom_r, mut dom_g, mut dom_b) = (most_used[0] / 255.0, most_used[1] / 255.0, most_used[2] / 255.0);
    let luma = 0.299 * dom_r + 0.587 * dom_g + 0.114 * dom_b;
    if luma > 0.82 {
        let factor = 0.82 / luma;
        dom_r *= factor;
        dom_g *= factor;
        dom_b *= factor;
    } else if luma < 0.16 {
        let factor = 0.18 / luma.max(0.01);
        dom_r = (dom_r * factor).min(1.0);
        dom_g = (dom_g * factor).min(1.0);
        dom_b = (dom_b * factor).min(1.0);
    }
    let dominant = Color32::from_rgb(
        (dom_r * 255.0).round() as u8,
        (dom_g * 255.0).round() as u8,
        (dom_b * 255.0).round() as u8,
    );

    // 2. Vivid accent color for UI highlights, buttons, and progress bar
    let mut best_score = 0.0f32;
    let mut best_color = [200.0, 60.0, 80.0];

    for bin in &bins {
        if bin.count == 0 {
            continue;
        }
        let count_f = bin.count as f32;
        let r = bin.sum[0] / count_f / 255.0;
        let g = bin.sum[1] / count_f / 255.0;
        let b = bin.sum[2] / count_f / 255.0;

        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = 0.299 * r + 0.587 * g + 0.114 * b;
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };

        // Exclude extreme dark (pure black) and extreme washed-out white
        if l < 0.06 || (l > 0.94 && sat < 0.12) {
            continue;
        }

        // Presence: strongly rewards colours that cover the greatest amount of the artwork
        let presence = count_f / total_pixels;
        // Saturation factor: rewards chromatic tones over muddy grey
        let sat_factor = (sat + 0.25).min(1.0);
        // Lightness balance: midtones look richest on backgrounds
        let luma_factor = 1.0 - (l - 0.52).abs() * 0.7;

        let score = presence.powf(0.7) * sat_factor * luma_factor;
        if score > best_score {
            best_score = score;
            best_color = [bin.sum[0] / count_f, bin.sum[1] / count_f, bin.sum[2] / count_f];
        }
    }

    let avg = [sum[0] / total_pixels, sum[1] / total_pixels, sum[2] / total_pixels];
    let accent_raw = if best_score > 0.01 { best_color } else { avg };
    let accent = lift(accent_raw);

    (accent, dominant)
}

/// Raise brightness so the colour reads on a dark UI.
fn lift(c: [f32; 3]) -> Color32 {
    let max = c[0].max(c[1]).max(c[2]).max(1.0);
    let k = (220.0 / max).max(1.0);
    Color32::from_rgb((c[0] * k).min(255.0) as u8, (c[1] * k).min(255.0) as u8, (c[2] * k).min(255.0) as u8)
}

/// A neutral backdrop for when nothing is playing.
pub fn idle() -> Art {
    let n = SIZE as usize;
    let mut rgba = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 / n as f32, y as f32 / n as f32);
            let r = 40.0 + 70.0 * (1.0 - fy) * fx;
            let g = 18.0 + 10.0 * fy;
            let b = 46.0 + 60.0 * fy * (1.0 - fx);
            rgba.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        }
    }
    Art { rgba, accent: Color32::from_rgb(0xff, 0x4e, 0x6a), dominant: Color32::from_rgb(0x18, 0x14, 0x1c) }
}
