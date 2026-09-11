//! `omarchy-eluktonics-keyboard` – command-line client for the keyboard RGB daemon.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use uwbl_core::{PowerSource, Profile, Status};

#[derive(Parser, Debug)]
#[command(version, about = "Control the Eluktronics RP-17 keyboard RGB lighting")]
struct Cli {
    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Show current state (default).
    Status,
    /// Turn the backlight on.
    On,
    /// Turn the backlight off.
    Off,
    /// Toggle the backlight.
    Toggle,
    /// Set brightness 0-4 (0 = off), or `up` / `down`.
    Brightness { level: String },
    /// Set a static colour: `#rrggbb`, `r,g,b` or a preset name (see `colors`).
    Color { color: String },
    /// Set the effect: static, breathing, cycle, rainbow.
    Effect {
        effect: String,
        /// Animation speed 1-10.
        #[arg(short, long)]
        speed: Option<u8>,
    },
    /// Set animation speed 1-10.
    Speed { speed: u8 },
    /// Set the colours used by the `cycle` effect.
    Cycle { colors: Vec<String> },
    /// Fan mode: balanced (Windows "Gaming") or performance ("Beast").
    Fan { mode: String },
    /// Fan boost: on / off.
    Boost { on: String },
    /// Manage AC / battery profiles.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    /// Discard runtime changes and reload the profiles from config.toml.
    Reset,
    /// Re-apply the current profile to the hardware.
    Reapply,
    /// List colour presets.
    Colors,
    /// Print status changes as they happen (Ctrl-C to stop).
    Watch,
}

#[derive(Subcommand, Debug)]
enum ProfileCmd {
    /// Show both profiles.
    Show,
    /// Copy one profile onto the other (default: ac -> battery).
    Copy {
        #[arg(default_value = "ac")]
        from: String,
        #[arg(default_value = "battery")]
        to: String,
    },
    /// Merge JSON into a profile (`uwbl profile set battery '{"enabled":false}'`).
    Set { source: String, patch: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(Cmd::Colors) = cli.cmd {
        for (name, rgb) in uwbl_core::color::PRESETS {
            println!("{name:<12} {}", rgb.to_hex());
        }
        return Ok(());
    }

    let proxy = uwbl_dbus::connect()
        .await
        .context("connecting to the keyboard daemon - is uwbld.service running?")?;

    match cli.cmd.unwrap_or(Cmd::Status) {
        Cmd::Status => {}
        Cmd::On => proxy.set_enabled(true).await?,
        Cmd::Off => proxy.set_enabled(false).await?,
        Cmd::Toggle => {
            let s = proxy.status().await?;
            proxy.set_enabled(!s.enabled).await?
        }
        Cmd::Brightness { level } => {
            let s = proxy.status().await?;
            let current = if s.enabled { s.brightness } else { 0 };
            let level = match level.as_str() {
                "up" | "+" => (current + 1).min(s.max_brightness),
                "down" | "-" => current.saturating_sub(1),
                n => n.parse().context("brightness must be 0-4, up or down")?,
            };
            proxy.set_brightness(level).await?
        }
        Cmd::Color { color } => proxy.set_color(&color).await?,
        Cmd::Effect { effect, speed } => proxy.set_effect(&effect, speed.unwrap_or(0)).await?,
        Cmd::Speed { speed } => proxy.set_speed(speed).await?,
        Cmd::Cycle { colors } => {
            if colors.is_empty() {
                bail!("give at least one colour");
            }
            proxy.set_cycle_colors(colors).await?
        }
        Cmd::Fan { mode } => proxy.set_fan_mode(&mode).await?,
        Cmd::Boost { on } => proxy.set_fan_boost(parse_bool(&on)?).await?,
        Cmd::Profile { cmd } => match cmd {
            ProfileCmd::Show => {
                let s = proxy.status().await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&s.profiles)?);
                } else {
                    print_profile("ac", &s.profiles.ac);
                    print_profile("battery", &s.profiles.battery);
                }
                return Ok(());
            }
            ProfileCmd::Copy { from, to } => {
                let s = proxy.status().await?;
                let src: PowerSource = from.parse()?;
                let p = s.profiles.get(src).clone();
                proxy.set_profile(&to, &serde_json::to_string(&p)?).await?
            }
            ProfileCmd::Set { source, patch } => {
                let s = proxy.status().await?;
                let src: PowerSource = source.parse()?;
                let mut base = serde_json::to_value(s.profiles.get(src))?;
                let patch: serde_json::Value =
                    serde_json::from_str(&patch).context("invalid JSON")?;
                merge(&mut base, patch);
                let profile: Profile = serde_json::from_value(base)?;
                proxy
                    .set_profile(&source, &serde_json::to_string(&profile)?)
                    .await?
            }
        },
        Cmd::Reset => proxy.reset_to_config().await?,
        Cmd::Reapply => proxy.reapply().await?,
        Cmd::Colors => unreachable!(),
        Cmd::Watch => {
            let mut stream = proxy.receive_status_changed().await?;
            print_status(&proxy.status().await?, cli.json);
            while let Some(sig) = stream.next().await {
                let args = sig.args()?;
                if cli.json {
                    println!("{}", args.status_json);
                } else {
                    let s: Status = serde_json::from_str(&args.status_json)?;
                    println!("---");
                    print_status(&s, false);
                }
            }
            return Ok(());
        }
    }

    print_status(&proxy.status().await?, cli.json);
    Ok(())
}

fn parse_bool(s: &str) -> Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Ok(true),
        "off" | "false" | "0" | "no" => Ok(false),
        _ => bail!("expected on|off"),
    }
}

fn merge(base: &mut serde_json::Value, patch: serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (b, p) => *b = p,
    }
}

fn print_status(s: &Status, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(s).unwrap());
        return;
    }
    println!("backend:     {}", s.backend);
    println!("power:       {}", s.power_source);
    let state = if s.idle_off {
        "off (idle)".to_string()
    } else if s.enabled {
        format!("on, brightness {}/{}", s.brightness, s.max_brightness)
    } else {
        "off".to_string()
    };
    println!("backlight:   {state}");
    println!("color:       {}", s.color);
    let speed = if s.effect.is_animated() {
        format!(" (speed {})", s.speed)
    } else {
        String::new()
    };
    println!("effect:      {}{speed}", s.effect);
    match s.fan_mode {
        Some(m) => println!("fan mode:    {m} ({})", m.windows_name()),
        None => println!("fan mode:    unsupported"),
    }
    match s.fan_boost {
        Some(b) => println!("fan boost:   {}", if b { "on" } else { "off" }),
        None => println!("fan boost:   unsupported"),
    }
}

fn print_profile(name: &str, p: &Profile) {
    println!("[{name}]");
    println!("  enabled:    {}", p.enabled);
    println!("  brightness: {}", p.brightness);
    println!("  effect:     {} (speed {})", p.effect.kind, p.effect.speed);
    println!("  color:      {}", p.effect.color);
    println!(
        "  cycle:      {}",
        p.effect
            .colors
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  fan mode:   {}",
        p.fan_mode
            .map(|m| m.to_string())
            .unwrap_or_else(|| "(unchanged)".into())
    );
}
