use crate::app::ChatApp;
use gpui::{component::TitleBar,
           *};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TitlebarMode {
    Native,
    #[default]
    Gpui,
}

impl TitlebarMode {
    pub(super) fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path).as_deref().map(str::trim) {
            | Ok("native") => Self::Native,
            | _ => Self::Gpui,
        }
    }

    pub(super) fn window_options(self) -> WindowOptions {
        let mut options = match self {
            | Self::Native => WindowOptions::default(),
            | Self::Gpui => TitleBar::window_options(),
        };
        options.titlebar.get_or_insert_with(TitlebarOptions::default).title = Some("Gpui Agent".into());
        options.window_min_size = Some(size(px(500.), px(360.)));
        options.window_decorations = Some(match self {
            | Self::Native => WindowDecorations::Server,
            | Self::Gpui => WindowDecorations::Client,
        });
        options
    }
}

impl ChatApp {
    pub(super) fn set_titlebar_mode(&mut self, mode: TitlebarMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.titlebar_mode == mode {
            return;
        }
        let view = cx.entity();
        let old_window = window.window_handle();
        let options = WindowOptions {
            window_bounds: Some(window.window_bounds()),
            display_id: window.display(cx).map(|display| display.id()),
            ..mode.window_options()
        };
        // Defer window recreation so rendering can borrow this entity.
        cx.defer(move |cx| {
            match gpui::open_window(options, cx, |window, cx| {
                view.update(cx, |s, cx| {
                    s.titlebar_mode = mode;
                    // Rebuild window-bound input subscriptions from the drafts.
                    s.inputs.clear();
                    s.modal_focus = None;
                    s.focus.focus(window, cx);
                });
                view.clone()
            }) {
                | Ok(_) => {
                    view.update(cx, |s, cx| {
                        if let Some(path) = &s.language_path {
                            let path = path.with_file_name("ui-titlebar");
                            let result = std::fs::create_dir_all(path.parent().expect("preference directory"))
                                .and_then(|_| std::fs::write(path, if mode == TitlebarMode::Native { "native" } else { "gpui" }));
                            if let Err(error) = result {
                                s.conversations[s.selected].error = Some(format!("Cannot save title bar preference: {error}"));
                            }
                        }
                        cx.notify();
                    });
                    let _ = old_window.update(cx, |_, window, _| window.remove_window());
                    cx.activate(true);
                },
                | Err(error) => view.update(cx, |s, cx| {
                    s.conversations[s.selected].error = Some(format!("Cannot change title bar: {error}"));
                    cx.notify();
                }),
            }
        });
    }
}
