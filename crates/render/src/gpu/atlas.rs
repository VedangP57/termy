// Glyph atlas: packs rasterized glyphs onto an R8Unorm GPU texture.
// Uses a simple shelf packer — each new row of glyphs starts a new shelf.

use std::collections::HashMap;

use fonts::RasterizedGlyph;

pub const ATLAS_SIZE: u32 = 1024; // square atlas, 1024×1024

#[derive(Copy, Clone, Debug)]
pub struct AtlasEntry {
    pub uv:        [f32; 4], // [u0, v0, u1, v1] in normalised 0..1 coords
    pub bearing_x: i32,
    pub bearing_y: i32,      // pixels above baseline
    pub advance_x: f32,
    pub width:     u32,      // glyph bitmap width in pixels
    pub height:    u32,      // glyph bitmap height in pixels
}

pub struct GlyphAtlas {
    pub texture: wgpu::Texture,
    pub view:    wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    entries:     HashMap<char, AtlasEntry>,
    // Shelf-packer state.
    shelf_x: u32,
    shelf_y: u32,
    shelf_h: u32,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("glyph_atlas"),
            size:            wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::R8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:       Some("atlas_sampler"),
            mag_filter:  wgpu::FilterMode::Linear,
            min_filter:  wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { texture, view, sampler, entries: HashMap::new(), shelf_x: 0, shelf_y: 0, shelf_h: 0 }
    }

    /// Return the cached atlas entry for `ch`, or rasterize and upload it.
    /// Returns None if `rg` is None (no glyph for this char in the font system).
    pub fn get_or_insert(
        &mut self,
        ch: char,
        rg: Option<&RasterizedGlyph>,
        queue: &wgpu::Queue,
    ) -> Option<AtlasEntry> {
        if let Some(e) = self.entries.get(&ch) {
            return Some(*e);
        }
        let rg = rg?;
        let w = rg.width;
        let h = rg.height;
        if w == 0 || h == 0 { return None; }

        // Advance to next shelf if the current row is full.
        if self.shelf_x + w + 1 > ATLAS_SIZE {
            self.shelf_y += self.shelf_h + 1;
            self.shelf_x  = 0;
            self.shelf_h  = 0;
        }
        if self.shelf_y + h + 1 > ATLAS_SIZE {
            eprintln!("[atlas] glyph atlas full — some chars will be missing");
            return None;
        }

        let x = self.shelf_x;
        let y = self.shelf_y;
        self.shelf_x += w + 1;
        if h > self.shelf_h { self.shelf_h = h; }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &self.texture,
                mip_level: 0,
                origin:    wgpu::Origin3d { x, y, z: 0 },
                aspect:    wgpu::TextureAspect::All,
            },
            &rg.pixels,
            wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(w),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        let entry = AtlasEntry {
            uv: [
                x as f32 / ATLAS_SIZE as f32,
                y as f32 / ATLAS_SIZE as f32,
                (x + w) as f32 / ATLAS_SIZE as f32,
                (y + h) as f32 / ATLAS_SIZE as f32,
            ],
            bearing_x: rg.bearing_x,
            bearing_y: rg.bearing_y,
            advance_x: rg.advance_x,
            width:     w,
            height:    h,
        };
        self.entries.insert(ch, entry);
        Some(entry)
    }
}
