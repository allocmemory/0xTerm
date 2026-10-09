//! How a terminal looks: font, cell metrics, colors. Built from the config.

use gpui_kit::*;
use oxt_config::{Colors, Config};
use oxt_session::Palette;
use oxt_session::alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

#[derive(Clone, Debug, PartialEq)]
pub struct TerminalStyle {
    pub font_family: SharedString,
    pub font_size: Pixels,
    pub line_height: f32,
    pub palette: Palette,
    pub selection: Rgb,
    pub padding: Pixels,
}

impl TerminalStyle {
    pub fn from_config(config: &Config, dark: bool) -> Self {
        let colors = if dark { &config.theme.dark } else { &config.theme.light };
        TerminalStyle {
            font_family: config.font.family.clone().into(),
            font_size: px(config.font.size),
            line_height: config.font.line_height,
            palette: palette(colors),
            selection: rgb_of(colors.selection),
            padding: px(8.),
        }
    }

    pub fn font(&self) -> Font {
        font(self.font_family.clone())
    }

    /// Resolves a cell color. Colors a program changed (OSC 4/10/11) win over the theme.
    pub fn color(&self, color: Color, overrides: &oxt_session::alacritty_terminal::term::color::Colors) -> Rgb {
        let index = match color {
            Color::Spec(rgb) => return rgb,
            Color::Indexed(i) => i as usize,
            Color::Named(named) => named as usize,
        };
        if let Some(rgb) = overrides[index] {
            return rgb;
        }
        let p = &self.palette;
        match color {
            Color::Named(named) => match named {
                NamedColor::Foreground | NamedColor::BrightForeground => p.foreground,
                NamedColor::Background => p.background,
                NamedColor::Cursor => p.cursor,
                NamedColor::DimForeground => dim(p.foreground),
                n if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&(n as usize)) => {
                    dim(p.ansi[n as usize - NamedColor::DimBlack as usize])
                }
                n => p.rgb(n as usize),
            },
            _ => p.rgb(index),
        }
    }
}

pub fn hsla(rgb: Rgb) -> Hsla {
    Rgba { r: rgb.r as f32 / 255., g: rgb.g as f32 / 255., b: rgb.b as f32 / 255., a: 1. }.into()
}

pub fn dim(rgb: Rgb) -> Rgb {
    Rgb { r: (rgb.r as f32 * 0.66) as u8, g: (rgb.g as f32 * 0.66) as u8, b: (rgb.b as f32 * 0.66) as u8 }
}

fn rgb_of(hex: oxt_config::Hex) -> Rgb {
    let (r, g, b) = hex.rgb();
    Rgb { r, g, b }
}

pub fn palette(colors: &Colors) -> Palette {
    Palette {
        ansi: colors.ansi.map(rgb_of),
        foreground: rgb_of(colors.foreground),
        background: rgb_of(colors.background),
        cursor: rgb_of(colors.cursor),
    }
}

/// The size of one cell, measured from the font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellSize {
    pub width: Pixels,
    pub height: Pixels,
}

impl CellSize {
    pub fn measure(style: &TerminalStyle, window: &Window) -> Self {
        let text = window.text_system();
        let font_id = text.resolve_font(&style.font());
        let width = text.advance(font_id, style.font_size, 'm').map(|s| s.width).unwrap_or(style.font_size * 0.6);
        CellSize { width, height: (style.font_size * style.line_height).round() }
    }
}
