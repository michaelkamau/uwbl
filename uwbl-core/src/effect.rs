//! Software lighting effects.
//!
//! The EC only offers a static colour (plus a firmware "rainbow" flag that the kernel driver does
//! not expose for the keyboard), so like the Windows Control Center we animate in software by
//! rewriting `multi_intensity`. Each write costs ~5 slow EC transactions, so the engine coalesces
//! identical frames and targets ≤15 frames/s.
//!
//! The engine is pure with respect to time: [`EffectEngine::frame_at`] maps an elapsed
//! [`Duration`] to a frame, which makes it deterministic and unit-testable.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::color::{Rgb, PRESETS};
use crate::{Backend, Error, Result};

/// Frame period for animated effects.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(66);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EffectKind {
    /// Fixed colour (Control Center "Monochrome").
    #[default]
    Static,
    /// Fade the colour in and out.
    Breathing,
    /// Step through a list of colours (Control Center "Manual").
    Cycle,
    /// Continuous hue sweep.
    Rainbow,
}

impl EffectKind {
    pub const ALL: [EffectKind; 4] = [
        EffectKind::Static,
        EffectKind::Breathing,
        EffectKind::Cycle,
        EffectKind::Rainbow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Static => "static",
            EffectKind::Breathing => "breathing",
            EffectKind::Cycle => "cycle",
            EffectKind::Rainbow => "rainbow",
        }
    }

    pub fn is_animated(self) -> bool {
        self != EffectKind::Static
    }
}

impl fmt::Display for EffectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for EffectKind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "static" | "monochrome" | "solid" => Ok(EffectKind::Static),
            "breathing" | "breathe" | "pulse" => Ok(EffectKind::Breathing),
            "cycle" | "manual" => Ok(EffectKind::Cycle),
            "rainbow" | "spectrum" => Ok(EffectKind::Rainbow),
            other => Err(Error::Invalid(format!(
                "unknown effect {other:?}; expected static|breathing|cycle|rainbow"
            ))),
        }
    }
}

/// Complete description of what the keyboard should show while on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub kind: EffectKind,
    /// Base colour for static / breathing.
    pub color: Rgb,
    /// 1 (slow) ..= 10 (fast).
    pub speed: u8,
    /// Colours for [`EffectKind::Cycle`]; defaults to the seven Control Center presets.
    #[serde(default = "default_cycle_colors")]
    pub colors: Vec<Rgb>,
}

pub fn default_cycle_colors() -> Vec<Rgb> {
    PRESETS.iter().take(7).map(|(_, c)| *c).collect()
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            kind: EffectKind::Static,
            color: Rgb::new(255, 255, 255),
            speed: 5,
            colors: default_cycle_colors(),
        }
    }
}

impl Effect {
    pub fn speed_clamped(&self) -> u8 {
        self.speed.clamp(1, 10)
    }

    /// Period of one animation cycle for the configured speed.
    pub fn period(&self) -> Duration {
        let speed = self.speed_clamped() as f32;
        let secs = match self.kind {
            EffectKind::Static => 0.0,
            EffectKind::Breathing => 8.0 / speed,
            EffectKind::Cycle => (12.0 / speed) * self.cycle_colors().len().max(1) as f32,
            EffectKind::Rainbow => 24.0 / speed,
        };
        Duration::from_secs_f32(secs.max(0.0))
    }

    fn cycle_colors(&self) -> &[Rgb] {
        if self.colors.is_empty() {
            &[]
        } else {
            &self.colors
        }
    }
}

/// Drives a [`Backend`] with an [`Effect`], coalescing unchanged frames.
pub struct EffectEngine {
    effect: Effect,
    cycle_colors: Vec<Rgb>,
    last_frame: Option<[u8; 3]>,
}

impl EffectEngine {
    pub fn new(effect: Effect) -> Self {
        let cycle_colors = if effect.colors.is_empty() {
            default_cycle_colors()
        } else {
            effect.colors.clone()
        };
        Self {
            effect,
            cycle_colors,
            last_frame: None,
        }
    }

    pub fn effect(&self) -> &Effect {
        &self.effect
    }

    pub fn is_animated(&self) -> bool {
        self.effect.kind.is_animated()
    }

    /// How long the caller should wait before the next [`apply`](Self::apply).
    pub fn frame_interval(&self) -> Option<Duration> {
        self.is_animated().then_some(FRAME_INTERVAL)
    }

    /// Colour for the given elapsed time since the effect started.
    pub fn color_at(&self, t: Duration) -> Rgb {
        let period = self.effect.period().as_secs_f32();
        let phase = if period > 0.0 {
            (t.as_secs_f32() % period) / period
        } else {
            0.0
        };
        match self.effect.kind {
            EffectKind::Static => self.effect.color,
            EffectKind::Breathing => {
                // Cosine breathing between 8 % and 100 % so the keys never go fully dark
                // (0,0,0 would reset the EC colour).
                let wave = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
                self.effect.color.scaled(0.08 + 0.92 * wave)
            }
            EffectKind::Cycle => {
                let n = self.cycle_colors.len();
                let pos = phase * n as f32;
                let i = (pos.floor() as usize).min(n - 1);
                let frac = pos - i as f32;
                let cur = self.cycle_colors[i];
                let next = self.cycle_colors[(i + 1) % n];
                // Hold for 80 % of the slot, cross-fade for the last 20 %.
                if frac < 0.8 {
                    cur
                } else {
                    cur.lerp(next, (frac - 0.8) / 0.2)
                }
            }
            EffectKind::Rainbow => Rgb::from_hsv(phase * 360.0, 1.0, 1.0),
        }
    }

    /// EC intensity frame at time `t`.
    pub fn frame_at(&self, t: Duration) -> [u8; 3] {
        self.color_at(t).to_intensity()
    }

    /// Write the frame for `t` to the backend if it differs from the previous one.
    /// Returns `true` if a write happened.
    pub fn apply(&mut self, t: Duration, backend: &mut dyn Backend) -> Result<bool> {
        let frame = self.frame_at(t);
        if self.last_frame == Some(frame) {
            return Ok(false);
        }
        backend.set_intensity(frame)?;
        self.last_frame = Some(frame);
        Ok(true)
    }

    /// Forget the last written frame (e.g. after resume, when the EC state is unknown).
    pub fn invalidate(&mut self) {
        self.last_frame = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FakeBackend;

    fn engine(kind: EffectKind, speed: u8) -> EffectEngine {
        EffectEngine::new(Effect {
            kind,
            color: Rgb::new(0, 0, 255),
            speed,
            colors: default_cycle_colors(),
        })
    }

    #[test]
    fn static_writes_once() {
        let mut e = engine(EffectKind::Static, 5);
        let mut b = FakeBackend::new();
        assert!(e.apply(Duration::ZERO, &mut b).unwrap());
        assert!(!e.apply(Duration::from_secs(1), &mut b).unwrap());
        assert_eq!(b.snapshot().intensity, [0, 0, 50]);
        assert!(e.frame_interval().is_none());
    }

    #[test]
    fn breathing_never_black_and_peaks() {
        let e = engine(EffectKind::Breathing, 1); // 8 s period
        assert_ne!(e.frame_at(Duration::ZERO), [0, 0, 0]);
        assert_eq!(e.frame_at(Duration::from_secs(4)), [0, 0, 50]);
        let low = e.frame_at(Duration::ZERO)[2];
        let mid = e.frame_at(Duration::from_secs(2))[2];
        assert!(low < mid && mid < 50);
    }

    #[test]
    fn cycle_holds_then_fades() {
        let e = engine(EffectKind::Cycle, 1); // 12 s per colour, 7 colours
        assert_eq!(e.color_at(Duration::ZERO), Rgb::new(255, 0, 0));
        assert_eq!(e.color_at(Duration::from_secs(9)), Rgb::new(255, 0, 0));
        assert_eq!(e.color_at(Duration::from_secs(12)), Rgb::new(255, 165, 0));
        let fading = e.color_at(Duration::from_secs_f32(11.0));
        assert_ne!(fading, Rgb::new(255, 0, 0));
        assert_ne!(fading, Rgb::new(255, 165, 0));
    }

    #[test]
    fn rainbow_sweeps_hue() {
        let e = engine(EffectKind::Rainbow, 1); // 24 s period
        assert_eq!(e.color_at(Duration::ZERO), Rgb::new(255, 0, 0));
        assert_eq!(e.color_at(Duration::from_secs(8)), Rgb::new(0, 255, 0));
        assert_eq!(e.color_at(Duration::from_secs(16)), Rgb::new(0, 0, 255));
    }

    #[test]
    fn speed_scales_period() {
        let slow = engine(EffectKind::Breathing, 1).effect().period();
        let fast = engine(EffectKind::Breathing, 10).effect().period();
        assert!(slow > fast);
        assert_eq!(engine(EffectKind::Breathing, 0).effect().period(), slow);
    }

    #[test]
    fn coalesces_frames() {
        let mut e = engine(EffectKind::Rainbow, 1);
        let mut b = FakeBackend::new();
        let mut writes = 0;
        for ms in (0..24_000).step_by(66) {
            if e.apply(Duration::from_millis(ms), &mut b).unwrap() {
                writes += 1;
            }
        }
        let frames = 24_000 / 66;
        assert!(writes > 50, "should animate ({writes} writes)");
        assert!(writes < frames, "should coalesce ({writes} of {frames})");
    }

    #[test]
    fn effect_kind_parse() {
        assert_eq!("Monochrome".parse::<EffectKind>().unwrap(), EffectKind::Static);
        assert_eq!("manual".parse::<EffectKind>().unwrap(), EffectKind::Cycle);
        assert!("disco".parse::<EffectKind>().is_err());
    }
}
