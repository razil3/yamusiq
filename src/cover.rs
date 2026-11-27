//! Experimental TUI-only cover treatment. System artwork remains untouched.
pub const SIDE: usize = 128;

#[derive(Debug)]
pub struct Preview {
    pixels: [u8; SIDE * SIDE],
    low: u8,
    high: u8,
}

impl Preview {
    pub fn from_luma(pixels: [u8; SIDE * SIDE]) -> Self {
        let mut sorted = pixels;
        sorted.sort_unstable();
        Self {
            pixels,
            low: sorted[SIDE * SIDE / 50],
            high: sorted[SIDE * SIDE * 49 / 50],
        }
    }

    /// Eight dots per cell: twice the vertical detail of quadrant blocks.
    pub fn cell(&self, x: usize, y: usize, width: usize, rows: usize) -> (char, u8, u8) {
        let samples = std::array::from_fn(|i| {
            self.sample(x * 2 + i % 2, y * 4 + i / 2, width * 2, rows * 4)
        });
        braille_cell(samples, x)
    }

    #[cfg(test)]
    pub fn shade(&self, x: usize, y: usize, side: usize) -> u8 {
        self.sample(x, y, side, side)
    }

    /// Area-average without introducing a noisy dither pattern.
    fn sample(&self, x: usize, y: usize, width: usize, height: usize) -> u8 {
        let sx = SIDE as f32 / width as f32;
        let sy = SIDE as f32 / height as f32;
        let x0 = x as f32 * sx;
        let x1 = (x + 1) as f32 * sx;
        let y0 = y as f32 * sy;
        let y1 = (y + 1) as f32 * sy;
        let mut sum = 0.0;
        let mut weight = 0.0;
        for py in y0.floor() as usize..(y1.ceil() as usize).min(SIDE) {
            let wy = (y1.min((py + 1) as f32) - y0.max(py as f32)).max(0.0);
            for px in x0.floor() as usize..(x1.ceil() as usize).min(SIDE) {
                let wx = (x1.min((px + 1) as f32) - x0.max(px as f32)).max(0.0);
                sum += f32::from(self.pixels[py * SIDE + px]) * wx * wy;
                weight += wx * wy;
            }
        }
        let value = sum / weight;
        let value = if self.high.saturating_sub(self.low) >= 24 {
            ((value - f32::from(self.low)) / f32::from(self.high - self.low)).clamp(0.0, 1.0)
        } else {
            value / 255.0
        };
        (16.0 + value.powf(0.9) * 208.0).round() as u8
    }
}

/// Use a spatially stable ordered screen for tones, not a per-frame random
/// dither. Foreground follows the local highlight; background stays dark so
/// the result is dots rather than another mosaic of rectangular grey cells.
fn braille_cell(samples: [u8; 8], x: usize) -> (char, u8, u8) {
    const BITS: [u8; 8] = [0, 3, 1, 4, 2, 5, 6, 7];
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let fg = samples.iter().copied().max().unwrap_or(16).max(176);
    let mut mask = 0u8;
    for (i, sample) in samples.iter().enumerate() {
        let tone = (f32::from(sample.saturating_sub(16)) / f32::from(fg - 16)).powf(0.85);
        let threshold = (f32::from(BAYER[i / 2][(x * 2 + i % 2) % 4]) + 0.5) / 16.0;
        if tone > threshold {
            mask |= 1 << BITS[i];
        }
    }
    (char::from_u32(0x2800 + u32::from(mask)).unwrap(), fg, 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treatment_is_bounded_and_flat_images_have_no_dither() {
        for value in [0, 128, 255] {
            let preview = Preview::from_luma([value; SIDE * SIDE]);
            let expected = preview.shade(0, 0, 16);
            for side in [10, 12, 16, 18, 20, 24, 28, 32] {
                for y in 0..side {
                    for x in 0..side {
                        let grey = preview.shade(x, y, side);
                        assert!((16..=224).contains(&grey));
                        assert_eq!(grey, expected);
                    }
                }
            }
        }
        assert_eq!(Preview::from_luma([0; SIDE * SIDE]).shade(0, 0, 16), 16);
        assert_eq!(Preview::from_luma([255; SIDE * SIDE]).shade(0, 0, 16), 224);
    }

    #[test]
    fn braille_dot_positions_and_dark_background_are_correct() {
        for (i, bit) in [0, 3, 1, 4, 2, 5, 6, 7].into_iter().enumerate() {
            let mut samples = [16; 8];
            samples[i] = 224;
            for x in 0..2 {
                let (glyph, fg, bg) = braille_cell(samples, x);
                assert_eq!(glyph as u32, 0x2800 + (1 << bit));
                assert_eq!((fg, bg), (224, 16));
            }
        }
        assert_eq!(braille_cell([16; 8], 0).0, '\u{2800}');
        assert_eq!(braille_cell([224; 8], 0).0, '⣿');
    }

    #[test]
    fn dot_density_increases_with_brightness_without_animation() {
        let mut previous = 0;
        for value in 16..=224 {
            let count = (0..2).map(|x| {
                let cell = braille_cell([value; 8], x);
                assert_eq!(cell, braille_cell([value; 8], x));
                (cell.0 as u32 - 0x2800).count_ones()
            }).sum::<u32>();
            assert!(count >= previous);
            previous = count;
        }
        assert_eq!(previous, 16);
        let pixels = std::array::from_fn(|i| ((i % SIDE) * 255 / (SIDE - 1)) as u8);
        let preview = Preview::from_luma(pixels);
        for side in [10, 12, 16, 18, 20, 24, 28, 32] {
            for y in 0..side / 2 {
                for x in 0..side {
                    let (glyph, fg, bg) = preview.cell(x, y, side, side / 2);
                    assert!(('\u{2800}'..='\u{28ff}').contains(&glyph));
                    assert!((176..=224).contains(&fg));
                    assert_eq!(bg, 16);
                }
            }
        }
    }

    #[test]
    fn gradient_retains_more_than_eight_distinct_shades() {
        let pixels = std::array::from_fn(|i| ((i % SIDE) * 255 / (SIDE - 1)) as u8);
        let preview = Preview::from_luma(pixels);
        let shades: Vec<_> = (0..32).map(|x| preview.shade(x, 0, 32)).collect();
        assert!(shades.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
