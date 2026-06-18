use vt::Color;

/// Standard 16-colour ANSI palette as 0x00RRGGBB.
const ANSI16: [u32; 16] = [
    0x000000, 0xcc0000, 0x4e9a06, 0xc4a000,
    0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf,
    0x555753, 0xef2929, 0x8ae234, 0xfce94f,
    0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
];

const FG_DEFAULT: u32 = 0xd3d7cf; // light gray
const BG_DEFAULT: u32 = 0x000000; // black

/// Map a terminal colour to a packed `0x00RRGGBB` value for the softbuffer.
/// `is_fg` selects the default colour when `Color::Default` is passed.
pub fn to_argb(color: Color, is_fg: bool) -> u32 {
    match color {
        Color::Default => if is_fg { FG_DEFAULT } else { BG_DEFAULT },
        Color::Indexed(n) if (n as usize) < 16 => ANSI16[n as usize],
        Color::Indexed(n) | Color::Palette(n) => palette256(n),
        Color::Rgb(r, g, b) => rgb(r, g, b),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// xterm 256-colour palette → 0x00RRGGBB.
fn palette256(n: u8) -> u32 {
    if (n as usize) < 16 { return ANSI16[n as usize]; }
    if n >= 232 {
        let v = (n - 232) * 10 + 8;
        return rgb(v, v, v);
    }
    let n = n - 16;
    let b = n % 6;
    let g = (n / 6) % 6;
    let r = n / 36;
    let e = |c: u8| -> u8 { if c == 0 { 0 } else { c * 40 + 55 } };
    rgb(e(r), e(g), e(b))
}
