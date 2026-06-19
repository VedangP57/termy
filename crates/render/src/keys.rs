use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Translate a winit key-press event into the byte sequence the PTY expects.
///
/// `app_cursor_keys`:  DECCKM mode 1 — arrows send SS3 (ESC O …) instead of CSI (ESC [ …).
/// `app_keypad`:       DECKPAM — numpad keys send application sequences.
/// `keyboard_flags`:   Kitty keyboard protocol flags (0 = off, bit 0 = disambiguate).
pub fn to_bytes(event: &KeyEvent, mods: ModifiersState, app_cursor_keys: bool, _app_keypad: bool, keyboard_flags: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let ctrl      = mods.control_key();
    let shift     = mods.shift_key();
    let alt       = mods.alt_key();
    let disambig  = keyboard_flags & 1 != 0;

    // Kitty CSI-u modifier encoding: 1 + shift*1 + alt*2 + ctrl*4
    let kitty_mods = 1u32
        + if shift { 1 } else { 0 }
        + if alt   { 2 } else { 0 }
        + if ctrl  { 4 } else { 0 };

    match &event.logical_key {
        Key::Character(s) => {
            if ctrl {
                if let Some(ch) = s.chars().next() {
                    let lo = ch.to_ascii_lowercase();
                    if disambig {
                        // Kitty disambiguate: send CSI codepoint ; mods u
                        let cp = lo as u32;
                        let seq = format!("\x1b[{};{}u", cp, kitty_mods);
                        out.extend_from_slice(seq.as_bytes());
                        return out;
                    }
                    match lo {
                        'a'..='z' => { out.push(lo as u8 - b'a' + 1); return out; }
                        '[' => { out.push(0x1b); return out; }
                        '\\' => { out.push(0x1c); return out; }
                        ']' => { out.push(0x1d); return out; }
                        _ => {}
                    }
                }
            }
            out.extend_from_slice(s.as_bytes());
        }
        Key::Named(name) => {
            // Kitty disambiguate mode: send CSI-u for traditionally ambiguous named keys
            // when modifiers are present, so the app can distinguish e.g. Shift-Enter.
            if disambig && kitty_mods > 1 {
                let cp: Option<u32> = match name {
                    NamedKey::Enter     => Some(13),
                    NamedKey::Tab       => Some(9),
                    NamedKey::Escape    => Some(27),
                    NamedKey::Backspace => Some(127),
                    _ => None,
                };
                if let Some(cp) = cp {
                    let seq = format!("\x1b[{};{}u", cp, kitty_mods);
                    out.extend_from_slice(seq.as_bytes());
                    return out;
                }
            }

            // Arrow keys: CSI in normal mode, SS3 in application-cursor-keys mode.
            if app_cursor_keys {
                let seq: &[u8] = match name {
                    NamedKey::ArrowUp    => b"\x1bOA",
                    NamedKey::ArrowDown  => b"\x1bOB",
                    NamedKey::ArrowRight => b"\x1bOC",
                    NamedKey::ArrowLeft  => b"\x1bOD",
                    _ => b"",
                };
                if !seq.is_empty() {
                    out.extend_from_slice(seq);
                    return out;
                }
            }

            let seq: &[u8] = match name {
                NamedKey::Enter        => b"\r",
                NamedKey::Backspace    => b"\x7f",
                NamedKey::Delete       => b"\x1b[3~",
                NamedKey::Tab          => b"\t",
                NamedKey::Escape       => b"\x1b",
                NamedKey::ArrowUp      => b"\x1b[A",
                NamedKey::ArrowDown    => b"\x1b[B",
                NamedKey::ArrowRight   => b"\x1b[C",
                NamedKey::ArrowLeft    => b"\x1b[D",
                NamedKey::Home         => b"\x1b[H",
                NamedKey::End          => b"\x1b[F",
                NamedKey::PageUp       => b"\x1b[5~",
                NamedKey::PageDown     => b"\x1b[6~",
                NamedKey::Insert       => b"\x1b[2~",
                NamedKey::F1           => b"\x1bOP",
                NamedKey::F2           => b"\x1bOQ",
                NamedKey::F3           => b"\x1bOR",
                NamedKey::F4           => b"\x1bOS",
                NamedKey::F5           => b"\x1b[15~",
                NamedKey::F6          => b"\x1b[17~",
                NamedKey::F7           => b"\x1b[18~",
                NamedKey::F8           => b"\x1b[19~",
                NamedKey::F9           => b"\x1b[20~",
                NamedKey::F10          => b"\x1b[21~",
                NamedKey::F11          => b"\x1b[23~",
                NamedKey::F12          => b"\x1b[24~",
                _                      => b"",
            };
            out.extend_from_slice(seq);
        }
        _ => {}
    }
    out
}
