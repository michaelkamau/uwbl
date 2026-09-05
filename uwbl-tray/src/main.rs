//! `uwbl-tray` – StatusNotifierItem (KDE/Plasma, and any SNI host) for `uwbld`.
//!
//! Left click opens the menu, middle click toggles the backlight, mouse wheel changes brightness, the menu exposes
//! brightness / colour / effect / speed / fan controls. The icon is a swatch of the current
//! colour so the tray reflects the keyboard at a glance.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Icon, MenuItem, ToolTip, TrayMethods};
use log::{info, warn};
use tokio::sync::mpsc;
use uwbl_core::color::PRESETS;
use uwbl_core::{EffectKind, FanMode, Rgb, Status};
use uwbl_dbus::BacklightProxy;

/// Commands issued from the (synchronous) menu callbacks to the async D-Bus task.
#[derive(Debug, Clone)]
enum Cmd {
    Enabled(bool),
    Brightness(u8),
    Color(Rgb),
    Effect(EffectKind),
    Speed(u8),
    FanMode(FanMode),
    FanBoost(bool),
    CopyAcToBattery,
    Reset,
    Quit,
}

struct Tray {
    status: Option<Status>,
    tx: mpsc::UnboundedSender<Cmd>,
}

impl Tray {
    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    fn effective_brightness(&self) -> u8 {
        match &self.status {
            Some(s) if s.enabled && !s.idle_off => s.brightness,
            _ => 0,
        }
    }
}

const SPEEDS: [u8; 5] = [1, 3, 5, 7, 10];

impl ksni::Tray for Tray {
    /// Left click opens the menu (Plasma convention); middle click toggles the backlight.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "uwbl-tray".into()
    }

    fn title(&self) -> String {
        "Keyboard backlight".into()
    }

    fn icon_name(&self) -> String {
        "input-keyboard".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let color = match &self.status {
            Some(s) if s.enabled && !s.idle_off => {
                // Dim the swatch with brightness so level 1 and 4 look different.
                let f = 0.4 + 0.6 * (s.brightness as f32 / s.max_brightness.max(1) as f32);
                s.color.scaled(f)
            }
            Some(_) => Rgb::new(70, 70, 70),
            None => Rgb::new(40, 40, 40),
        };
        [22, 32, 48]
            .into_iter()
            .map(|size| swatch_icon(size, color))
            .collect()
    }

    fn tool_tip(&self) -> ToolTip {
        let description = match &self.status {
            None => "uwbld not running".to_string(),
            Some(s) => {
                let mut lines = vec![if s.idle_off {
                    "Off (idle)".to_string()
                } else if s.enabled {
                    format!("Brightness {}/{}", s.brightness, s.max_brightness)
                } else {
                    "Off".to_string()
                }];
                lines.push(format!("{} · {}", s.color, s.effect));
                if let Some(m) = s.fan_mode {
                    lines.push(format!("Fan: {} ({})", m, m.windows_name()));
                }
                lines.push(format!("Power: {}", s.power_source));
                lines.join("\n")
            }
        };
        ToolTip {
            title: "Keyboard backlight".into(),
            description,
            icon_name: "input-keyboard".into(),
            ..Default::default()
        }
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        if let Some(s) = &self.status {
            self.send(Cmd::Enabled(!s.enabled));
        }
    }

    fn scroll(&mut self, delta: i32, _orientation: ksni::Orientation) {
        let Some(s) = &self.status else { return };
        let cur = self.effective_brightness() as i32;
        let next = (cur + delta.signum()).clamp(0, s.max_brightness as i32);
        if next != cur {
            self.send(Cmd::Brightness(next as u8));
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let Some(s) = self.status.clone() else {
            return vec![
                StandardItem {
                    label: "uwbld is not running".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                quit_item(),
            ];
        };

        let brightness_group = RadioGroup {
            selected: self.effective_brightness() as usize,
            select: Box::new(|t: &mut Self, i| t.send(Cmd::Brightness(i as u8))),
            options: (0..=s.max_brightness)
                .map(|l| RadioItem {
                    label: if l == 0 {
                        "Off".into()
                    } else {
                        format!("Level {l}")
                    },
                    ..Default::default()
                })
                .collect(),
        };

        let preset_idx = PRESETS.iter().position(|(_, c)| *c == s.color);
        let color_menu = SubMenu {
            label: format!("Colour: {}", s.color),
            icon_name: "color-picker".into(),
            submenu: vec![RadioGroup {
                selected: preset_idx.unwrap_or(usize::MAX),
                select: Box::new(|t: &mut Self, i| {
                    if let Some((_, c)) = PRESETS.get(i) {
                        t.send(Cmd::Color(*c));
                    }
                }),
                options: PRESETS
                    .iter()
                    .map(|(name, _)| RadioItem {
                        label: capitalise(name),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into()],
            ..Default::default()
        };

        let effect_menu = SubMenu {
            label: format!("Effect: {}", s.effect),
            icon_name: "view-refresh".into(),
            submenu: vec![RadioGroup {
                selected: EffectKind::ALL
                    .iter()
                    .position(|k| *k == s.effect)
                    .unwrap_or(0),
                select: Box::new(|t: &mut Self, i| t.send(Cmd::Effect(EffectKind::ALL[i]))),
                options: EffectKind::ALL
                    .iter()
                    .map(|k| RadioItem {
                        label: capitalise(k.name()),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into()],
            ..Default::default()
        };

        let speed_menu = SubMenu {
            label: format!("Speed: {}", s.speed),
            enabled: s.effect.is_animated(),
            submenu: vec![RadioGroup {
                selected: SPEEDS
                    .iter()
                    .position(|v| *v == s.speed)
                    .unwrap_or(usize::MAX),
                select: Box::new(|t: &mut Self, i| t.send(Cmd::Speed(SPEEDS[i]))),
                options: SPEEDS
                    .iter()
                    .map(|v| RadioItem {
                        label: format!("{v}"),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into()],
            ..Default::default()
        };

        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: format!("Keyboard backlight ({} profile)", s.power_source),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            brightness_group.into(),
            MenuItem::Separator,
            color_menu.into(),
            effect_menu.into(),
            speed_menu.into(),
        ];

        if let Some(mode) = s.fan_mode {
            items.push(MenuItem::Separator);
            items.push(
                SubMenu {
                    label: format!("Fan mode: {} ({})", mode, mode.windows_name()),
                    icon_name: "temperature-normal".into(),
                    submenu: vec![RadioGroup {
                        selected: if mode == FanMode::Balanced { 0 } else { 1 },
                        select: Box::new(|t: &mut Self, i| {
                            t.send(Cmd::FanMode(if i == 0 {
                                FanMode::Balanced
                            } else {
                                FanMode::Performance
                            }))
                        }),
                        options: vec![
                            RadioItem {
                                label: "Balanced (Gaming)".into(),
                                ..Default::default()
                            },
                            RadioItem {
                                label: "Performance (Beast)".into(),
                                ..Default::default()
                            },
                        ],
                    }
                    .into()],
                    ..Default::default()
                }
                .into(),
            );
        }
        if let Some(boost) = s.fan_boost {
            items.push(
                CheckmarkItem {
                    label: "Fan boost".into(),
                    checked: boost,
                    activate: Box::new(move |t: &mut Self| t.send(Cmd::FanBoost(!boost))),
                    ..Default::default()
                }
                .into(),
            );
        }

        items.extend([
            MenuItem::Separator,
            StandardItem {
                label: "Copy AC profile to battery".into(),
                icon_name: "edit-copy".into(),
                activate: Box::new(|t: &mut Self| t.send(Cmd::CopyAcToBattery)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Reset to config defaults".into(),
                icon_name: "edit-undo".into(),
                activate: Box::new(|t: &mut Self| t.send(Cmd::Reset)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            quit_item(),
        ]);
        items
    }
}

fn quit_item() -> MenuItem<Tray> {
    StandardItem {
        label: "Quit tray".into(),
        icon_name: "application-exit".into(),
        activate: Box::new(|t: &mut Tray| t.send(Cmd::Quit)),
        ..Default::default()
    }
    .into()
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Rounded square swatch as ARGB32 (network byte order), as required by the SNI spec.
fn swatch_icon(size: i32, color: Rgb) -> Icon {
    let n = size as usize;
    let radius = (n as f32) * 0.22;
    let mut data = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let a = rounded_rect_alpha(x as f32 + 0.5, y as f32 + 0.5, n as f32, radius);
            data.extend_from_slice(&[a, color.r, color.g, color.b]);
        }
    }
    Icon {
        width: size,
        height: size,
        data,
    }
}

fn rounded_rect_alpha(x: f32, y: f32, n: f32, r: f32) -> u8 {
    let inset = 1.0;
    let (lo, hi) = (inset, n - inset);
    if x < lo || y < lo || x > hi || y > hi {
        return 0;
    }
    // Clamp to the nearest corner-circle centre; inside the straight edges d == 0.
    let cx = x.clamp(lo + r, hi - r);
    let cy = y.clamp(lo + r, hi - r);
    let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
    let cov = (r + 0.5 - d).clamp(0.0, 1.0);
    (cov * 255.0) as u8
}

fn to_json<T: serde::Serialize>(v: &T) -> zbus::Result<String> {
    serde_json::to_string(v).map_err(|e| zbus::Error::Failure(e.to_string()))
}

async fn run_commands(mut rx: mpsc::UnboundedReceiver<Cmd>, proxy: BacklightProxy<'static>) {
    while let Some(cmd) = rx.recv().await {
        let res = match cmd.clone() {
            Cmd::Enabled(on) => proxy.set_enabled(on).await,
            Cmd::Brightness(l) => proxy.set_brightness(l).await,
            Cmd::Color(c) => proxy.set_color(&c.to_hex()).await,
            Cmd::Effect(k) => proxy.set_effect(k.name(), 0).await,
            Cmd::Speed(v) => proxy.set_speed(v).await,
            Cmd::FanMode(m) => proxy.set_fan_mode(m.as_profile()).await,
            Cmd::FanBoost(b) => proxy.set_fan_boost(b).await,
            Cmd::CopyAcToBattery => {
                match proxy.status().await.and_then(|s| to_json(&s.profiles.ac)) {
                    Ok(json) => proxy.set_profile("battery", &json).await,
                    Err(e) => Err(e),
                }
            }
            Cmd::Reset => proxy.reset_to_config().await,
            Cmd::Quit => std::process::exit(0),
        };
        if let Err(e) = res {
            warn!("{cmd:?} failed: {e}");
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let (tx, rx) = mpsc::unbounded_channel();
    let handle = Tray { status: None, tx }
        .spawn()
        .await
        .context("registering StatusNotifierItem – is a system tray running?")?;

    // The bus connection itself is stable; the daemon coming and going is tracked via
    // NameOwnerChanged, so we only need to connect once (retrying while the bus is unavailable).
    let proxy = loop {
        match uwbl_dbus::connect().await {
            Ok(p) => break p,
            Err(e) => {
                warn!("cannot connect to D-Bus ({e}); retrying in 10 s");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    };
    tokio::spawn(run_commands(rx, proxy.clone()));

    let mut changes = proxy.receive_status_changed().await?;
    let mut owner = proxy.inner().receive_owner_changed().await?;
    match proxy.status().await {
        Ok(s) => {
            info!("connected to uwbld");
            handle.update(|t| t.status = Some(s)).await;
        }
        Err(e) => warn!("uwbld not reachable yet: {e}"),
    }

    loop {
        tokio::select! {
            sig = changes.next() => {
                let Some(sig) = sig else { break };
                if let Ok(args) = sig.args() {
                    if let Ok(s) = serde_json::from_str::<Status>(&args.status_json) {
                        handle.update(|t| t.status = Some(s)).await;
                    }
                }
            }
            o = owner.next() => {
                match o {
                    Some(Some(_)) => {
                        info!("uwbld appeared");
                        if let Ok(s) = proxy.status().await {
                            handle.update(|t| t.status = Some(s)).await;
                        }
                    }
                    Some(None) => {
                        warn!("uwbld went away");
                        handle.update(|t| t.status = None).await;
                    }
                    None => break,
                }
            }
        }
    }
    anyhow::bail!("lost D-Bus connection")
}
