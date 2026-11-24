use nle_core::{
    apply_relink_rules, ConversionOptions, ConversionReport, NleFormat, Project, ProjectInspection,
};
use nle_effects::{
    analyze_project_effects, apply_effect_decisions, EffectMappingPlan, EffectMappingPreset,
    UserEffectDecisions,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

pub struct AppState {
    pub current_project: Mutex<Option<Project>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedProjectPayload {
    pub inspection: ProjectInspection,
    pub effect_plan: EffectMappingPlan,
}

fn load_project_from_disk(path: &Path) -> Result<Project, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "prproj" => nle_premiere::read_prproj_file(path).map_err(|e| e.to_string()),
        "drp" | "drt" => nle_resolve::read_drp_file(path).map_err(|e| e.to_string()),
        _ => Err(format!(
            "Unsupported project file '.{ext}'. Supported inputs: .prproj (Premiere Pro), .drp (DaVinci Resolve)."
        )),
    }
}

fn build_loaded_payload(project: &Project, target_format: NleFormat) -> LoadedProjectPayload {
    let inspection = ProjectInspection::from_project(project);
    let effect_plan = analyze_project_effects(project, &[], target_format);
    LoadedProjectPayload {
        inspection,
        effect_plan,
    }
}

#[tauri::command]
async fn pick_and_inspect_project(
    app: AppHandle,
    state: State<'_, AppState>,
    target_format: NleFormat,
) -> Result<Option<LoadedProjectPayload>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Open NLE Project File")
        .add_filter("NLE Projects", &["prproj", "drp", "drt"])
        .add_filter("Premiere Pro Project", &["prproj"])
        .add_filter("DaVinci Resolve Project", &["drp", "drt"])
        .blocking_pick_file();

    let Some(file_path) = picked else {
        return Ok(None);
    };
    let path_buf = file_path
        .into_path()
        .map_err(|e| format!("Invalid file path: {e}"))?;

    let project = load_project_from_disk(&path_buf)?;
    let effective_target = if project.source_format == target_format {
        match project.source_format {
            NleFormat::DaVinciResolve => NleFormat::FinalCutPro,
            _ => NleFormat::DaVinciResolve,
        }
    } else {
        target_format
    };
    let payload = build_loaded_payload(&project, effective_target);
    *state.current_project.lock().unwrap() = Some(project);
    Ok(Some(payload))
}

#[tauri::command]
async fn inspect_project_path(
    state: State<'_, AppState>,
    path: String,
    target_format: NleFormat,
) -> Result<LoadedProjectPayload, String> {
    let project = load_project_from_disk(Path::new(&path))?;
    let payload = build_loaded_payload(&project, target_format);
    *state.current_project.lock().unwrap() = Some(project);
    Ok(payload)
}

#[tauri::command]
async fn load_demo_project(
    state: State<'_, AppState>,
    target_format: NleFormat,
) -> Result<LoadedProjectPayload, String> {
    let xml = nle_premiere::sample_prproj_xml();
    let mut project =
        nle_premiere::parse_prproj_xml(&xml, "Studio_Demo_Commercial").map_err(|e| e.to_string())?;
    project.source_path = Some("Built-in Demo (.prproj)".to_string());
    let payload = build_loaded_payload(&project, target_format);
    *state.current_project.lock().unwrap() = Some(project);
    Ok(payload)
}

#[tauri::command]
async fn reanalyze_effects(
    state: State<'_, AppState>,
    selected_sequence_ids: Vec<String>,
    target_format: NleFormat,
) -> Result<EffectMappingPlan, String> {
    let guard = state.current_project.lock().unwrap();
    let project = guard
        .as_ref()
        .ok_or_else(|| "No project is currently loaded.".to_string())?;
    Ok(analyze_project_effects(
        project,
        &selected_sequence_ids,
        target_format,
    ))
}

#[tauri::command]
async fn pick_output_and_convert(
    app: AppHandle,
    state: State<'_, AppState>,
    mut options: ConversionOptions,
    decisions: UserEffectDecisions,
) -> Result<Option<ConversionReport>, String> {
    let mut project = {
        let guard = state.current_project.lock().unwrap();
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "No project is currently loaded.".to_string())?
    };

    let ext = options.target_format.default_extension();
    let default_name = format!(
        "{}.{}",
        options
            .project_name
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&project.name),
        ext
    );

    let saved = app
        .dialog()
        .file()
        .set_title("Save Converted Project")
        .set_file_name(&default_name)
        .add_filter(options.target_format.display_name(), &[ext])
        .blocking_save_file();

    let Some(file_path) = saved else {
        return Ok(None);
    };
    let output_path: PathBuf = file_path
        .into_path()
        .map_err(|e| format!("Invalid output path: {e}"))?;

    // Apply path relinking
    apply_relink_rules(&mut project, &options.relink_rules);

    // Apply user effect mapping decisions
    let plan = analyze_project_effects(
        &project,
        &options.selected_sequence_ids,
        options.target_format,
    );
    let fx_summary = apply_effect_decisions(
        &mut project,
        &plan,
        &decisions,
        options.inject_effect_review_markers,
    );
    if fx_summary.disable_lumetri_native {
        options.include_lumetri_grades = false;
    }

    let mut report = match options.target_format {
        NleFormat::DaVinciResolve => {
            nle_resolve::write_drp_file(&project, &output_path, &options)
                .map_err(|e| e.to_string())?
        }
        NleFormat::FinalCutPro => {
            nle_interchange::write_fcpxml_file(&project, &output_path, &options)
                .map_err(|e| e.to_string())?
        }
        NleFormat::OpenTimelineIo => {
            nle_interchange::write_otio_file(&project, &output_path, &options)
                .map_err(|e| e.to_string())?
        }
        NleFormat::Fcp7Xml => {
            nle_interchange::write_fcp7_xml_file(&project, &output_path, &options)
                .map_err(|e| e.to_string())?
        }
        other => {
            return Err(format!(
                "Direct writing to {} is on the roadmap — please select Resolve (.drp), Final Cut Pro (.fcpxml), OTIO (.otio), or FCP7 XML (.xml).",
                other.display_name()
            ));
        }
    };

    report.effect_markers_injected = fx_summary.markers_injected;
    report.messages.extend(fx_summary.messages);
    Ok(Some(report))
}

#[tauri::command]
async fn save_effect_preset(
    app: AppHandle,
    preset: EffectMappingPreset,
) -> Result<Option<String>, String> {
    let saved = app
        .dialog()
        .file()
        .set_title("Save Effect Mapping Preset")
        .set_file_name("nle-effect-preset.json")
        .add_filter("Effect Preset JSON", &["json"])
        .blocking_save_file();

    let Some(fp) = saved else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&preset).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn load_effect_preset(app: AppHandle) -> Result<Option<EffectMappingPreset>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Load Effect Mapping Preset")
        .add_filter("Effect Preset JSON", &["json"])
        .blocking_pick_file();

    let Some(fp) = picked else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let preset: EffectMappingPreset = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(Some(preset))
}

#[tauri::command]
async fn reveal_in_file_manager(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(Path::new(&path))
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            current_project: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            pick_and_inspect_project,
            inspect_project_path,
            load_demo_project,
            reanalyze_effects,
            pick_output_and_convert,
            save_effect_preset,
            load_effect_preset,
            reveal_in_file_manager,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Aagedal NLE Project Converter");
}
