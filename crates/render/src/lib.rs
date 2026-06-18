// Phase 4 — real font rendering via fontconfig + rustybuzz + ab_glyph, with
//           font8x8 kept as a character-level fallback for unparseable glyphs.
// Client-only: must never appear in server-bin's dependency tree.

mod colors;
mod keys;

use std::collections::HashMap;
use std::io::{Read, Write};
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};

use fonts::{FontSystem, RasterizedGlyph};
use vt::Terminal;

const INIT_COLS: usize = 80;
const INIT_ROWS: usize = 24;
// Fallback cell size used when FontSystem initialisation fails.
const FALLBACK_CELL_W: u32 = 8;
const FALLBACK_CELL_H: u32 = 16;

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("server not reachable at {0}")]
    Socket(String),
}

struct App {
    terminal:      Arc<Mutex<Terminal>>,
    socket_writer: Arc<Mutex<Option<UnixStream>>>,
    modifiers:     ModifiersState,
    window:        Option<Arc<Window>>,
    context:       Option<softbuffer::Context<Arc<Window>>>,
    surface:       Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    // Phase 4: real font pipeline.  None only if fontconfig is unavailable.
    font_system:   Option<FontSystem>,
    // Cached rasterized glyphs keyed by char.  None = confirmed unparseable,
    // so we fall back to font8x8 without retrying on every frame.
    glyph_cache:   HashMap<char, Option<RasterizedGlyph>>,
    cell_w:        u32,
    cell_h:        u32,
    cell_ascent:   u32,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let w = (INIT_COLS as u32) * self.cell_w;
        let h = (INIT_ROWS as u32) * self.cell_h;
        let attrs = WindowAttributes::default()
            .with_title("Termy")
            .with_inner_size(PhysicalSize::new(w, h))
            .with_resizable(true);

        let win = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("[render] window creation failed: {e}");
                event_loop.exit();
                return;
            }
        };
        let ctx = match softbuffer::Context::new(win.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[render] softbuffer context failed: {e}");
                event_loop.exit();
                return;
            }
        };
        let surf = match softbuffer::Surface::new(&ctx, win.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[render] softbuffer surface failed: {e}");
                event_loop.exit();
                return;
            }
        };
        self.context = Some(ctx);
        self.surface = Some(surf);
        self.window  = Some(win);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }

            WindowEvent::Resized(size) => {
                let cols = ((size.width  / self.cell_w) as usize).max(1);
                let rows = ((size.height / self.cell_h) as usize).max(1);
                self.terminal.lock().unwrap().resize(rows, cols);
                if let Some(w) = &self.window { w.request_redraw(); }
            }

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let bytes = keys::to_bytes(&event, self.modifiers);
                if !bytes.is_empty() {
                    if let Ok(mut g) = self.socket_writer.lock() {
                        if let Some(w) = g.as_mut() { let _ = w.write_all(&bytes); }
                    }
                }
            }

            WindowEvent::RedrawRequested => self.draw(),

            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(w) = &self.window { w.request_redraw(); }
    }
}

impl App {
    fn draw(&mut self) {
        let Some(surf) = self.surface.as_mut() else { return };
        let Some(win)  = self.window.as_ref()   else { return };

        let size   = win.inner_size();
        let width  = size.width;
        let height = size.height;
        if width == 0 || height == 0 { return; }

        if surf.resize(
            NonZeroU32::new(width).unwrap(),
            NonZeroU32::new(height).unwrap(),
        ).is_err() { return; }

        let Ok(mut buf) = surf.buffer_mut() else { return };
        let term = self.terminal.lock().unwrap();
        let rows = term.screen.rows();
        let cols = term.screen.cols();
        let cell_w    = self.cell_w;
        let cell_h    = self.cell_h;
        let cell_asc  = self.cell_ascent;

        buf.fill(0x000000);

        for row in 0..rows {
            for col in 0..cols {
                let cell = term.screen.cell(row, col);
                let (fg, bg) = if cell.attrs.inverse {
                    (colors::to_argb(cell.attrs.bg, false), colors::to_argb(cell.attrs.fg, true))
                } else {
                    (colors::to_argb(cell.attrs.fg, true),  colors::to_argb(cell.attrs.bg, false))
                };

                let px = (col as u32) * cell_w;
                let py = (row as u32) * cell_h;

                // Fill background rectangle.
                if bg != 0x000000 {
                    for dy in 0..cell_h {
                        for dx in 0..cell_w {
                            let x = px + dx;
                            let y = py + dy;
                            if x < width && y < height {
                                buf[(y * width + x) as usize] = bg;
                            }
                        }
                    }
                }

                // Render glyph: real font first, font8x8 as fallback.
                if cell.ch != ' ' {
                    let rg = self.glyph_cache.entry(cell.ch).or_insert_with(|| {
                        self.font_system
                            .as_ref()
                            .and_then(|fs| fs.rasterize(cell.ch))
                    });

                    if let Some(rg) = rg.as_ref() {
                        // Baseline-relative placement.
                        // bearing_y is upward distance from baseline to top of bitmap.
                        let bx = rg.bearing_x;
                        let by = rg.bearing_y as i32;
                        let base_y = (py + cell_asc) as i32;

                        for gy in 0..rg.height {
                            for gx in 0..rg.width {
                                let cov = rg.pixels[(gy * rg.width + gx) as usize];
                                if cov == 0 { continue; }
                                let sx = px as i32 + bx + gx as i32;
                                let sy = base_y - by + gy as i32;
                                if sx < 0 || sy < 0 { continue; }
                                let sx = sx as u32;
                                let sy = sy as u32;
                                if sx >= width || sy >= height { continue; }

                                let idx = (sy * width + sx) as usize;
                                if cov == 255 {
                                    buf[idx] = fg;
                                } else {
                                    // Alpha-blend coverage over the existing background pixel.
                                    let dst = buf[idx];
                                    let a = cov as u32;
                                    let ia = 255 - a;
                                    let r = (((fg >> 16) & 0xFF) * a + ((dst >> 16) & 0xFF) * ia) / 255;
                                    let g = (((fg >>  8) & 0xFF) * a + ((dst >>  8) & 0xFF) * ia) / 255;
                                    let b = ((fg & 0xFF) * a + (dst & 0xFF) * ia) / 255;
                                    buf[idx] = (r << 16) | (g << 8) | b;
                                }
                            }
                        }
                    } else {
                        // font8x8 fallback: 8×8 bitmap stretched to cell dimensions.
                        let glyph_idx = if (cell.ch as u32) < 128 {
                            cell.ch as usize
                        } else {
                            b'?' as usize
                        };
                        let bitmap = font8x8::legacy::BASIC_LEGACY[glyph_idx];
                        let scale_x = cell_w.max(8) / 8;
                        let scale_y = cell_h.max(8) / 8;
                        for (gy, row_byte) in bitmap.iter().enumerate() {
                            for bit in 0..8u32 {
                                if (row_byte >> (7 - bit)) & 1 == 0 { continue; }
                                for sy in 0..scale_y {
                                    for sx in 0..scale_x {
                                        let x = px + bit * scale_x + sx;
                                        let y = py + (gy as u32) * scale_y + sy;
                                        if x < width && y < height {
                                            buf[(y * width + x) as usize] = fg;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Full-height block cursor (2 px wide) via XOR invert.
        let cur = term.screen.cursor();
        if cur.visible && cur.row < rows && cur.col < cols {
            let px = (cur.col as u32) * cell_w;
            let py = (cur.row as u32) * cell_h;
            for dy in 0..cell_h {
                for dx in 0..2u32 {
                    let x = px + dx;
                    let y = py + dy;
                    if x < width && y < height {
                        buf[(y * width + x) as usize] ^= 0x00_FF_FF_FF;
                    }
                }
            }
        }

        drop(term);
        let _ = buf.present();
    }
}

/// Connect to (or auto-start) the server at `socket_path`, open a window,
/// and run the render loop. Blocks until the window is closed.
pub fn run_window(socket_path: &str) -> Result<(), RenderError> {
    let stream = connect_or_start(socket_path)?;
    let writer = stream.try_clone()?;

    // Initialise the font pipeline. Fall back to font8x8 metrics if unavailable.
    let (font_system, cell_w, cell_h, cell_ascent) = match FontSystem::new(16.0) {
        Ok(fs) => {
            let m = fs.metrics();
            (Some(fs), m.cell_w, m.cell_h, m.ascent)
        }
        Err(e) => {
            eprintln!("[render] font system unavailable ({e}), using bitmap fallback");
            (None, FALLBACK_CELL_W, FALLBACK_CELL_H, 13u32)
        }
    };

    let terminal      = Arc::new(Mutex::new(Terminal::new(INIT_ROWS, INIT_COLS)));
    let socket_writer = Arc::new(Mutex::new(Some(writer)));

    let event_loop = EventLoop::new()?;
    let proxy      = event_loop.create_proxy();

    let term_clone      = Arc::clone(&terminal);
    let mut read_stream = stream;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match read_stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    term_clone.lock().unwrap().advance(&buf[..n]);
                    let _ = proxy.send_event(());
                }
            }
        }
    });

    let mut app = App {
        terminal,
        socket_writer,
        modifiers:   ModifiersState::empty(),
        window:      None,
        context:     None,
        surface:     None,
        font_system,
        glyph_cache: HashMap::new(),
        cell_w,
        cell_h,
        cell_ascent,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn connect_or_start(socket_path: &str) -> Result<UnixStream, RenderError> {
    if let Ok(s) = UnixStream::connect(socket_path) { return Ok(s); }

    let exe = std::env::current_exe()?;
    std::process::Command::new(&exe)
        .args(["--server", socket_path])
        .spawn()?;

    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(s) = UnixStream::connect(socket_path) { return Ok(s); }
    }
    Err(RenderError::Socket(socket_path.to_owned()))
}
