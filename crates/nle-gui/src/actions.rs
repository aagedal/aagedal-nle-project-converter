use gpui_kit::component::input;
use gpui_kit::{App, KeyBinding, Menu, MenuItem, OsAction};
#[cfg(target_os = "macos")]
use gpui_kit::SystemMenuType;

gpui_kit::actions!(
    nle_gui,
    [
        Quit,
        HideApp,
        HideOtherApps,
        MinimizeWindow,
        ZoomWindow,
        CloseWindow,
        DismissOverlay,
        OpenAbout,
        OpenSettings,
        OpenShortcuts,
        OpenProject,
        OpenProjectFolder,
        LoadDemoProject,
        ConvertProject,
        CloseProject,
        LoadEffectPreset,
        SaveEffectPreset,
        SelectAllSequences,
        DeselectAllSequences,
        ApplyRecommendedEffects,
        RevealOutput,
        CopyConversionLog,
    ]
);

/// Platform-appropriate primary modifier keystroke helper (`cmd-*` on macOS, `ctrl-*` elsewhere).
macro_rules! primary_key {
    ($suffix:literal) => {{
        #[cfg(target_os = "macos")]
        {
            concat!("cmd-", $suffix)
        }
        #[cfg(not(target_os = "macos"))]
        {
            concat!("ctrl-", $suffix)
        }
    }};
}

#[derive(Clone, Copy, Debug)]
pub struct ShortcutEntry {
    pub category: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub keystroke: &'static str,
}

/// Single source of truth for displaying shortcuts in the Settings → Keyboard Shortcuts page.
pub const SHORTCUT_CATALOG: &[ShortcutEntry] = &[
    // File & Project
    ShortcutEntry {
        category: "File & Project",
        label: "Open Project",
        description: "Open a .prproj, .drp/.drt, .fcpbundle/.fcpxml, .otio/.otioz, .xml, .edl, or .aaf file",
        keystroke: primary_key!("o"),
    },
    ShortcutEntry {
        category: "File & Project",
        label: "Open Project Bundle / Folder",
        description: "Select a .fcpbundle, .fcpxmld, or .otiod package folder directly",
        keystroke: primary_key!("shift-o"),
    },
    ShortcutEntry {
        category: "File & Project",
        label: "Load Demo Project",
        description: "Load the built-in Studio Demo Premiere Pro project",
        keystroke: primary_key!("shift-d"),
    },
    ShortcutEntry {
        category: "File & Project",
        label: "Convert & Save",
        description: "Convert selected sequences and save to the target NLE format",
        keystroke: primary_key!("s"),
    },
    ShortcutEntry {
        category: "File & Project",
        label: "Export Project (Alias)",
        description: "Alternate shortcut to convert and save the active project",
        keystroke: primary_key!("e"),
    },
    ShortcutEntry {
        category: "File & Project",
        label: "Reveal Last Output",
        description: "Highlight the last converted file in Finder / File Explorer",
        keystroke: primary_key!("shift-r"),
    },
    ShortcutEntry {
        category: "File & Project",
        label: "Close Project",
        description: "Unload the current project and return to the start screen",
        keystroke: primary_key!("shift-w"),
    },
    // Effects & Sequences
    ShortcutEntry {
        category: "Sequences & Effects",
        label: "Select All Sequences",
        description: "Check every sequence in the loaded project for conversion",
        keystroke: primary_key!("shift-a"),
    },
    ShortcutEntry {
        category: "Sequences & Effects",
        label: "Deselect All Sequences",
        description: "Uncheck all sequences in the loaded project",
        keystroke: primary_key!("alt-a"),
    },
    ShortcutEntry {
        category: "Sequences & Effects",
        label: "Apply Recommended Effects",
        description: "Reset all detected effect mappings to their recommended defaults",
        keystroke: primary_key!("alt-r"),
    },
    ShortcutEntry {
        category: "Sequences & Effects",
        label: "Load Effect Preset",
        description: "Import saved effect mapping decisions from a JSON preset",
        keystroke: primary_key!("alt-o"),
    },
    ShortcutEntry {
        category: "Sequences & Effects",
        label: "Save Effect Preset",
        description: "Export current effect mapping decisions to a JSON preset",
        keystroke: primary_key!("alt-s"),
    },
    ShortcutEntry {
        category: "Sequences & Effects",
        label: "Copy Conversion Log",
        description: "Copy the conversion report log messages to the clipboard",
        keystroke: primary_key!("shift-c"),
    },
    // Application & Navigation
    ShortcutEntry {
        category: "Application & Window",
        label: "Settings",
        description: "Open or toggle the Settings page",
        keystroke: primary_key!(","),
    },
    ShortcutEntry {
        category: "Application & Window",
        label: "Keyboard Shortcuts",
        description: "Jump directly to the Keyboard Shortcuts reference",
        keystroke: primary_key!("/"),
    },
    ShortcutEntry {
        category: "Application & Window",
        label: "Back / Dismiss",
        description: "Exit Settings, close About window, or dismiss status banners",
        keystroke: "escape",
    },
    ShortcutEntry {
        category: "Application & Window",
        label: "Close Window",
        description: "Close the active window",
        keystroke: primary_key!("w"),
    },
    ShortcutEntry {
        category: "Application & Window",
        label: "Minimize Window",
        description: "Minimize the active window",
        keystroke: primary_key!("m"),
    },
    ShortcutEntry {
        category: "Application & Window",
        label: "Quit",
        description: "Quit Aagedal NLE Project Converter",
        keystroke: primary_key!("q"),
    },
];

pub const OPEN_PROJECT_KEYSTROKE: &str = primary_key!("o");
pub const CONVERT_KEYSTROKE: &str = primary_key!("s");
pub const SETTINGS_KEYSTROKE: &str = primary_key!(",");

pub fn bind_default_keys(cx: &mut App) {
    let mut bindings = vec![
        KeyBinding::new(primary_key!("q"), Quit, None),
        KeyBinding::new(primary_key!(","), OpenSettings, None),
        KeyBinding::new(primary_key!("/"), OpenShortcuts, None),
        KeyBinding::new(primary_key!("o"), OpenProject, None),
        KeyBinding::new(primary_key!("shift-o"), OpenProjectFolder, None),
        KeyBinding::new(primary_key!("shift-d"), LoadDemoProject, None),
        KeyBinding::new(primary_key!("s"), ConvertProject, None),
        KeyBinding::new(primary_key!("e"), ConvertProject, None),
        KeyBinding::new(primary_key!("shift-w"), CloseProject, None),
        KeyBinding::new(primary_key!("alt-o"), LoadEffectPreset, None),
        KeyBinding::new(primary_key!("alt-s"), SaveEffectPreset, None),
        KeyBinding::new(primary_key!("shift-a"), SelectAllSequences, None),
        KeyBinding::new(primary_key!("alt-a"), DeselectAllSequences, None),
        KeyBinding::new(primary_key!("alt-r"), ApplyRecommendedEffects, None),
        KeyBinding::new(primary_key!("shift-r"), RevealOutput, None),
        KeyBinding::new(primary_key!("shift-c"), CopyConversionLog, None),
        KeyBinding::new(primary_key!("w"), CloseWindow, None),
        KeyBinding::new(primary_key!("m"), MinimizeWindow, None),
        KeyBinding::new("escape", DismissOverlay, None),
    ];

    #[cfg(target_os = "macos")]
    {
        bindings.push(KeyBinding::new("cmd-h", HideApp, None));
        bindings.push(KeyBinding::new("cmd-alt-h", HideOtherApps, None));
    }

    cx.bind_keys(bindings);
}

pub fn build_app_menus() -> Vec<Menu> {
    let mut app_menu_items = vec![
        MenuItem::action("About Aagedal NLE Project Converter", OpenAbout),
        MenuItem::separator(),
        MenuItem::action("Settings...", OpenSettings),
    ];

    #[cfg(target_os = "macos")]
    {
        app_menu_items.push(MenuItem::separator());
        app_menu_items.push(MenuItem::os_submenu("Services", SystemMenuType::Services));
        app_menu_items.push(MenuItem::separator());
        app_menu_items.push(MenuItem::action(
            "Hide Aagedal NLE Project Converter",
            HideApp,
        ));
        app_menu_items.push(MenuItem::action("Hide Others", HideOtherApps));
    }

    app_menu_items.push(MenuItem::separator());
    app_menu_items.push(MenuItem::action("Quit Aagedal NLE Project Converter", Quit));

    vec![
        Menu {
            name: "Aagedal NLE Project Converter".into(),
            items: app_menu_items,
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open Project...", OpenProject),
                MenuItem::action("Open Project Bundle / Folder...", OpenProjectFolder),
                MenuItem::action("Load Demo Project", LoadDemoProject),
                MenuItem::separator(),
                MenuItem::action("Convert & Save...", ConvertProject),
                MenuItem::action("Reveal Last Output", RevealOutput),
                MenuItem::separator(),
                MenuItem::action("Load Effect Mapping Preset...", LoadEffectPreset),
                MenuItem::action("Save Effect Mapping Preset...", SaveEffectPreset),
                MenuItem::separator(),
                MenuItem::action("Close Project", CloseProject),
                MenuItem::action("Close Window", CloseWindow),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Copy Conversion Log", CopyConversionLog),
            ],
            disabled: false,
        },
        Menu {
            name: "Project".into(),
            items: vec![
                MenuItem::action("Select All Sequences", SelectAllSequences),
                MenuItem::action("Deselect All Sequences", DeselectAllSequences),
                MenuItem::separator(),
                MenuItem::action(
                    "Apply Recommended Effect Mappings",
                    ApplyRecommendedEffects,
                ),
            ],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", MinimizeWindow),
                MenuItem::action("Zoom", ZoomWindow),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ],
            disabled: false,
        },
        Menu {
            name: "Help".into(),
            items: vec![
                MenuItem::action("Keyboard Shortcuts", OpenShortcuts),
                MenuItem::separator(),
                MenuItem::action("About Aagedal NLE Project Converter", OpenAbout),
            ],
            disabled: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Keystroke;

    #[test]
    fn shortcut_catalog_keystrokes_parse_cleanly() {
        for entry in SHORTCUT_CATALOG {
            Keystroke::parse(entry.keystroke)
                .unwrap_or_else(|e| panic!("invalid keystroke for {}: {e}", entry.label));
        }
        Keystroke::parse(OPEN_PROJECT_KEYSTROKE).unwrap();
        Keystroke::parse(CONVERT_KEYSTROKE).unwrap();
        Keystroke::parse(SETTINGS_KEYSTROKE).unwrap();
    }
}
