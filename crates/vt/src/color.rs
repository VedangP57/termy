/// Terminal color value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    /// One of the 8 standard + 8 bright ANSI colors (index 0-15).
    Indexed(u8),
    /// xterm 256-color palette (index 0-255).
    Palette(u8),
    /// 24-bit truecolor.
    Rgb(u8, u8, u8),
}

/// The 8 standard ANSI named colors, used to map SGR 30-37 / 40-47.
pub const ANSI_NAMED: [Color; 8] = [
    Color::Indexed(0),
    Color::Indexed(1),
    Color::Indexed(2),
    Color::Indexed(3),
    Color::Indexed(4),
    Color::Indexed(5),
    Color::Indexed(6),
    Color::Indexed(7),
];

/// Bright variants of the standard ANSI colors, SGR 90-97 / 100-107.
pub const ANSI_BRIGHT: [Color; 8] = [
    Color::Indexed(8),
    Color::Indexed(9),
    Color::Indexed(10),
    Color::Indexed(11),
    Color::Indexed(12),
    Color::Indexed(13),
    Color::Indexed(14),
    Color::Indexed(15),
];
