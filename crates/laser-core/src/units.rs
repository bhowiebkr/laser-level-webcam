//! Display units. Heights are kept in µm everywhere; a unit only changes how they are shown.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    #[default]
    Um,
    Mm,
    Inch,
}

impl Unit {
    pub const ALL: [Unit; 3] = [Unit::Um, Unit::Mm, Unit::Inch];

    /// Value in this unit of a height in µm.
    pub fn from_um(self, um: f64) -> f64 {
        match self {
            Self::Um => um,
            Self::Mm => um / 1000.0,
            Self::Inch => um / 25_400.0,
        }
    }

    /// Height in µm of a value in this unit.
    pub fn to_um(self, value: f64) -> f64 {
        match self {
            Self::Um => value,
            Self::Mm => value * 1000.0,
            Self::Inch => value * 25_400.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Um => "µm",
            Self::Mm => "mm",
            Self::Inch => "in",
        }
    }

    /// Label with ASCII letters only, for CSV headers.
    pub fn ascii(self) -> &'static str {
        match self {
            Self::Um => "um",
            Self::Mm => "mm",
            Self::Inch => "in",
        }
    }

    /// Decimals that show 0.01 µm or a little better.
    pub fn decimals(self) -> usize {
        match self {
            Self::Um => 2,
            Self::Mm => 5,
            Self::Inch => 6,
        }
    }

    /// A height in µm as text in this unit, without the label.
    pub fn format(self, um: f64) -> String {
        fixed(self.from_um(um), self.decimals())
    }
}

/// `v` with `decimals` decimals. A value that rounds to zero has no minus sign.
pub fn fixed(v: f64, decimals: usize) -> String {
    let text = format!("{v:.decimals$}");
    match text.strip_prefix('-') {
        Some(rest) if rest.bytes().all(|b| b == b'0' || b == b'.') => rest.to_owned(),
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_text() {
        for u in Unit::ALL {
            assert!((u.to_um(u.from_um(12.7)) - 12.7).abs() < 1e-12);
        }
        assert_eq!(Unit::Inch.format(25.4), "0.001000");
        assert_eq!(Unit::Mm.format(-1.5), "-0.00150");
        assert_eq!(Unit::Mm.format(-0.004), "0.00000");
        assert_eq!(fixed(-0.0, 0), "0");
        assert_eq!(fixed(-0.05, 1), "-0.1");
        assert_eq!(Unit::Um.format(4.56789), "4.57");
    }
}
