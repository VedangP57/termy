// Phase 6 — terminal feature completeness:
// mouse reporting, scrollback navigation, bracketed paste tracking,
// app cursor keys, window title, DSR response injection.

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
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowAttributes, WindowId};

use fonts::FontSystem;
use vt::{MouseMode, Terminal};

use gpu::Renderer;

const INIT_COLS: usize = 80;
const INIT_ROWS: usize = 24;
const FONT_PX:   f32   = 16.0;
// Lines scrolled per mouse-wheel notch.
const SCROLL_LINES: usize = 3;

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
    window:        Option<Arc<Window>>,
    cell_w:        u32,
    cell_h:        u32,
    cell_ascent:   u32,
    // Scrollback navigation: 0 = live view.
    scroll_offset: usize,
    // Current cursor pixel position (for mouse reporting).
    cursor_px: (f64, f64),
    // Which mouse button is currently held (for ButtonMotion encoding).
    mouse_btn_held: Option<u8>,
}

impl App {
    fn send(&self, bytes: &[u8]) {
        if let Ok(mut g) = self.socket_writer.lock() {
            if let Some(w) = g.as_mut() { let _ = w.write_all(bytes); }
        }
    }

    fn draw(&mut self) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        let Some(fs)       = self.font_system.as_ref() else { return };
        let term = self.terminal.lock().unwrap();
        renderer.render(&term, fs, self.cell_w, self.cell_h, self.cell_ascent, self.scroll_offset);
    }

    fn encode_mouse(&self, btn_code: u8, col: usize, row: usize, release: bool) -> Vec<u8> {
        let term = self.terminal.lock().unwrap();
        if term.screen.mouse_sgr {
            let trailer = if release { b'm' } else { b'M' };
            format!("\x1b[<{};{};{}{}", btn_code, col + 1, row + 1, trailer as char)
                .into_bytes()
        } else {
            // X10 encoding (classic): ESC [ M <btn+32> <col+32> <row+32>
            // Clamp to the encodable range (cols/rows must fit in a byte minus 32).
            let c = ((col + 1 + 32) as u8).min(255);
            let r = ((row + 1 + 32) as u8).min(255);
            vec![0x1b, b'[', b'M', btn_code + 32, c, r]
        }
    }

    fn pixel_to_cell(&self, px: f64, py: f64) -> (usize, usize) {
        let col = (px / self.cell_w as f64) as usize;
        let row = (py / self.cell_h as f64) as usize;
        let term = self.terminal.lock().unwrap();
        let cols = term.screen.cols().saturating_sub(1);
        let rows = term.screen.rows().saturating_sub(1);
        (col.min(cols), row.min(rows))
    }
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
            Ok(r) => {
                self.renderer = Some(r);
                self.window   = Some(win);
            }
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
                    eprintln!("[render] damage stats: {rendered} rendered, {skipped} skipped");
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
                self.scroll_offset = 0;
                self.draw();
            }

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                // Any keypress cancels scrollback view.
                self.scroll_offset = 0;

                let (app_cursor, app_kp) = {
                    let t = self.terminal.lock().unwrap();
                    (t.screen.app_cursor_keys, t.screen.app_keypad)
                };
                let bytes = keys::to_bytes(&event, self.modifiers, app_cursor, app_kp);
                if !bytes.is_empty() {
                    self.send(&bytes);
                }
            }

            WindowEvent::RedrawRequested => self.draw(),

            // ── Mouse wheel ───────────────────────────────────────────────────
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as i32,
                    MouseScrollDelta::PixelDelta(d)   => (d.y / self.cell_h as f64) as i32,
                };
                if lines == 0 { return; }

                let (mouse_mode, in_alt) = {
                    let t = self.terminal.lock().unwrap();
                    (t.screen.mouse_mode, t.screen.is_in_alt())
                };

                if mouse_mode != MouseMode::Off {
                    // Forward scroll as mouse button 64 (up) / 65 (down) per X10 / SGR.
                    let (col, row) = self.pixel_to_cell(self.cursor_px.0, self.cursor_px.1);
                    let btn = if lines > 0 { 64u8 } else { 65u8 };
                    let msg = self.encode_mouse(btn, col, row, false);
                    self.send(&msg);
                } else if in_alt {
                    // Alt-screen with no mouse mode: send arrow key sequences.
                    let seq: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                    for _ in 0..lines.unsigned_abs() { self.send(seq); }
                } else {
                    // Normal screen: scroll the scrollback view.
                    let sb_len = self.terminal.lock().unwrap().screen.scrollback().len();
                    if lines < 0 {
                        self.scroll_offset = (self.scroll_offset + SCROLL_LINES * lines.unsigned_abs() as usize).min(sb_len);
                    } else {
                        self.scroll_offset = self.scroll_offset.saturating_sub(SCROLL_LINES * lines as usize);
                    }
                    self.draw();
                }
            }

            // ── Mouse cursor position ─────────────────────────────────────────
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_px = (position.x, position.y);

                let mouse_mode = self.terminal.lock().unwrap().screen.mouse_mode;
                if mouse_mode == MouseMode::AnyMotion
                    || (mouse_mode == MouseMode::ButtonMotion && self.mouse_btn_held.is_some())
                {
                    let (col, row) = self.pixel_to_cell(position.x, position.y);
                    let btn = self.mouse_btn_held.unwrap_or(3) + 32; // 32 = motion flag
                    let msg = self.encode_mouse(btn, col, row, false);
                    self.send(&msg);
                }
            }

            // ── Mouse buttons ─────────────────────────────────────────────────
            WindowEvent::MouseInput { state, button, .. } => {
                let mouse_mode = self.terminal.lock().unwrap().screen.mouse_mode;
                if mouse_mode == MouseMode::Off { return; }

                let btn_code: u8 = match button {
                    MouseButton::Left   => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right  => 2,
                    _                   => return,
                };
                let (col, row) = self.pixel_to_cell(self.cursor_px.0, self.cursor_px.1);
                let release = state == ElementState::Released;

                if release {
                    self.mouse_btn_held = None;
                } else {
                    self.mouse_btn_held = Some(btn_code);
                }

                let msg = self.encode_mouse(btn_code, col, row, release);
                self.send(&msg);
            }

            _ => {}
        }

        // Propagate window title changes from OSC sequences.
        if let Some(win) = &self.window {
            if let Ok(t) = self.terminal.try_lock() {
                if let Some(title) = &t.screen.window_title {
                    win.set_title(title);
                }
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        self.draw();
    }
}

/// Connect to (or auto-start) the server at `socket_path`, open a GPU window,
/// and run the render loop. Blocks until the window is closed.
pub fn run_window(socket_path: &str) -> Result<(), RenderError> {
    let stream = connect_or_start(socket_path)?;
    let writer = stream.try_clone()?;

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

    let term_clone   = Arc::clone(&terminal);
    let writer_clone = Arc::clone(&socket_writer);
    let mut read_stream = stream;
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match read_stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let responses = {
                        let mut term = term_clone.lock().unwrap();
                        term.advance(&buf[..n]);
                        term.drain_responses()
                    };
                    // Inject terminal responses (DSR, DA) back to PTY via the server.
                    if !responses.is_empty() {
                        if let Ok(mut w) = writer_clone.lock() {
                            if let Some(w) = w.as_mut() {
                                for resp in responses {
                                    let _ = w.write_all(&resp);
                                }
                            }
                        }
                    }
                    let _ = proxy.send_event(());
                }
            }
        }
    });

    let mut app = App {
        terminal,
        socket_writer,
        modifiers:      ModifiersState::empty(),
        font_system,
        renderer:       None,
        window:         None,
        cell_w,
        cell_h,
        cell_ascent,
        scroll_offset:  0,
        cursor_px:      (0.0, 0.0),
        mouse_btn_held: None,
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
