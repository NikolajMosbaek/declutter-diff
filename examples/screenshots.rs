//! Renders the README screenshots: `cargo run --example screenshots`.
//!
//! Draws the viewer on a demo change — an AI-style edit to a small cart module — into
//! an in-memory terminal and writes each frame as an SVG to `docs/images/`, so the
//! pictures are reproducible and always match the current UI.

use std::fmt::Write;
use std::fs;

use declutter::project::LayerMode;
use declutter::review::{ChangeStatus, FileChange, FileReview, Layers};
use declutter::store::{Note, NoteSide};
use declutter::tui::{App, Focus, draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier};

const WIDTH: u16 = 140;
const HEIGHT: u16 = 30;

const OLD_CART: &str = "import { Item } from './item';

export function total(items: Item[]): number {
  return items.reduce((sum, item) => sum + item.price, 0);
}
";

const NEW_CART: &str = "import { Item } from './item';
import { logger } from './logger';

/**
 * Calculates the total price of all items in the cart.
 *
 * @param items - The items currently in the cart.
 * @param discount - An optional discount between 0 and 1.
 * @returns The total price after the discount is applied.
 */
export function total(items: Item[], discount = 0): number {
  // Validate the discount so we never return a negative total.
  if (discount < 0 || discount > 1) {
    // Log the invalid value to help with debugging.
    logger.warn('Invalid discount', { discount });
    throw new RangeError('discount must be between 0 and 1');
  }

  // Sum up the prices of every item in the cart.
  const subtotal = items.reduce((sum, item) => sum + item.price, 0);

  // Apply the discount to the subtotal and return the result.
  return subtotal * (1 - discount);
}
";

const NEW_TEST: &str = "import { describe, expect, it } from 'vitest';
import { total } from './cart';

describe('total', () => {
  // A discount of 10% should reduce the total accordingly.
  it('applies a discount', () => {
    expect(total([{ price: 10 }, { price: 30 }], 0.1)).toBe(36);
  });

  it('rejects a discount above 1', () => {
    expect(() => total([], 1.5)).toThrow(RangeError);
  });
});
";

const OLD_FORMAT: &str = "export function price(cents: number): string {
  return '$' + (cents/100).toFixed(2);
}
";

const NEW_FORMAT: &str = "/** Formats a price in cents as dollars, e.g. 1999 → \"$19.99\". */
export function price(cents: number): string {
  return '$' + (cents / 100).toFixed(2);
}
";

fn files() -> Vec<FileReview> {
    let change = |path: &str, status, old: Option<&str>, new: &str| {
        FileReview::new(FileChange {
            path: path.to_string(),
            old_path: None,
            status,
            old: old.map(str::to_string),
            new: Some(new.to_string()),
            binary: false,
        })
    };
    vec![
        change(
            "src/cart.ts",
            ChangeStatus::Modified,
            Some(OLD_CART),
            NEW_CART,
        ),
        change("src/cart.test.ts", ChangeStatus::Added, None, NEW_TEST),
        change(
            "src/format.ts",
            ChangeStatus::Modified,
            Some(OLD_FORMAT),
            NEW_FORMAT,
        ),
    ]
}

fn app(layers: Layers) -> App {
    let mut app = App::new(files(), layers);
    app.title = "origin/main...feature/cart-discount".to_string();
    app
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::from(code));
}

fn main() -> std::io::Result<()> {
    fs::create_dir_all("docs/images")?;

    // Everything shown: what a plain diff looks like.
    let all = Layers {
        comments: LayerMode::Shown,
        ..Layers::default()
    };
    save("docs/images/everything.svg", &mut app(all))?;

    // Comments and logging hidden, cursor in the diff, with a review note.
    let mut decluttered = app(Layers {
        logging: LayerMode::Hidden,
        ..Layers::default()
    });
    decluttered.focus = Focus::Diff;
    decluttered
        .notes
        .set(Note {
            path: "src/cart.ts".to_string(),
            side: NoteSide::New,
            line: 13,
            code: "  if (discount < 0 || discount > 1) {".to_string(),
            text: "Should a 100% discount be allowed?".to_string(),
            review: None,
        })
        .expect("in-memory note");
    // From the first change down to the noted `if`.
    for _ in 0..5 {
        press(&mut decluttered, KeyCode::Down);
    }
    save("docs/images/decluttered.svg", &mut decluttered)?;

    // Only the comments: reviewing what the AI wrote about its code.
    save(
        "docs/images/comments-only.svg",
        &mut app(Layers {
            comments: LayerMode::Only,
            ..Layers::default()
        }),
    )?;

    // The key help.
    let mut help = app(Layers::default());
    press(&mut help, KeyCode::Char('?'));
    save("docs/images/keys.svg", &mut help)?;
    Ok(())
}

fn save(path: &str, app: &mut App) -> std::io::Result<()> {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test terminal");
    terminal.draw(|frame| draw(frame, app)).expect("draw");
    fs::write(path, svg(terminal.backend().buffer()))?;
    println!("wrote {path}");
    Ok(())
}

// A dark theme for the terminal's own colours.
const BACKGROUND: &str = "#1b1d23";
const FOREGROUND: &str = "#d5d8de";
const CELL_WIDTH: f32 = 8.4;
const CELL_HEIGHT: f32 = 18.0;
const PADDING: f32 = 14.0;

/// The buffer as an SVG: a background rectangle and a text run per stretch of cells
/// with the same style, each placed on the cell grid so fonts cannot drift.
fn svg(buffer: &Buffer) -> String {
    let (columns, rows) = (buffer.area.width as usize, buffer.area.height as usize);
    let width = columns as f32 * CELL_WIDTH + 2.0 * PADDING;
    let height = rows as f32 * CELL_HEIGHT + 2.0 * PADDING;
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" font-family="'SF Mono', Menlo, Consolas, 'DejaVu Sans Mono', monospace" font-size="14">"#
    );
    let _ = write!(
        out,
        r#"<rect width="{width}" height="{height}" rx="8" fill="{BACKGROUND}"/>"#
    );
    for (y, row) in buffer.content().chunks(columns).enumerate() {
        let mut x = 0;
        while x < row.len() {
            let style = Look::of(&row[x]);
            let mut end = x + 1;
            while end < row.len() && Look::of(&row[end]) == style {
                end += 1;
            }
            let left = PADDING + x as f32 * CELL_WIDTH;
            let top = PADDING + y as f32 * CELL_HEIGHT;
            if let Some(bg) = &style.bg {
                let _ = write!(
                    out,
                    r#"<rect x="{left}" y="{top}" width="{}" height="{CELL_HEIGHT}" fill="{bg}"/>"#,
                    (end - x) as f32 * CELL_WIDTH
                );
            }
            for (i, cell) in row[x..end].iter().enumerate() {
                let symbol = cell.symbol();
                if symbol.trim().is_empty() {
                    continue;
                }
                let _ = write!(
                    out,
                    r#"<text x="{}" y="{}" fill="{}"{}{}{}>{}</text>"#,
                    left + i as f32 * CELL_WIDTH,
                    top + CELL_HEIGHT * 0.75,
                    style.fg,
                    if style.bold {
                        r#" font-weight="bold""#
                    } else {
                        ""
                    },
                    if style.italic {
                        r#" font-style="italic""#
                    } else {
                        ""
                    },
                    if style.dim { r#" opacity="0.55""# } else { "" },
                    escape(symbol)
                );
            }
            x = end;
        }
    }
    out.push_str("</svg>\n");
    out
}

#[derive(PartialEq)]
struct Look {
    fg: String,
    bg: Option<String>,
    bold: bool,
    italic: bool,
    dim: bool,
}

impl Look {
    fn of(cell: &Cell) -> Look {
        let mut fg = hex(cell.fg).unwrap_or_else(|| FOREGROUND.to_string());
        let mut bg = hex(cell.bg);
        if cell.modifier.contains(Modifier::REVERSED) {
            let swapped = bg.unwrap_or_else(|| BACKGROUND.to_string());
            bg = Some(fg);
            fg = swapped;
        }
        Look {
            fg,
            bg,
            bold: cell.modifier.contains(Modifier::BOLD),
            italic: cell.modifier.contains(Modifier::ITALIC),
            dim: cell.modifier.contains(Modifier::DIM),
        }
    }
}

/// A colour as hex; `None` for the terminal's default.
fn hex(color: Color) -> Option<String> {
    let rgb = |r: u8, g: u8, b: u8| Some(format!("#{r:02x}{g:02x}{b:02x}"));
    match color {
        Color::Reset => None,
        Color::Rgb(r, g, b) => rgb(r, g, b),
        Color::Black => rgb(0x1b, 0x1d, 0x23),
        Color::Red => rgb(0xe0, 0x6c, 0x75),
        Color::Green => rgb(0x98, 0xc3, 0x79),
        Color::Yellow => rgb(0xe5, 0xc0, 0x7b),
        Color::Blue => rgb(0x61, 0xaf, 0xef),
        Color::Magenta => rgb(0xc6, 0x78, 0xdd),
        Color::Cyan => rgb(0x56, 0xb6, 0xc2),
        Color::Gray => rgb(0xab, 0xb2, 0xbf),
        Color::DarkGray => rgb(0x5c, 0x63, 0x70),
        Color::LightRed => rgb(0xff, 0x7b, 0x86),
        Color::LightGreen => rgb(0xb5, 0xe8, 0x90),
        Color::LightYellow => rgb(0xff, 0xd8, 0x8a),
        Color::LightBlue => rgb(0x82, 0xc4, 0xff),
        Color::LightMagenta => rgb(0xe0, 0x9a, 0xf5),
        Color::LightCyan => rgb(0x7d, 0xd6, 0xe0),
        Color::White => rgb(0xff, 0xff, 0xff),
        Color::Indexed(i) => {
            let (r, g, b) = xterm(i);
            rgb(r, g, b)
        }
    }
}

/// The standard xterm 256-colour table, past the 16 theme colours.
fn xterm(index: u8) -> (u8, u8, u8) {
    match index {
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            (v, v, v)
        }
        _ => (0xab, 0xb2, 0xbf),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
