use rune::{El, col, text};
use rune_kit::TextArea;

use super::{Paint, Ui, tr};
use crate::rune_ui::{controls::DESCRIPTION, lists::small_button};

pub(super) fn body(ui: &Ui, p: Paint) -> El {
    let edit = &ui.lab_store.destinations;
    let valid = crate::linux::destinations::parse(&ui.settings, &edit.text);
    let help = tr("Shortcut | Window class | Optional key before paste | Optional enter");
    let mut body = col()
        .gap(6.0)
        .child(text(help).font(DESCRIPTION).color(p.muted()).wrap())
        .child(
            TextArea::new(edit)
                .placeholder("F6 | MyApp\nF7 | MyApp | Ctrl+N | enter")
                .rows(4)
                .width(None)
                .on_edit(|ui: &mut Ui| &mut ui.lab_store.destinations)
                .build(p.k)
                .key("destination-bindings"),
        );
    if let Err(error) = valid {
        body = body.child(
            text(error.to_string())
                .font(DESCRIPTION)
                .color(p.danger())
                .wrap(),
        );
    } else if edit.text != ui.settings.labs.destination_bindings {
        body = body.child(small_button(p, "save-destinations", tr("Save"), |ui, _| {
            let saved = ui.lab_store.destinations.text.clone();
            ui.update(|settings| settings.labs.destination_bindings = saved);
        }));
    }
    body
}
