//! Lista os microfones e saídas do sistema (PulseAudio/PipeWire, pelo `pactl`).

use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// Nome técnico (o que vai para o PulseAudio).
    pub name: String,
    /// Nome para mostrar.
    pub description: String,
    /// Quantos canais o aparelho tem (1 = mono, 2 = estéreo).
    pub channels: u8,
}

/// Lê a saída de `pactl list sources|sinks`: blocos com "Name:", "Description:" e
/// "Sample Specification: s16le 2ch 48000Hz".
fn parse(text: &str) -> Vec<Device> {
    let mut out = Vec::new();
    let mut cur: Option<Device> = None;
    for line in text.lines() {
        let l = line.trim();
        if let Some(name) = l.strip_prefix("Name:") {
            if let Some(d) = cur.take() {
                out.push(d);
            }
            cur = Some(Device {
                name: name.trim().to_string(),
                description: name.trim().to_string(),
                channels: 2,
            });
        } else if let Some(desc) = l.strip_prefix("Description:") {
            if let Some(d) = cur.as_mut() {
                d.description = desc.trim().to_string();
            }
        } else if let Some(spec) = l.strip_prefix("Sample Specification:")
            && let Some(d) = cur.as_mut()
            && let Some(ch) = spec.split_whitespace().find_map(|w| w.strip_suffix("ch"))
        {
            d.channels = ch.parse().unwrap_or(2);
        }
    }
    if let Some(d) = cur {
        out.push(d);
    }
    out
}

fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl")
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Microfones e entradas (sem os "monitores" das saídas).
pub fn sources() -> Vec<Device> {
    pactl(&["list", "sources"])
        .map(|t| parse(&t))
        .unwrap_or_default()
        .into_iter()
        .filter(|d| !d.name.ends_with(".monitor"))
        .collect()
}

pub fn sinks() -> Vec<Device> {
    pactl(&["list", "sinks"])
        .map(|t| parse(&t))
        .unwrap_or_default()
}

/// Nome do microfone padrão do sistema.
pub fn default_source() -> Option<String> {
    pactl(&["get-default-source"]).map(|s| s.trim().to_string())
}

pub fn default_sink() -> Option<String> {
    pactl(&["get-default-sink"]).map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pactl_blocks() {
        let text = "Source #59\n\tState: SUSPENDED\n\tName: alsa_input.usb-Fifine\n\tDescription: Fifine Microphone Analog Stereo\n\tDriver: PipeWire\n\tSample Specification: s16le 2ch 48000Hz\n\nSource #61\n\tName: alsa_input.mono\n\tDescription: Mic\n\tSample Specification: s32le 1ch 44100Hz\n";
        let d = parse(text);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].description, "Fifine Microphone Analog Stereo");
        assert_eq!((d[0].channels, d[1].channels), (2, 1));
    }
}
