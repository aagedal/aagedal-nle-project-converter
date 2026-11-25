#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backend;
mod theme;

use app::ConverterApp;
use gpui_kit::*;

gpui_kit::actions!(nle_gui, [Quit]);

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            theme::apply_app_theme(cx);

            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.set_menus(vec![Menu {
                name: "Aagedal NLE Project Converter".into(),
                items: vec![MenuItem::action("Quit", Quit)],
                disabled: false,
            }]);

            let bounds = Bounds::centered(None, size(px(1220.), px(840.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(960.), px(640.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Aagedal NLE Project Converter".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| ConverterApp::new(window, cx))
            })
            .expect("failed to open Aagedal NLE Project Converter window");

            cx.activate(true);
        });
}
