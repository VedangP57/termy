// Phase 4 — font discovery (fontconfig), shaping (rustybuzz), rasterization (ab_glyph).
// Client-only: must never appear in server-bin's dependency tree.

use std::path::PathBuf;
use std::sync::Arc;

use ab_glyph::{Font, PxScale, ScaleFont};
use thiserror::Error;

// ─── public types ─────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum FontsError {
    #[error("fontconfig initialization failed")]
    FcInit,
    #[error("no monospace font found on system")]
    NoMonoFont,
    #[error("cannot read font file {0}")]
    ReadFail(PathBuf),
    #[error("font parse error for {0}")]
    ParseError(PathBuf),
}

/// Coverage-mask bitmap for one glyph, ready to blit.
#[derive(Debug, Clone)]
pub struct RasterizedGlyph {
    /// 8-bit coverage values (0=transparent, 255=opaque), row-major, top-left origin.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Pixels from the pen origin (left edge of cell) to left edge of bitmap.
    pub bearing_x: i32,
    /// Pixels from the baseline upward to the top of the bitmap.
    /// Positive = above baseline.
    pub bearing_y: i32,
    /// Horizontal advance for the pen after this glyph.
    pub advance_x: f32,
}

/// Font cell metrics for the render crate's layout.
#[derive(Debug, Clone, Copy)]
pub struct FontMetrics {
    /// Width of one monospace cell in pixels.
    pub cell_w: u32,
    /// Height of one monospace cell in pixels (ascent + descent + line_gap).
    pub cell_h: u32,
    /// Pixels from top of cell down to the text baseline.
    pub ascent: u32,
}

/// One glyph returned from a rustybuzz shaping run.
#[derive(Debug, Clone)]
pub struct ShapedGlyph {
    pub glyph_id: u32,
    /// Source string byte index (multiple ShapedGlyphs may share a cluster for ligatures).
    pub cluster: u32,
    /// x-advance in fractional font units (divide by 64 for approximate pixels).
    pub advance_x: i32,
    pub advance_y: i32,
    pub offset_x: i32,
    pub offset_y: i32,
}

// ─── internal face ────────────────────────────────────────────────────────────

struct LoadedFace {
    data: Arc<Vec<u8>>,
}

impl LoadedFace {
    fn load(path: &PathBuf) -> Result<Self, FontsError> {
        let data = std::fs::read(path).map_err(|_| FontsError::ReadFail(path.clone()))?;
        // Validate the file is parseable before storing it.
        ab_glyph::FontRef::try_from_slice(&data)
            .map_err(|_| FontsError::ParseError(path.clone()))?;
        Ok(Self { data: Arc::new(data) })
    }

    /// True when this font's cmap has an entry for `ch`.
    fn has_glyph(&self, ch: char) -> bool {
        ab_glyph::FontRef::try_from_slice(&self.data)
            .map(|f| f.glyph_id(ch).0 != 0)
            .unwrap_or(false)
    }

    /// Rasterize `ch` at `px_size` pixels tall.  Returns None when the font
    /// has no outline for this character (space, .notdef, or colour-bitmap-only).
    fn rasterize(&self, ch: char, px_size: f32) -> Option<RasterizedGlyph> {
        let font = ab_glyph::FontRef::try_from_slice(&self.data).ok()?;
        let scale = PxScale::from(px_size);
        let sf = font.as_scaled(scale);

        let gid = font.glyph_id(ch);
        if gid.0 == 0 {
            return None; // character not in this font's cmap
        }

        let advance = sf.h_advance(gid);
        let glyph = gid.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
        let outlined = sf.outline_glyph(glyph)?; // None for spaces and .notdef without outlines

        let bounds = outlined.px_bounds();
        let w = bounds.width().ceil() as u32;
        let h = bounds.height().ceil() as u32;
        if w == 0 || h == 0 {
            return None;
        }

        let mut pixels = vec![0u8; (w * h) as usize];
        outlined.draw(|x, y, cov| {
            let idx = (y * w + x) as usize;
            if idx < pixels.len() {
                pixels[idx] = (cov * 255.0) as u8;
            }
        });

        Some(RasterizedGlyph {
            pixels,
            width: w,
            height: h,
            bearing_x: bounds.min.x.round() as i32,
            // bounds.min.y is negative for glyphs above the baseline; negate to get upward distance.
            bearing_y: (-bounds.min.y).round() as i32,
            advance_x: advance,
        })
    }

    /// Shape `text` with rustybuzz.  Ligatures produce a single ShapedGlyph
    /// whose `cluster` index equals the first input byte's position.
    fn shape(&self, text: &str) -> Vec<ShapedGlyph> {
        let face = match rustybuzz::Face::from_slice(&self.data, 0) {
            Some(f) => f,
            None => return vec![],
        };
        let mut buf = rustybuzz::UnicodeBuffer::new();
        buf.push_str(text);
        let result = rustybuzz::shape(&face, &[], buf);
        result
            .glyph_infos()
            .iter()
            .zip(result.glyph_positions().iter())
            .map(|(info, pos)| ShapedGlyph {
                glyph_id: info.glyph_id,
                cluster:  info.cluster,
                advance_x: pos.x_advance,
                advance_y: pos.y_advance,
                offset_x:  pos.x_offset,
                offset_y:  pos.y_offset,
            })
            .collect()
    }
}

// ─── public FontSystem ────────────────────────────────────────────────────────

/// Primary monospace face + ordered fallback chain (emoji, CJK, symbols).
pub struct FontSystem {
    primary:   LoadedFace,
    fallbacks: Vec<LoadedFace>,
    px_size:   f32,
    metrics:   FontMetrics,
}

impl FontSystem {
    /// Discover fonts via fontconfig and build the font pipeline.
    pub fn new(px_size: f32) -> Result<Self, FontsError> {
        let fc = fontconfig::Fontconfig::new().ok_or(FontsError::FcInit)?;

        // Best available monospace font.
        let mono_font = fc
            .find("monospace", Some("regular"))
            .or_else(|_| fc.find("monospace", None))
            .or_else(|_| fc.find("Mono", None))
            .or_else(|_| fc.find("Courier", None))
            .map_err(|_| FontsError::NoMonoFont)?;
        let primary = LoadedFace::load(&mono_font.path)?;

        // Emoji and CJK fallbacks — loaded in preference order, silently skipped when absent.
        let fallback_families: &[&str] = &[
            "Apple Color Emoji",
            "Noto Color Emoji",
            "Segoe UI Emoji",
            "Noto Sans CJK SC",
            "Noto Sans CJK TC",
            "WenQuanYi Micro Hei",
            "Hiragino Sans GB",
            "Source Han Sans CN",
            "Symbols Nerd Font",
        ];
        let fallbacks: Vec<LoadedFace> = fallback_families
            .iter()
            .filter_map(|fam| fc.find(fam, None).ok())
            .filter_map(|font| LoadedFace::load(&font.path).ok())
            .collect();

        let metrics = compute_metrics(&primary, px_size);
        Ok(Self { primary, fallbacks, px_size, metrics })
    }

    /// Rasterize `ch`, trying the primary face then each fallback in order.
    pub fn rasterize(&self, ch: char) -> Option<RasterizedGlyph> {
        self.primary.rasterize(ch, self.px_size).or_else(|| {
            self.fallbacks.iter().find_map(|fb| fb.rasterize(ch, self.px_size))
        })
    }

    /// Shape `text` using the primary face.
    /// Ligatures appear as a single ShapedGlyph spanning multiple input clusters.
    pub fn shape(&self, text: &str) -> Vec<ShapedGlyph> {
        self.primary.shape(text)
    }

    /// True when any face in the fallback chain has a glyph for `ch`.
    pub fn has_glyph(&self, ch: char) -> bool {
        self.primary.has_glyph(ch) || self.fallbacks.iter().any(|fb| fb.has_glyph(ch))
    }

    pub fn metrics(&self) -> FontMetrics { self.metrics }
    pub fn px_size(&self) -> f32 { self.px_size }
}

fn compute_metrics(face: &LoadedFace, px_size: f32) -> FontMetrics {
    let font = match ab_glyph::FontRef::try_from_slice(&face.data) {
        Ok(f) => f,
        Err(_) => return FontMetrics { cell_w: 8, cell_h: 16, ascent: 13 },
    };
    let scale = PxScale::from(px_size);
    let sf = font.as_scaled(scale);

    let ascent_px  = sf.ascent().round() as u32;
    let descent_px = sf.descent().abs().round() as u32;
    let gap_px     = sf.line_gap().round() as u32;
    let cell_h     = ascent_px + descent_px + gap_px;

    // For a monospace font every glyph has the same advance; 'm' is the canonical choice.
    let m_id  = font.glyph_id('m');
    let cell_w = if m_id.0 != 0 {
        sf.h_advance(m_id).round() as u32
    } else {
        sf.h_advance(font.glyph_id(' ')).round() as u32
    };

    FontMetrics {
        cell_w: cell_w.max(1),
        cell_h: cell_h.max(1),
        ascent: ascent_px,
    }
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn fs() -> FontSystem {
        FontSystem::new(16.0).expect("FontSystem::new should succeed")
    }

    #[test]
    fn rasterize_ascii_letter() {
        let g = fs().rasterize('A').expect("'A' must rasterize from primary font");
        assert!(g.width > 0 && g.height > 0, "glyph bitmap must be non-empty");
        assert!(
            g.pixels.iter().any(|&p| p > 0),
            "at least one pixel must be non-zero"
        );
    }

    #[test]
    fn metrics_are_reasonable() {
        let m = fs().metrics();
        assert!(m.cell_w >= 4, "cell_w too small: {}", m.cell_w);
        assert!(m.cell_h >= 8, "cell_h too small: {}", m.cell_h);
        assert!(m.ascent > 0 && m.ascent < m.cell_h, "ascent out of range");
    }

    #[test]
    fn cjk_covered_by_fallback() {
        let system = fs();
        assert!(
            system.has_glyph('日'),
            "no font in fallback chain covers CJK '日' — add a CJK font to this system"
        );
    }

    #[test]
    fn emoji_covered_by_fallback() {
        let system = fs();
        // Test with a code-point that most emoji fonts include.
        // has_glyph checks the cmap rather than requiring a renderable outline,
        // which correctly handles colour-bitmap-only fonts like Apple Color Emoji.
        assert!(
            system.has_glyph('😀') || system.has_glyph('🦀'),
            "no font in fallback chain covers emoji — add an emoji font to this system"
        );
    }

    #[test]
    fn shape_fi_ligature_or_pair() {
        let glyphs = fs().shape("fi");
        assert!(!glyphs.is_empty(), "shaping 'fi' must return at least one glyph");
        if glyphs.len() == 1 {
            // Ligature: single glyph, cluster 0.
            assert_eq!(glyphs[0].cluster, 0, "ligature glyph must reference cluster 0");
        } else {
            // No ligature in this font: two separate glyphs.
            assert_eq!(glyphs.len(), 2, "expected exactly 2 glyphs for non-ligature 'fi'");
        }
    }

    #[test]
    fn shape_does_not_crash_on_unicode() {
        let system = fs();
        // These cover a wide range of Unicode categories; the shaper must not panic.
        for text in &["hello", "日本語", "🦀🦞", "fn() -> i32", "fi ffi"] {
            let _ = system.shape(text);
        }
    }
}
