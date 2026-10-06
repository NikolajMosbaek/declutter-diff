use ratatui::style::{Color, Modifier, Style};

use crate::highlight::Class;

/// The viewer's colours, tuned for a dark terminal: a quiet tint behind changed lines,
/// a stronger one behind the words that changed, and only four syntax colours — none of
/// them loud — so the diff, not the syntax, is what stands out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub removed: Style,
    pub added: Style,
    pub removed_emphasis: Style,
    pub added_emphasis: Style,
    pub moved_away: Style,
    pub moved_here: Style,
    pub removed_sign: Color,
    pub added_sign: Color,
    pub moved_sign: Color,
    pub keyword: Color,
    pub string: Color,
    pub comment: Color,
    pub number: Color,
    pub search: Style,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Palette {
    /// For terminals with 24-bit colour, which allows tints dark enough to sit quietly
    /// behind syntax colours.
    pub const TRUE_COLOR: Palette = Palette {
        removed: Style::new().bg(rgb(0x2d1b1f)),
        added: Style::new().bg(rgb(0x1a2b20)),
        removed_emphasis: Style::new().bg(rgb(0x5c2b33)),
        added_emphasis: Style::new().bg(rgb(0x2b5237)),
        moved_away: Style::new().bg(rgb(0x262033)),
        moved_here: Style::new().bg(rgb(0x1b2a33)),
        removed_sign: rgb(0xe0707a),
        added_sign: rgb(0x7fc48a),
        moved_sign: rgb(0x8fb3d9),
        keyword: rgb(0xc2a8dc),
        string: rgb(0xd2b98c),
        comment: rgb(0x8c939e),
        number: rgb(0xd9a07c),
        search: Style::new().fg(Color::Black).bg(rgb(0xd7b84a)),
    };

    /// For 256-colour terminals. Their darkest reds and greens are too strong to fill a
    /// whole line, so changed lines get no tint — removed ones are dimmed instead — and
    /// only the changed words get a background.
    pub const COLOR_256: Palette = Palette {
        removed: Style::new().add_modifier(Modifier::DIM),
        added: Style::new(),
        removed_emphasis: Style::new().bg(Color::Indexed(52)),
        added_emphasis: Style::new().bg(Color::Indexed(22)),
        moved_away: Style::new().add_modifier(Modifier::DIM),
        moved_here: Style::new(),
        removed_sign: Color::Indexed(167),
        added_sign: Color::Indexed(114),
        moved_sign: Color::Indexed(110),
        keyword: Color::Indexed(140),
        string: Color::Indexed(180),
        comment: Color::Indexed(245),
        number: Color::Indexed(173),
        search: Style::new().fg(Color::Black).bg(Color::Indexed(179)),
    };

    /// True colour when the terminal says it has it (`COLORTERM`), 256 colours otherwise.
    pub fn detect() -> Palette {
        match std::env::var("COLORTERM").as_deref() {
            Ok("truecolor" | "24bit") => Palette::TRUE_COLOR,
            _ => Palette::COLOR_256,
        }
    }

    /// The colour of a syntax class, if it gets one: types, functions and attributes are
    /// left in the terminal's own text colour.
    pub fn syntax(&self, class: Class) -> Option<Style> {
        let fg = match class {
            Class::Keyword => self.keyword,
            Class::String => self.string,
            Class::Number | Class::Constant => self.number,
            Class::Comment => {
                return Some(Style::new().fg(self.comment).add_modifier(Modifier::ITALIC));
            }
            Class::Type | Class::Function | Class::Attribute => return None,
        };
        Some(Style::new().fg(fg))
    }
}
