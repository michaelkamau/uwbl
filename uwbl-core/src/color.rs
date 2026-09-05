//! Colour model: sRGB `u8` triplets on the API side, EC intensity levels `0..=50` on the wire.
//!
//! The EC treats `0,0,0` as "restore firmware default colour", so a fully black colour is never
//! sent; the minimum per-channel intensity on the wire is `1` (the driver enforces the same).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, MAX_INTENSITY};

/// An 8-bit-per-channel sRGB colour. Serialises as `"#rrggbb"` (or a preset name on input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Convert to EC intensity levels (`0..=50`), never returning `0,0,0`.
    pub fn to_intensity(self) -> [u8; 3] {
        let scale = |v: u8| ((v as u32 * MAX_INTENSITY as u32 + 127) / 255) as u8;
        let mut out = [scale(self.r), scale(self.g), scale(self.b)];
        if out == [0, 0, 0] {
            out = [1, 1, 1];
        }
        out
    }

    /// Convert from EC intensity levels back to sRGB (lossy).
    pub fn from_intensity(v: [u8; 3]) -> Self {
        let scale = |x: u8| ((x.min(MAX_INTENSITY) as u32 * 255 + 25) / MAX_INTENSITY as u32) as u8;
        Self::new(scale(v[0]), scale(v[1]), scale(v[2]))
    }

    /// Hue (0..360), saturation and value (0..1).
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.rem_euclid(360.0) / 60.0;
        let i = h.floor() as i32;
        let f = h - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        let (r, g, b) = match i {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::new(c(r), c(g), c(b))
    }

    /// Scale all channels by `factor` (0..1).
    pub fn scaled(self, factor: f32) -> Self {
        let f = factor.clamp(0.0, 1.0);
        let s = |v: u8| (v as f32 * f).round() as u8;
        Self::new(s(self.r), s(self.g), s(self.b))
    }

    /// Linear interpolation between two colours.
    pub fn lerp(self, other: Rgb, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let l = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self::new(l(self.r, other.r), l(self.g, other.g), l(self.b, other.b))
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match preset_name(*self) {
            Some(n) => write!(f, "{} ({})", n, self.to_hex()),
            None => f.write_str(&self.to_hex()),
        }
    }
}

/// Parse `#rrggbb`, `rrggbb`, `r,g,b` or a preset name.
impl FromStr for Rgb {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let s = s.trim();
        if let Some(p) = preset(s) {
            return Ok(p);
        }
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = u32::from_str_radix(hex, 16).unwrap();
            return Ok(Self::new((v >> 16) as u8, (v >> 8) as u8, v as u8));
        }
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        if parts.len() == 3 {
            let ch = |p: &str| p.parse::<u8>().ok();
            if let (Some(r), Some(g), Some(b)) = (ch(parts[0]), ch(parts[1]), ch(parts[2])) {
                return Ok(Self::new(r, g, b));
            }
        }
        Err(Error::Invalid(format!(
            "cannot parse colour {s:?}; use #rrggbb, r,g,b or one of: {}",
            PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
        )))
    }
}

/// Named colours. The first seven mirror the Windows Control Center defaults.
pub const PRESETS: &[(&str, Rgb)] = &[
    ("red", Rgb::new(255, 0, 0)),
    ("orange", Rgb::new(255, 165, 0)),
    ("yellow", Rgb::new(255, 255, 0)),
    ("green", Rgb::new(0, 255, 0)),
    ("blue", Rgb::new(0, 0, 255)),
    ("cyan", Rgb::new(0, 255, 255)),
    ("violet", Rgb::new(139, 0, 255)),
    ("white", Rgb::new(255, 255, 255)),
    ("magenta", Rgb::new(255, 0, 255)),
    ("pink", Rgb::new(255, 105, 180)),
    ("purple", Rgb::new(128, 0, 128)),
    ("teal", Rgb::new(0, 128, 128)),
    ("lime", Rgb::new(128, 255, 0)),
    ("gold", Rgb::new(255, 215, 0)),
    ("warm-white", Rgb::new(255, 214, 170)),
    ("cool-white", Rgb::new(200, 220, 255)),
];

pub fn preset(name: &str) -> Option<Rgb> {
    let n = name.to_ascii_lowercase();
    PRESETS.iter().find(|(p, _)| *p == n).map(|(_, c)| *c)
}

pub fn preset_name(c: Rgb) -> Option<&'static str> {
    PRESETS.iter().find(|(_, p)| *p == c).map(|(n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intensity_roundtrip_never_black() {
        assert_eq!(Rgb::new(0, 0, 0).to_intensity(), [1, 1, 1]);
        assert_eq!(Rgb::new(255, 255, 255).to_intensity(), [50, 50, 50]);
        assert_eq!(Rgb::new(255, 0, 0).to_intensity(), [50, 0, 0]);
        let back = Rgb::from_intensity([50, 25, 0]);
        assert_eq!(back, Rgb::new(255, 128, 0));
    }

    #[test]
    fn parse_forms() {
        assert_eq!("#ff8000".parse::<Rgb>().unwrap(), Rgb::new(255, 128, 0));
        assert_eq!("FF8000".parse::<Rgb>().unwrap(), Rgb::new(255, 128, 0));
        assert_eq!("1, 2 ,3".parse::<Rgb>().unwrap(), Rgb::new(1, 2, 3));
        assert_eq!("Blue".parse::<Rgb>().unwrap(), Rgb::new(0, 0, 255));
        assert!("nope".parse::<Rgb>().is_err());
    }

    #[test]
    fn hsv_primaries() {
        assert_eq!(Rgb::from_hsv(0.0, 1.0, 1.0), Rgb::new(255, 0, 0));
        assert_eq!(Rgb::from_hsv(120.0, 1.0, 1.0), Rgb::new(0, 255, 0));
        assert_eq!(Rgb::from_hsv(240.0, 1.0, 1.0), Rgb::new(0, 0, 255));
        assert_eq!(Rgb::from_hsv(360.0, 1.0, 1.0), Rgb::new(255, 0, 0));
    }

    #[test]
    fn serde_hex() {
        #[derive(Serialize, Deserialize)]
        struct W { c: Rgb }
        let t = toml::to_string(&W { c: Rgb::new(255, 0, 16) }).unwrap();
        assert_eq!(t.trim(), r##"c = "#ff0010""##);
        let w: W = toml::from_str(r#"c = "blue""#).unwrap();
        assert_eq!(w.c, Rgb::new(0, 0, 255));
    }

    #[test]
    fn display_uses_preset_name() {
        assert_eq!(Rgb::new(255, 0, 0).to_string(), "red (#ff0000)");
        assert_eq!(Rgb::new(1, 2, 3).to_string(), "#010203");
    }
}
