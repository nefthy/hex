//! Destination shortcuts on X11. Match an exact WM_CLASS instance or class:
//! installed browser apps commonly share a class but have their own instance.

use super::input::LinuxHotkey;
use crate::app_settings::{AppSettings, HotkeySlot};
use color_eyre::eyre::{Result, bail, ensure, eyre};

pub(crate) const LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Destination {
    pub shortcut: LinuxHotkey,
    pub window_class: String,
    pub title_contains: Option<String>,
    pub before: Option<LinuxHotkey>,
    pub submit: bool,
}

/// Read the same rules in Settings and in the listener. Invalid edits do
/// not partially register a list and change which destination a key names.
pub(crate) fn parse(settings: &AppSettings, text: &str) -> Result<Vec<Destination>> {
    let rows = read_rows(text)?;
    for row in &rows {
        for key in std::iter::once(&row.shortcut).chain(row.before.as_ref()) {
            ensure!(
                !settings.hotkey_in_use(HotkeySlot::Destination, &key.into()),
                "A shortcut overlaps another Hex action"
            );
        }
    }
    Ok(rows)
}

pub(crate) fn read_rows(text: &str) -> Result<Vec<Destination>> {
    let mut rows: Vec<Destination> = Vec::new();
    ensure!(text.len() <= 4096, "Destination list is too long");
    for (i, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let line = line.split('|').map(str::trim).collect::<Vec<_>>();
        let [trigger, class, rest @ ..] = &line[..] else {
            bail!("Line {} needs a shortcut and a window class", i + 1)
        };
        ensure!(
            !class.is_empty() && !class.contains(char::is_control) && rest.len() <= 2,
            "Invalid destination on line {}",
            i + 1
        );
        ensure!(rows.len() < LIMIT, "At most {LIMIT} destination shortcuts");
        let shortcut = chord(trigger)?;
        ensure!(
            shortcut.control
                || shortcut.alt
                || shortcut.super_key
                || (shortcut.key.starts_with('f') && shortcut.key[1..].parse::<u8>().is_ok()),
            "Use Ctrl, Alt, Super or a function key for a global shortcut"
        );
        let (class, title_contains) = match class.split_once('@') {
            Some((class, title)) => {
                ensure!(
                    !class.trim().is_empty() && !title.trim().is_empty(),
                    "Use class@title with a nonempty class and title"
                );
                (class.trim(), Some(title.trim().to_lowercase()))
            }
            None => (*class, None),
        };
        ensure!(
            !(shortcut.control && !shortcut.alt && !shortcut.super_key && shortcut.key == "v"),
            "A global shortcut cannot take the paste key"
        );
        let before = rest
            .first()
            .filter(|s| !s.is_empty())
            .map(|s| chord(s))
            .transpose()?;
        let submit = match rest.get(1).copied().unwrap_or("") {
            "" => false,
            "enter" => true,
            _ => bail!("Use enter or leave the fourth field empty"),
        };
        rows.push(Destination {
            shortcut,
            window_class: class.to_string(),
            title_contains,
            before,
            submit,
        });
    }
    for row in &rows {
        ensure!(
            rows.iter().filter(|r| r.shortcut == row.shortcut).count() == 1,
            "Duplicate destination shortcut"
        );
        ensure!(
            !rows
                .iter()
                .any(|r| r.before.as_ref() == Some(&row.shortcut)),
            "A preparation key is also a destination shortcut"
        );
    }
    Ok(rows)
}

fn chord(value: &str) -> Result<LinuxHotkey> {
    let mut result = LinuxHotkey {
        alt: false,
        ..Default::default()
    };
    let value = value.to_ascii_lowercase();
    let mut parts = value.split('+').map(str::trim);
    let key = parts.next_back().unwrap_or("");
    result.key = super::input::keysym(key)
        .ok_or_else(|| eyre!("Unknown key: {key}"))?
        .name
        .to_ascii_lowercase();
    // Keep a spelling accepted by keysym (including aliases such as pageup).
    if super::input::keysym(&result.key).is_none() {
        result.key = key.to_owned();
    }
    for part in parts {
        let bit = match part {
            "ctrl" | "control" => &mut result.control,
            "alt" => &mut result.alt,
            "shift" => &mut result.shift,
            "super" => &mut result.super_key,
            _ => bail!("Unknown modifier: {part}"),
        };
        ensure!(!*bit, "Repeated modifier: {part}");
        *bit = true;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_can_share_an_app_but_not_a_trigger_or_a_preparation_trigger() {
        let settings = AppSettings::default();
        let rows = parse(
            &settings,
            "F6 | crx_example\nF7 | crx_example | Ctrl+N | enter",
        )
        .unwrap();
        assert!(!rows[0].submit && rows[0].before.is_none());
        assert!(rows[1].submit && rows[1].before.as_ref().unwrap().control);
        for bad in [
            "F6|app\nf6|other",
            "F6|app|F7\nF7|other",
            "F6|",
            "A|app",
            "F6|app||yes",
        ] {
            assert!(parse(&settings, bad).is_err(), "{bad}");
        }
    }
}
