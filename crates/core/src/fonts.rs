use crate::helvetica::win_ansi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Font {
    #[default]
    Helvetica,
    Times,
    Courier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Script {
    #[default]
    Normal,
    Sub,
    Super,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub script: Script,
}

pub const SCRIPT_SCALE: f32 = 0.65;
pub const SUPER_RISE: f32 = 0.35;
pub const SUB_RISE: f32 = -0.15;

impl Font {
    pub const ALL: [Font; 3] = [Font::Helvetica, Font::Times, Font::Courier];

    pub fn name(self) -> &'static str {
        match self {
            Self::Helvetica => "Helvetica",
            Self::Times => "Times",
            Self::Courier => "Courier",
        }
    }

    pub fn base_font(self, bold: bool, italic: bool) -> &'static str {
        match (self, bold, italic) {
            (Self::Helvetica, false, false) => "Helvetica",
            (Self::Helvetica, true, false) => "Helvetica-Bold",
            (Self::Helvetica, false, true) => "Helvetica-Oblique",
            (Self::Helvetica, true, true) => "Helvetica-BoldOblique",
            (Self::Times, false, false) => "Times-Roman",
            (Self::Times, true, false) => "Times-Bold",
            (Self::Times, false, true) => "Times-Italic",
            (Self::Times, true, true) => "Times-BoldItalic",
            (Self::Courier, false, false) => "Courier",
            (Self::Courier, true, false) => "Courier-Bold",
            (Self::Courier, false, true) => "Courier-Oblique",
            (Self::Courier, true, true) => "Courier-BoldOblique",
        }
    }

    pub fn resource(self, bold: bool, italic: bool) -> &'static str {
        match (self, bold, italic) {
            (Self::Helvetica, false, false) => "Helv",
            (Self::Helvetica, true, false) => "HeBo",
            (Self::Helvetica, false, true) => "HeOb",
            (Self::Helvetica, true, true) => "HeBO",
            (Self::Times, false, false) => "TiRo",
            (Self::Times, true, false) => "TiBo",
            (Self::Times, false, true) => "TiIt",
            (Self::Times, true, true) => "TiBI",
            (Self::Courier, false, false) => "Cour",
            (Self::Courier, true, false) => "CoBo",
            (Self::Courier, false, true) => "CoOb",
            (Self::Courier, true, true) => "CoBO",
        }
    }

    pub fn from_resource(name: &str) -> Option<(Font, bool, bool)> {
        Font::ALL.into_iter().find_map(|font| {
            [(false, false), (true, false), (false, true), (true, true)]
                .into_iter()
                .find(|&(bold, italic)| font.resource(bold, italic) == name)
                .map(|(bold, italic)| (font, bold, italic))
        })
    }

    fn widths(self, bold: bool, italic: bool) -> Option<&'static [u16; 224]> {
        match (self, bold, italic) {
            (Self::Helvetica, false, _) => Some(&crate::helvetica::WIDTHS),
            (Self::Helvetica, true, _) => Some(&HELVETICA_BOLD),
            (Self::Times, false, false) => Some(&TIMES_ROMAN),
            (Self::Times, true, false) => Some(&TIMES_BOLD),
            (Self::Times, false, true) => Some(&TIMES_ITALIC),
            (Self::Times, true, true) => Some(&TIMES_BOLD_ITALIC),
            (Self::Courier, _, _) => None,
        }
    }

    pub fn char_width(self, c: char, style: CharStyle, font_size: f32) -> f32 {
        let units = self.code_units(win_ansi(c), style.bold, style.italic);
        f32::from(units) / 1000.0 * style.size(font_size)
    }

    pub(crate) fn code_units(self, code: u8, bold: bool, italic: bool) -> u16 {
        match self.widths(bold, italic) {
            Some(table) => code
                .checked_sub(32)
                .and_then(|index| table.get(usize::from(index)))
                .copied()
                .unwrap_or(556),
            None => 600,
        }
    }

    pub(crate) fn standard(base_font: &str) -> Option<(Font, bool, bool)> {
        let name = base_font
            .split_once('+')
            .map_or(base_font, |(_, name)| name);
        let font = if name.contains("Courier") {
            Font::Courier
        } else if name.contains("Times") {
            Font::Times
        } else if name.contains("Helvetica") || name.contains("Arial") {
            Font::Helvetica
        } else {
            return None;
        };
        let bold = name.contains("Bold");
        let italic = name.contains("Italic") || name.contains("Oblique");
        Some((font, bold, italic))
    }
}

impl CharStyle {
    pub fn size(self, font_size: f32) -> f32 {
        match self.script {
            Script::Normal => font_size,
            Script::Sub | Script::Super => font_size * SCRIPT_SCALE,
        }
    }

    pub fn rise(self, font_size: f32) -> f32 {
        match self.script {
            Script::Normal => 0.0,
            Script::Sub => SUB_RISE * font_size,
            Script::Super => SUPER_RISE * font_size,
        }
    }
}

pub fn style_at(styles: &[CharStyle], index: usize) -> CharStyle {
    styles.get(index).copied().unwrap_or_default()
}

const HELVETICA_BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
    350, 556, 350, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 350, 611, 350, 350,
    278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 350, 500, 667, 278, 333, 556,
    556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 611,
    556, 278, 333, 333, 365, 556, 834, 834, 834, 611, 722, 722, 722, 722, 722, 722, 1000, 722, 667,
    667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722,
    722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278,
    278, 611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];

#[rustfmt::skip]
const TIMES_ROMAN: [u16; 224] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444,
    921, 722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722,
    556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469, 500,
    333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500,
    500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541, 350,
    500, 350, 333, 500, 444, 1000, 500, 500, 333, 1000, 556, 333, 889, 350, 611, 350,
    350, 333, 333, 444, 444, 350, 500, 1000, 333, 980, 389, 333, 722, 350, 444, 722,
    250, 333, 500, 500, 500, 500, 200, 500, 333, 760, 276, 500, 564, 333, 760, 333,
    400, 564, 300, 300, 333, 500, 453, 250, 333, 300, 310, 500, 750, 750, 750, 444,
    722, 722, 722, 722, 722, 722, 889, 667, 611, 611, 611, 611, 333, 333, 333, 333,
    722, 722, 722, 722, 722, 722, 722, 564, 722, 722, 722, 722, 722, 722, 556, 500,
    444, 444, 444, 444, 444, 444, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 500, 500, 500, 500, 500, 500, 564, 500, 500, 500, 500, 500, 500, 500, 500,
];

#[rustfmt::skip]
const TIMES_BOLD: [u16; 224] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500,
    930, 722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944, 722, 778,
    611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581, 500,
    333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500,
    556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520, 350,
    500, 350, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 1000, 350, 667, 350,
    350, 333, 333, 500, 500, 350, 500, 1000, 333, 1000, 389, 333, 722, 350, 444, 722,
    250, 333, 500, 500, 500, 500, 220, 500, 333, 747, 300, 500, 570, 333, 747, 333,
    400, 570, 300, 300, 333, 556, 540, 250, 333, 300, 330, 500, 750, 750, 750, 500,
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 389, 389, 389, 389,
    722, 722, 778, 778, 778, 778, 778, 570, 778, 722, 722, 722, 722, 722, 611, 556,
    500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 556, 500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 500, 556, 500,
];

#[rustfmt::skip]
const TIMES_ITALIC: [u16; 224] = [
    250, 333, 420, 500, 500, 833, 778, 214, 333, 333, 500, 675, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 675, 675, 675, 500,
    920, 611, 611, 667, 722, 611, 611, 722, 722, 333, 444, 667, 556, 833, 667, 722,
    611, 722, 611, 500, 556, 722, 611, 833, 611, 556, 556, 389, 278, 389, 422, 500,
    333, 500, 500, 444, 500, 444, 278, 500, 500, 278, 278, 444, 278, 722, 500, 500,
    500, 500, 389, 389, 278, 500, 444, 667, 444, 444, 389, 400, 275, 400, 541, 350,
    500, 350, 333, 500, 556, 889, 500, 500, 333, 1000, 500, 333, 944, 350, 556, 350,
    350, 333, 333, 556, 556, 350, 500, 889, 333, 980, 389, 333, 667, 350, 389, 556,
    250, 389, 500, 500, 500, 500, 275, 500, 333, 760, 276, 500, 675, 333, 760, 333,
    400, 675, 300, 300, 333, 500, 523, 250, 333, 300, 310, 500, 750, 750, 750, 500,
    611, 611, 611, 611, 611, 611, 889, 667, 611, 611, 611, 611, 333, 333, 333, 333,
    722, 667, 722, 722, 722, 722, 722, 675, 722, 722, 722, 722, 722, 556, 611, 500,
    500, 500, 500, 500, 500, 500, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 500, 500, 500, 500, 500, 500, 675, 500, 500, 500, 500, 500, 444, 500, 444,
];

#[rustfmt::skip]
const TIMES_BOLD_ITALIC: [u16; 224] = [
    250, 389, 555, 500, 500, 833, 778, 278, 333, 333, 500, 570, 250, 333, 250, 278,
    500, 500, 500, 500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500,
    832, 667, 667, 667, 722, 667, 667, 722, 778, 389, 500, 667, 611, 889, 722, 722,
    611, 722, 667, 556, 611, 722, 667, 889, 667, 611, 611, 333, 278, 333, 570, 500,
    333, 500, 500, 444, 500, 444, 333, 500, 556, 278, 278, 500, 278, 778, 556, 500,
    500, 500, 389, 389, 278, 556, 444, 667, 500, 444, 389, 348, 220, 348, 570, 350,
    500, 350, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 944, 350, 611, 350,
    350, 333, 333, 500, 500, 350, 500, 1000, 333, 1000, 389, 333, 722, 350, 389, 611,
    250, 389, 500, 500, 500, 500, 220, 500, 333, 747, 266, 500, 606, 333, 747, 333,
    400, 570, 300, 300, 333, 576, 500, 250, 333, 300, 300, 500, 750, 750, 750, 500,
    667, 667, 667, 667, 667, 667, 944, 667, 667, 667, 667, 667, 389, 389, 389, 389,
    722, 722, 722, 722, 722, 722, 722, 570, 722, 722, 722, 722, 722, 611, 611, 500,
    500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 556, 500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 444, 500, 444,
];
