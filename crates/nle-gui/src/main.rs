#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod actions;
mod app;
mod assets;
mod backend;
mod settings;
mod settings_view;
mod theme;

use actions::{
    ApplyRecommendedEffects, CloseProject, ConvertProject, CopyConversionLog, DeselectAllSequences,
    HideApp, HideOtherApps, LoadDemoProject, LoadEffectPreset, OpenAbout, OpenProject,
    OpenSettings, OpenShortcuts, Quit, RevealOutput, SaveEffectPreset, SelectAllSequences,
};
use app::ConverterApp;
use gpui_kit::*;
use settings::{AppSettings, AppSettingsGlobal};
use settings_view::SettingsSection;

/// Holds the primary Converter window so global menu actions still work when an auxiliary
/// window (such as About) is focused.
struct MainWindowRef {
    handle: AnyWindowHandle,
    view: Entity<ConverterApp>,
}

impl Global for MainWindowRef {}

fn dispatch_to_main_window(
    cx: &mut App,
    f: impl FnOnce(&mut ConverterApp, &mut Window, &mut Context<ConverterApp>),
) {
    let Some(main) = cx.try_global::<MainWindowRef>() else {
        return;
    };
    let handle = main.handle;
    let view = main.view.clone();
    let _ = cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        view.update(cx, |app, cx| f(app, window, cx));
    });
}

fn main() {
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            assets::install_platform_app_icon();
            theme::apply_app_theme(cx);
            cx.set_global(AppSettingsGlobal(AppSettings::load()));

            // Global application actions
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &HideApp, cx| cx.hide());
            cx.on_action(|_: &HideOtherApps, cx| cx.hide_other_apps());
            cx.on_action(|_: &OpenAbout, cx| about::open_about_window(cx));

            // Global fallbacks for main-window actions (e.g. triggered from menu bar while About is focused)
            cx.on_action(|_: &OpenSettings, cx| {
                dispatch_to_main_window(cx, |app, _, cx| app.toggle_settings(cx));
            });
            cx.on_action(|_: &OpenShortcuts, cx| {
                dispatch_to_main_window(cx, |app, _, cx| {
                    app.open_settings_section(SettingsSection::Shortcuts, cx)
                });
            });
            cx.on_action(|_: &OpenProject, cx| {
                dispatch_to_main_window(cx, |app, window, cx| app.trigger_open_file(window, cx));
            });
            cx.on_action(|_: &LoadDemoProject, cx| {
                dispatch_to_main_window(cx, |app, window, cx| app.trigger_load_demo(window, cx));
            });
            cx.on_action(|_: &ConvertProject, cx| {
                dispatch_to_main_window(cx, |app, window, cx| app.trigger_convert(window, cx));
            });
            cx.on_action(|_: &CloseProject, cx| {
                dispatch_to_main_window(cx, |app, _, cx| app.trigger_close_project(cx));
            });
            cx.on_action(|_: &LoadEffectPreset, cx| {
                dispatch_to_main_window(cx, |app, window, cx| app.trigger_load_preset(window, cx));
            });
            cx.on_action(|_: &SaveEffectPreset, cx| {
                dispatch_to_main_window(cx, |app, window, cx| app.trigger_save_preset(window, cx));
            });
            cx.on_action(|_: &SelectAllSequences, cx| {
                dispatch_to_main_window(cx, |app, window, cx| {
                    app.trigger_select_all_sequences(window, cx)
                });
            });
            cx.on_action(|_: &DeselectAllSequences, cx| {
                dispatch_to_main_window(cx, |app, window, cx| {
                    app.trigger_deselect_all_sequences(window, cx)
                });
            });
            cx.on_action(|_: &ApplyRecommendedEffects, cx| {
                dispatch_to_main_window(cx, |app, window, cx| {
                    app.trigger_apply_recommended_effects(window, cx)
                });
            });
            cx.on_action(|_: &RevealOutput, cx| {
                dispatch_to_main_window(cx, |app, _, cx| app.trigger_reveal_output(cx));
            });
            cx.on_action(|_: &CopyConversionLog, cx| {
                dispatch_to_main_window(cx, |app, _, cx| app.trigger_copy_conversion_log(cx));
            });

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            actions::bind_default_keys(cx);
            cx.set_menus(actions::build_app_menus());

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

            let (handle, view) = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| ConverterApp::new(window, cx))
            })
            .expect("failed to open Aagedal NLE Project Converter window");

            cx.set_global(MainWindowRef { handle, view });

            cx.activate(true);
        });
}
