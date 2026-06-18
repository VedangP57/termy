use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Translate a winit key-press event into the byte sequence the PTY expects.
pub fn to_bytes(event: &KeyEvent, mods: ModifiersState) -> Vec<u8> {
    let mut out = Vec::new();
    let ctrl = mods.control_key();

    match &event.logical_key {
        Key::Character(s) => {
            if ctrl {
                if let Some(ch) = s.chars().next() {
                    let lo = ch.to_ascii_lowercase();
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
            let seq: &[u8] = match name {
                NamedKey::Enter        => b"\r",
                NamedKey::Backspace    => b"\x7f",
                NamedKey::Delete       => b"\x1b[3~",
                NamedKey::Tab         => b"\t",
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
                NamedKey::F6           => b"\x1b[17~",
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
