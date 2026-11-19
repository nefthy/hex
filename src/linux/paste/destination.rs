//! Activate an existing destination through the EWMH window manager. Keep
//! the exact window ID through preparation, clipboard publication and Return.

use super::*;
use crate::linux::{
    destinations::Destination,
    input::{LinuxHotkey, XK_ALT_L, XK_SUPER_L},
};
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, EventMask, Window};

impl LinuxPaster {
    fn paste_to(&mut self, dest: &Destination, text: &str) -> Result<()> {
        let Backend::X11(x) = &mut self.backend else {
            return Err(eyre!("App destinations need X11"));
        };
        x.wait_for_modifier_release(&self.stop, &self.keys)?;
        let active = x.atom(b"_NET_ACTIVE_WINDOW")?;
        let window = x.find_window(&dest.window_class, dest.title_contains.as_deref())?;
        let target = Target {
            active,
            window,
            title: dest.title_contains.clone(),
        };
        x.activate(target, &self.stop)?;
        if let Some(before) = &dest.before {
            x.send_shortcut(before)?;
            // Give the application a turn to create and focus its composer.
            thread::sleep(Duration::from_millis(250));
        }
        // `send_keys` checks again after the clipboard and modifier waits.
        self.paste(text, false)?;
        if dest.submit {
            self.press_return()?;
        }
        Ok(())
    }

    /// Clear the guard on both success and failure before the next job.
    pub(crate) fn deliver_to(&mut self, dest: &Destination, text: &str) -> Result<()> {
        let result = self.paste_to(dest, text);
        if let Backend::X11(x) = &mut self.backend {
            x.target = None;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{CreateWindowAux, InputFocus, PropMode, WindowClass};
    use x11rb::wrapper::ConnectionExt as _;

    /// Run only on a private Xvfb. Three windows: two browser instances
    /// share a class; one app's child owns keyboard focus. A subsequent
    /// focus change must disarm even an optional Return.
    #[test]
    #[ignore = "needs an isolated X11 display"]
    fn destination_matches_browser_instance_and_rechecks_actual_focus() {
        let mut x = X11Paster::new().unwrap();
        let c = &x.connection;
        let make = |parent| {
            let id = c.generate_id().unwrap();
            c.create_window(
                0,
                id,
                parent,
                0,
                0,
                100,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().event_mask(EventMask::KEY_PRESS),
            )
            .unwrap()
            .check()
            .unwrap();
            c.map_window(id).unwrap().check().unwrap();
            id
        };
        let app = make(x.root);
        let other = make(x.root);
        let editor = make(app);
        for (id, class) in [
            (app, &b"crx_example\0Browser\0"[..]),
            (other, b"browser\0Browser\0"),
        ] {
            c.change_property8(
                PropMode::REPLACE,
                id,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                class,
            )
            .unwrap()
            .check()
            .unwrap();
        }
        let stack = x.atom(b"_NET_CLIENT_LIST_STACKING").unwrap();
        c.change_property32(
            PropMode::REPLACE,
            x.root,
            stack,
            AtomEnum::WINDOW,
            &[app, other],
        )
        .unwrap()
        .check()
        .unwrap();
        assert_eq!(x.find_window("crx_example", None).unwrap(), app);
        assert!(x.find_window("absent", None).is_err());
        let title = x.atom(b"_NET_WM_NAME").unwrap();
        let utf8 = x.atom(b"UTF8_STRING").unwrap();
        c.change_property8(
            PropMode::REPLACE,
            app,
            title,
            utf8,
            b"My session - OpenCode",
        )
        .unwrap()
        .check()
        .unwrap();
        assert_eq!(x.find_window("browser", Some("opencode")).unwrap(), app);
        assert!(x.find_window("browser", Some("other site")).is_err());
        let active = x.atom(b"_NET_ACTIVE_WINDOW").unwrap();
        c.change_property32(PropMode::REPLACE, x.root, active, AtomEnum::WINDOW, &[app])
            .unwrap()
            .check()
            .unwrap();
        c.set_input_focus(InputFocus::PARENT, editor, 0u32)
            .unwrap()
            .check()
            .unwrap();
        let mut paster =
            LinuxPaster::new(Default::default(), Default::default(), Default::default()).unwrap();
        let mut routes = crate::linux::destinations::parse(
            &Default::default(),
            "F6 | Browser@OpenCode\nF7 | crx_example | Ctrl+N | enter",
        )
        .unwrap();
        let received = || {
            c.get_input_focus().unwrap().reply().unwrap();
            let mut keys = Vec::new();
            while let Some(event) = c.poll_for_event().unwrap() {
                if let x11rb::protocol::Event::KeyPress(key) = event {
                    assert_eq!(key.event, editor);
                    keys.push(key.detail);
                }
            }
            keys
        };
        paster
            .deliver_to(&routes.remove(0), "editable draft")
            .unwrap();
        assert_eq!(received(), [x.control, x.v]);
        paster.deliver_to(&routes[0], "new session").unwrap();
        assert_eq!(
            received(),
            [
                x.control,
                x.keymap.keycode(b'n' as u32).unwrap(),
                x.control,
                x.v,
                x.keymap.keycode(XK_RETURN).unwrap()
            ]
        );
        x.target = Some(Target {
            active,
            window: app,
            title: Some("opencode".into()),
        });
        assert!(x.check_destination().is_ok());
        c.set_input_focus(InputFocus::PARENT, other, 0u32)
            .unwrap()
            .check()
            .unwrap();
        let enter = x.keymap.keycode(XK_RETURN).unwrap();
        assert!(
            x.send_keys(&[(KEY_PRESS, enter), (KEY_RELEASE, enter)])
                .is_err()
        );
    }
}

pub(super) struct Target {
    active: u32,
    window: Window,
    title: Option<String>,
}

impl X11Paster {
    fn atom(&self, name: &[u8]) -> Result<u32> {
        Ok(self.connection.intern_atom(false, name)?.reply()?.atom)
    }

    fn find_window(&self, class: &str, title: Option<&str>) -> Result<Window> {
        let reply = self
            .connection
            .get_property(
                false,
                self.root,
                self.atom(b"_NET_CLIENT_LIST_STACKING")?,
                AtomEnum::WINDOW,
                0,
                8192,
            )?
            .reply()?;
        let windows: Vec<_> = reply.value32().into_iter().flatten().collect();
        // EWMH order is bottom-to-top. Prefer the most recently raised
        // matching window; never broaden a missing instance to its browser.
        let title_atoms = title
            .map(|s| -> Result<_> {
                Ok((s, self.atom(b"_NET_WM_NAME")?, self.atom(b"UTF8_STRING")?))
            })
            .transpose()?;
        for w in windows.into_iter().rev() {
            let Ok(prop) = self
                .connection
                .get_property(false, w, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
                .reply()
            else {
                continue;
            };
            if prop
                .value
                .split(|b| *b == 0)
                .any(|s| s.eq_ignore_ascii_case(class.as_bytes()))
            {
                if let Some((needle, name, utf8)) = title_atoms {
                    let Ok(value) = self
                        .connection
                        .get_property(false, w, name, utf8, 0, 1024)?
                        .reply()
                    else {
                        continue;
                    };
                    if !String::from_utf8_lossy(&value.value)
                        .to_lowercase()
                        .contains(needle)
                    {
                        continue;
                    }
                }
                return Ok(w);
            }
        }
        Err(eyre!(
            "No open window matches {class}; use Paste last dictation to recover the text"
        ))
    }

    fn focused(&self, t: &Target) -> Result<bool> {
        let active = self
            .connection
            .get_property(false, self.root, t.active, AtomEnum::WINDOW, 0, 1)?
            .reply()?;
        if active.value32().and_then(|mut w| w.next()) != Some(t.window) {
            return Ok(false);
        }
        if let Some(title) = &t.title {
            let actual = self
                .connection
                .get_property(
                    false,
                    t.window,
                    self.atom(b"_NET_WM_NAME")?,
                    self.atom(b"UTF8_STRING")?,
                    0,
                    1024,
                )?
                .reply()?;
            if !String::from_utf8_lossy(&actual.value)
                .to_lowercase()
                .contains(title)
            {
                return Ok(false);
            }
        }
        let mut focus = self.connection.get_input_focus()?.reply()?.focus;
        for _ in 0..32 {
            if focus == t.window {
                return Ok(true);
            }
            if focus <= 1 || focus == self.root {
                break;
            }
            focus = self.connection.query_tree(focus)?.reply()?.parent;
        }
        Ok(false)
    }

    fn activate(&mut self, target: Target, stop: &AtomicBool) -> Result<()> {
        if self.focused(&target)? {
            self.target = Some(target);
            return Ok(());
        }
        let event = ClientMessageEvent::new(32, target.window, target.active, [2, 0, 0, 0, 0]);
        self.connection
            .send_event(
                false,
                self.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )?
            .check()?;
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
            if self.focused(&target)? {
                self.target = Some(target);
                return Ok(());
            }
            thread::sleep(Duration::from_millis(15));
        }
        Err(eyre!("Could not focus the destination window"))
    }

    pub(super) fn check_destination(&self) -> Result<()> {
        if let Some(target) = &self.target
            && !self.focused(target)?
        {
            return Err(eyre!(
                "The destination window lost focus; use Paste last dictation to recover the text"
            ));
        }
        Ok(())
    }

    fn send_shortcut(&mut self, chord: &LinuxHotkey) -> Result<()> {
        let mut codes = Vec::new();
        for (on, sym) in [
            (chord.control, XK_CONTROL_L),
            (chord.alt, XK_ALT_L),
            (chord.shift, XK_SHIFT_L),
            (chord.super_key, XK_SUPER_L),
        ] {
            if on {
                codes.push(self.keymap.keycode(sym)?);
            }
        }
        codes.push(self.keymap.keycode(chord.keysym()?.value)?);
        let sequence = codes
            .iter()
            .map(|&k| (KEY_PRESS, k))
            .chain(codes.iter().rev().map(|&k| (KEY_RELEASE, k)))
            .collect::<Vec<_>>();
        self.send_keys(&sequence)
    }
}
