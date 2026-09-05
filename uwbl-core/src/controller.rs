//! Synchronous controller: owns the backend, the profiles and the effect engine and applies
//! changes. The daemon wraps it in a mutex and adds timers, D-Bus and power/resume events.

use std::time::{Duration, Instant};

use log::{debug, info, warn};

use crate::backend::{Backend, FanMode};
use crate::color::Rgb;
use crate::config::{Config, PowerSource, Profile, Profiles};
use crate::effect::{Effect, EffectEngine, EffectKind};
use crate::state::{State, Status};
use crate::Result;

pub struct Controller {
    backend: Box<dyn Backend>,
    config: Config,
    profiles: Profiles,
    fan_boost: bool,
    power: PowerSource,
    engine: EffectEngine,
    effect_started: Instant,
    /// Backlight forced off by the idle timer (independent of the profile's `enabled`).
    idle_off: bool,
    dirty: bool,
}

impl Controller {
    pub fn new(
        backend: Box<dyn Backend>,
        config: Config,
        state: Option<State>,
        power: PowerSource,
    ) -> Self {
        let (profiles, fan_boost) = match state {
            Some(s) => (s.profiles, s.fan_boost),
            None => (config.profiles.clone(), false),
        };
        let engine = EffectEngine::new(profiles.get(power).effect.clone());
        Self {
            backend,
            config,
            profiles,
            fan_boost,
            power,
            engine,
            effect_started: Instant::now(),
            idle_off: false,
            dirty: false,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn power_source(&self) -> PowerSource {
        self.power
    }

    pub fn profile(&self) -> &Profile {
        self.profiles.get(self.power)
    }

    pub fn profiles(&self) -> &Profiles {
        &self.profiles
    }

    /// Whether unsaved profile changes exist; cleared by [`take_state`](Self::take_state).
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn take_state(&mut self) -> State {
        self.dirty = false;
        State {
            profiles: self.profiles.clone(),
            fan_boost: self.fan_boost,
        }
    }

    /// Push the whole active profile (brightness, colour/effect, fan mode) to the hardware.
    pub fn apply_all(&mut self) -> Result<()> {
        let profile = self.profile().clone();
        info!(
            "applying {} profile: enabled={} brightness={} effect={} color={} fan={:?}",
            self.power,
            profile.enabled,
            profile.brightness,
            profile.effect.kind,
            profile.effect.color,
            profile.fan_mode
        );
        self.restart_engine(profile.effect.clone());
        self.apply_brightness()?;
        if !self.idle_off && profile.enabled {
            self.engine.invalidate();
            self.tick(Instant::now())?;
        }
        if let Some(mode) = profile.fan_mode {
            if self.backend.supports_fan_mode() {
                if let Err(e) = self.backend.set_fan_mode(mode) {
                    warn!("cannot set fan mode: {e}");
                }
            }
        }
        if self.backend.supports_fan_boost() {
            if let Err(e) = self.backend.set_fan_boost(self.fan_boost) {
                warn!("cannot set fan boost: {e}");
            }
        }
        Ok(())
    }

    fn restart_engine(&mut self, effect: Effect) {
        self.engine = EffectEngine::new(effect);
        self.effect_started = Instant::now();
    }

    fn apply_brightness(&mut self) -> Result<()> {
        let level = if self.idle_off {
            0
        } else {
            self.profile().effective_brightness()
        };
        let level = level.min(self.backend.max_brightness());
        self.backend.set_brightness(level)
    }

    /// Advance the animation. Returns the delay until the next tick, or `None` when static.
    pub fn tick(&mut self, now: Instant) -> Result<Option<Duration>> {
        if self.idle_off || !self.profile().enabled {
            return Ok(None);
        }
        let t = now.saturating_duration_since(self.effect_started);
        if self.engine.apply(t, self.backend.as_mut())? {
            debug!("frame {:?}", self.engine.frame_at(t));
        }
        Ok(self.engine.frame_interval())
    }

    // --- power / idle / hardware events -------------------------------------------------------

    pub fn set_power_source(&mut self, power: PowerSource) -> Result<()> {
        if power == self.power {
            return Ok(());
        }
        info!("power source {} -> {}", self.power, power);
        self.power = power;
        self.apply_all()
    }

    /// Called from the resume path; the EC may have been reset.
    pub fn resume(&mut self) -> Result<()> {
        if self.config.general.restore_on_resume {
            self.apply_all()
        } else {
            Ok(())
        }
    }

    pub fn set_idle_off(&mut self, off: bool) -> Result<()> {
        if off == self.idle_off {
            return Ok(());
        }
        debug!("idle off = {off}");
        self.idle_off = off;
        self.apply_brightness()?;
        if !off {
            self.engine.invalidate();
            self.tick(Instant::now())?;
        }
        Ok(())
    }

    pub fn is_idle_off(&self) -> bool {
        self.idle_off
    }

    /// Poll the backend for Fn-key brightness changes and fold them into the active profile.
    pub fn poll_hw_brightness(&mut self) -> Result<bool> {
        let Some(level) = self.backend.take_hw_brightness_change()? else {
            return Ok(false);
        };
        if !self.config.general.follow_hw_brightness {
            return Ok(false);
        }
        info!("hardware changed brightness to {level}");
        let p = self.profiles.get_mut(self.power);
        if level == 0 {
            p.enabled = false;
        } else {
            p.enabled = true;
            p.brightness = level;
        }
        self.dirty = true;
        self.idle_off = false;
        // Colour is preserved by the EC, but a running animation needs to keep going.
        if p.enabled {
            self.engine.invalidate();
            self.tick(Instant::now())?;
        }
        Ok(true)
    }

    // --- user-facing setters (act on the active profile) ---------------------------------------

    pub fn set_enabled(&mut self, on: bool) -> Result<()> {
        self.profiles.get_mut(self.power).enabled = on;
        self.dirty = true;
        self.idle_off = false;
        self.apply_brightness()?;
        if on {
            self.engine.invalidate();
            self.tick(Instant::now())?;
        }
        Ok(())
    }

    pub fn set_brightness(&mut self, level: u8) -> Result<()> {
        let max = self.backend.max_brightness();
        if level > max {
            return Err(crate::Error::Invalid(format!("brightness {level} > {max}")));
        }
        let p = self.profiles.get_mut(self.power);
        if level == 0 {
            p.enabled = false;
        } else {
            p.enabled = true;
            p.brightness = level;
        }
        self.dirty = true;
        self.idle_off = false;
        self.apply_brightness()?;
        if level > 0 {
            self.engine.invalidate();
            self.tick(Instant::now())?;
        }
        Ok(())
    }

    /// Set a static colour (switches the effect to `static` unless it is breathing).
    pub fn set_color(&mut self, color: Rgb) -> Result<()> {
        let p = self.profiles.get_mut(self.power);
        p.effect.color = color;
        if !matches!(p.effect.kind, EffectKind::Breathing) {
            p.effect.kind = EffectKind::Static;
        }
        let effect = p.effect.clone();
        self.dirty = true;
        self.restart_engine(effect);
        self.tick(Instant::now())?;
        Ok(())
    }

    pub fn set_effect(&mut self, kind: EffectKind, speed: Option<u8>) -> Result<()> {
        let p = self.profiles.get_mut(self.power);
        p.effect.kind = kind;
        if let Some(s) = speed {
            p.effect.speed = s.clamp(1, 10);
        }
        let effect = p.effect.clone();
        self.dirty = true;
        self.restart_engine(effect);
        self.tick(Instant::now())?;
        Ok(())
    }

    pub fn set_speed(&mut self, speed: u8) -> Result<()> {
        let p = self.profiles.get_mut(self.power);
        p.effect.speed = speed.clamp(1, 10);
        let effect = p.effect.clone();
        self.dirty = true;
        self.restart_engine(effect);
        self.tick(Instant::now())?;
        Ok(())
    }

    pub fn set_cycle_colors(&mut self, colors: Vec<Rgb>) -> Result<()> {
        if colors.is_empty() {
            return Err(crate::Error::Invalid(
                "cycle needs at least one colour".into(),
            ));
        }
        let p = self.profiles.get_mut(self.power);
        p.effect.colors = colors;
        let effect = p.effect.clone();
        self.dirty = true;
        self.restart_engine(effect);
        self.tick(Instant::now())?;
        Ok(())
    }

    pub fn set_fan_mode(&mut self, mode: FanMode) -> Result<()> {
        self.backend.set_fan_mode(mode)?;
        self.profiles.get_mut(self.power).fan_mode = Some(mode);
        self.dirty = true;
        Ok(())
    }

    pub fn set_fan_boost(&mut self, on: bool) -> Result<()> {
        self.backend.set_fan_boost(on)?;
        self.fan_boost = on;
        self.dirty = true;
        Ok(())
    }

    /// Replace a whole profile (used by `uwbl profile set` / tray "copy to battery").
    pub fn set_profile(&mut self, source: PowerSource, profile: Profile) -> Result<()> {
        *self.profiles.get_mut(source) = profile;
        self.dirty = true;
        if source == self.power {
            self.apply_all()?;
        }
        Ok(())
    }

    /// Discard runtime changes and go back to `config.toml`.
    pub fn reset_to_config(&mut self) -> Result<()> {
        self.profiles = self.config.profiles.clone();
        self.fan_boost = false;
        self.dirty = true;
        self.apply_all()
    }

    pub fn status(&self) -> Status {
        let p = self.profile();
        Status {
            backend: self.backend.describe(),
            power_source: self.power,
            enabled: p.enabled,
            brightness: p.brightness,
            max_brightness: self.backend.max_brightness(),
            color: p.effect.color,
            effect: p.effect.kind,
            speed: p.effect.speed,
            idle_off: self.idle_off,
            fan_mode: if self.backend.supports_fan_mode() {
                self.backend.fan_mode().ok()
            } else {
                None
            },
            fan_boost: if self.backend.supports_fan_boost() {
                self.backend.fan_boost().ok()
            } else {
                None
            },
            profiles: self.profiles.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FakeBackend;

    fn ctl(power: PowerSource) -> (Controller, FakeBackend) {
        let fake = FakeBackend::new();
        let c = Controller::new(Box::new(fake.clone()), Config::default(), None, power);
        (c, fake)
    }

    #[test]
    fn apply_ac_defaults() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.apply_all().unwrap();
        let s = fake.snapshot();
        assert_eq!(s.brightness, 3);
        assert_eq!(s.intensity, [50, 50, 50]);
    }

    #[test]
    fn battery_default_is_off_and_switching_back_restores() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.apply_all().unwrap();
        c.set_power_source(PowerSource::Battery).unwrap();
        assert_eq!(fake.snapshot().brightness, 0);
        c.set_power_source(PowerSource::Ac).unwrap();
        assert_eq!(fake.snapshot().brightness, 3);
    }

    #[test]
    fn color_and_brightness_persist_in_profile() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.apply_all().unwrap();
        c.set_color(Rgb::new(255, 0, 0)).unwrap();
        c.set_brightness(1).unwrap();
        assert_eq!(fake.snapshot().intensity, [50, 0, 0]);
        assert_eq!(fake.snapshot().brightness, 1);
        assert!(c.is_dirty());
        let st = c.take_state();
        assert!(!c.is_dirty());
        assert_eq!(st.profiles.ac.brightness, 1);
        assert_eq!(st.profiles.ac.effect.color, Rgb::new(255, 0, 0));
        assert_eq!(
            st.profiles.battery,
            Profile::default_for(PowerSource::Battery)
        );
    }

    #[test]
    fn brightness_zero_disables() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.set_brightness(0).unwrap();
        assert!(!c.profile().enabled);
        assert_eq!(fake.snapshot().brightness, 0);
        c.set_enabled(true).unwrap();
        assert_eq!(fake.snapshot().brightness, 3);
        assert!(c.set_brightness(9).is_err());
    }

    #[test]
    fn idle_off_and_back() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.apply_all().unwrap();
        c.set_idle_off(true).unwrap();
        assert_eq!(fake.snapshot().brightness, 0);
        assert!(c.tick(Instant::now()).unwrap().is_none());
        c.set_idle_off(false).unwrap();
        assert_eq!(fake.snapshot().brightness, 3);
        assert!(c.profile().enabled);
    }

    #[test]
    fn hw_change_updates_profile() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.apply_all().unwrap();
        fake.state.lock().unwrap().pending_hw_change = Some(1);
        assert!(c.poll_hw_brightness().unwrap());
        assert_eq!(c.profile().brightness, 1);
        fake.state.lock().unwrap().pending_hw_change = Some(0);
        c.poll_hw_brightness().unwrap();
        assert!(!c.profile().enabled);
        assert!(!c.poll_hw_brightness().unwrap());
    }

    #[test]
    fn animated_effect_ticks() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.apply_all().unwrap();
        c.set_effect(EffectKind::Rainbow, Some(10)).unwrap();
        let before = fake.snapshot().writes;
        let next = c.tick(Instant::now() + Duration::from_secs(1)).unwrap();
        assert!(next.is_some());
        assert!(fake.snapshot().writes > before);
        c.set_effect(EffectKind::Static, None).unwrap();
        assert!(c.tick(Instant::now()).unwrap().is_none());
    }

    #[test]
    fn fan_controls() {
        let (mut c, fake) = ctl(PowerSource::Ac);
        c.set_fan_mode(FanMode::Performance).unwrap();
        c.set_fan_boost(true).unwrap();
        assert_eq!(fake.snapshot().fan_mode, FanMode::Performance);
        assert!(fake.snapshot().fan_boost);
        let st = c.status();
        assert_eq!(st.fan_mode, Some(FanMode::Performance));
        assert_eq!(st.fan_boost, Some(true));
        assert_eq!(c.profile().fan_mode, Some(FanMode::Performance));
    }

    #[test]
    fn state_overrides_config() {
        let fake = FakeBackend::new();
        let mut st = State::default();
        st.profiles.ac.brightness = 4;
        let mut c = Controller::new(
            Box::new(fake.clone()),
            Config::default(),
            Some(st),
            PowerSource::Ac,
        );
        c.apply_all().unwrap();
        assert_eq!(fake.snapshot().brightness, 4);
        c.reset_to_config().unwrap();
        assert_eq!(fake.snapshot().brightness, 3);
    }
}
