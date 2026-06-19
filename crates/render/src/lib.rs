// Phase 5 — GPU rendering via wgpu (glyph atlas, damage tracking, Mailbox/Immediate present).
// Phase 4 font pipeline (fontconfig + rustybuzz + ab_glyph) feeds the atlas.
// Client-only: must never appear in server-bin's dependency tree.

mod colors;
mod gpu;
mod keys;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{WindowAttributes, WindowId};

use fonts::FontSystem;
use vt::Terminal;

use gpu::Renderer;

const INIT_COLS: usize = 80;
const INIT_ROWS: usize = 24;
// Font size in pixels (ascent + descent).
const FONT_PX: f32 = 16.0;

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
    font_system:   Option<FontSystem>,
    renderer:      Option<Renderer>,
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

        let renderer = pollster::block_on(Renderer::new(win.clone(), w, h));
        match renderer {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                eprintln!("[render] GPU init failed: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if let Some(r) = &self.renderer {
                    let (rendered, skipped) = r.damage_stats();
                    eprintln!("[render] damage stats: {rendered} frames rendered, {skipped} skipped (unchanged)");
                }
                event_loop.exit();
            }

            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
            }

            WindowEvent::Resized(size) => {
                let cols = ((size.width  / self.cell_w) as usize).max(1);
                let rows = ((size.height / self.cell_h) as usize).max(1);
                self.terminal.lock().unwrap().resize(rows, cols);
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                self.draw();
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
        self.draw();
    }
}

impl App {
    fn draw(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        let Some(fs)       = self.font_system.as_ref() else { return };
        let term = self.terminal.lock().unwrap();
        renderer.render(&term, fs, self.cell_w, self.cell_h, self.cell_ascent);
    }
}

/// Connect to (or auto-start) the server at `socket_path`, open a GPU window,
/// and run the render loop. Blocks until the window is closed.
pub fn run_window(socket_path: &str) -> Result<(), RenderError> {
    let stream = connect_or_start(socket_path)?;
    let writer = stream.try_clone()?;

    // Initialise font pipeline; fall back to reasonable defaults if unavailable.
    let (font_system, cell_w, cell_h, cell_ascent) = match FontSystem::new(FONT_PX) {
        Ok(fs) => {
            let m = fs.metrics();
            eprintln!("[render] font metrics: {}×{} cells, ascent={}", m.cell_w, m.cell_h, m.ascent);
            (Some(fs), m.cell_w, m.cell_h, m.ascent)
        }
        Err(e) => {
            eprintln!("[render] font system unavailable ({e}), using defaults");
            (None, 8u32, 16u32, 13u32)
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
        modifiers: ModifiersState::empty(),
        font_system,
        renderer:    None,
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
