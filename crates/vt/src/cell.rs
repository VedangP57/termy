use crate::color::Color;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attrs {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

impl Default for Attrs {
    fn default() -> Self {
        Self {
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            italic: false,
            underline: false,
            inverse: false,
        }
    }
}

/// A single terminal cell: a Unicode scalar + its visual attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            attrs: Attrs::default(),
        }
    }
}

impl Cell {
    pub fn blank() -> Self {
        Self::default()
    }

    pub fn blank_with(attrs: Attrs) -> Self {
        Self { ch: ' ', attrs }
    }
}
