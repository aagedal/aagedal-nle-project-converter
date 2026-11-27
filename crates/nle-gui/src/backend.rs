use nle_core::{
    apply_relink_rules, flatten_nested_and_multicam_clips, ConversionOptions, ConversionReport,
    NleFormat, Project, ProjectInspection,
};
use nle_effects::{
    analyze_project_effects, apply_effect_decisions, EffectMappingPlan, EffectMappingPreset,
    UserEffectDecisions,
};
use std::path::Path;

pub const SUPPORTED_TARGETS: &[(NleFormat, &str)] = &[
    (NleFormat::DaVinciResolve, "DaVinci Resolve Native (.drp)"),
    (NleFormat::PremierePro, "Adobe Premiere Pro Native (.prproj)"),
    (NleFormat::FinalCutPro, "Final Cut Pro (.fcpxml)"),
    (NleFormat::OpenTimelineIo, "OpenTimelineIO (.otio)"),
    (NleFormat::Fcp7Xml, "FCP7 XML (.xml — Premiere / Resolve)"),
];

#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub project: Project,
    pub inspection: ProjectInspection,
    pub effect_plan: EffectMappingPlan,
}

pub fn load_project_from_disk(path: &Path) -> Result<Project, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "prproj" => nle_premiere::read_prproj_file(path).map_err(|e| e.to_string()),
        "drp" | "drt" => nle_resolve::read_drp_file(path).map_err(|e| e.to_string()),
        "fcpxml" | "fcpxmld" => nle_interchange::read_fcpxml_file(path).map_err(|e| e.to_string()),
        "fcpbundle" => nle_interchange::read_fcpbundle_file(path).map_err(|e| e.to_string()),
        _ => Err(format!(
            "Unsupported project file '.{ext}'. Supported inputs: .prproj (Premiere Pro), .drp/.drt (DaVinci Resolve), .fcpbundle/.fcpxml/.fcpxmld (Final Cut Pro)."
        )),
    }
}

pub fn effective_target_for(source_format: NleFormat, requested_target: NleFormat) -> NleFormat {
    if source_format == requested_target {
        match source_format {
            NleFormat::DaVinciResolve => NleFormat::PremierePro,
            _ => NleFormat::DaVinciResolve,
        }
    } else {
        requested_target
    }
}

pub fn load_and_inspect_project(
    path: &Path,
    requested_target: NleFormat,
) -> Result<LoadedProject, String> {
    let project = load_project_from_disk(path)?;
    let target = effective_target_for(project.source_format, requested_target);
    let inspection = ProjectInspection::from_project(&project);
    let effect_plan = analyze_project_effects(&project, &[], target);
    Ok(LoadedProject {
        project,
        inspection,
        effect_plan,
    })
}

pub fn load_demo_project(requested_target: NleFormat) -> Result<LoadedProject, String> {
    let xml = nle_premiere::sample_prproj_xml();
    let mut project =
        nle_premiere::parse_prproj_xml(&xml, "Studio_Demo_Commercial").map_err(|e| e.to_string())?;
    project.source_path = Some("Built-in Demo (.prproj)".to_string());
    let target = effective_target_for(project.source_format, requested_target);
    let inspection = ProjectInspection::from_project(&project);
    let effect_plan = analyze_project_effects(&project, &[], target);
    Ok(LoadedProject {
        project,
        inspection,
        effect_plan,
    })
}

pub fn run_conversion(
    source_project: &Project,
    output_path: &Path,
    mut options: ConversionOptions,
    decisions: &UserEffectDecisions,
) -> Result<ConversionReport, String> {
    let mut project = source_project.clone();

    apply_relink_rules(&mut project, &options.relink_rules);

    let mut pre_messages = Vec::new();
    if options.flatten_nested_and_multicam {
        let flat = flatten_nested_and_multicam_clips(&mut project);
        pre_messages.extend(flat.messages);
        options.flatten_nested_and_multicam = false;
    }

    let plan = analyze_project_effects(
        &project,
        &options.selected_sequence_ids,
        options.target_format,
    );
    let fx_summary = apply_effect_decisions(
        &mut project,
        &plan,
        decisions,
        options.inject_effect_review_markers,
    );
    if fx_summary.disable_lumetri_native {
        options.include_lumetri_grades = false;
    }

    let mut report = match options.target_format {
        NleFormat::PremierePro => {
            nle_premiere::write_prproj_file(&project, output_path, &options)
                .map_err(|e| e.to_string())?
        }
        NleFormat::DaVinciResolve => {
            nle_resolve::write_drp_file(&project, output_path, &options).map_err(|e| e.to_string())?
        }
        NleFormat::FinalCutPro => {
            nle_interchange::write_fcpxml_file(&project, output_path, &options)
                .map_err(|e| e.to_string())?
        }
        NleFormat::OpenTimelineIo => {
            nle_interchange::write_otio_file(&project, output_path, &options)
                .map_err(|e| e.to_string())?
        }
        NleFormat::Fcp7Xml => {
            nle_interchange::write_fcp7_xml_file(&project, output_path, &options)
                .map_err(|e| e.to_string())?
        }
        other => {
            return Err(format!(
                "Direct writing to {} is on the roadmap — please select Premiere Pro (.prproj), Resolve (.drp), Final Cut Pro (.fcpxml), OTIO (.otio), or FCP7 XML (.xml).",
                other.display_name()
            ));
        }
    };

    report.effect_markers_injected = fx_summary.markers_injected;
    pre_messages.append(&mut report.messages);
    pre_messages.extend(fx_summary.messages);
    report.messages = pre_messages;
    Ok(report)
}

pub fn save_effect_preset(path: &Path, preset: &EffectMappingPreset) -> Result<(), String> {
    let json = serde_json::to_string_pretty(preset).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_effect_preset(path: &Path) -> Result<EffectMappingPreset, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if let Ok(preset) = serde_json::from_str::<EffectMappingPreset>(&raw) {
        return Ok(preset);
    }
    let decisions = UserEffectDecisions::from_json_str(&raw).map_err(|e| e.to_string())?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported Decisions")
        .to_string();
    Ok(EffectMappingPreset {
        name,
        source_format: NleFormat::PremierePro,
        target_format: NleFormat::DaVinciResolve,
        decisions,
    })
}

/// Count how many media assets in `project` have paths matching `from_prefix`
/// (using the same separator-normalized prefix matching as `nle_core::apply_relink_rules`).
pub fn count_matching_relink_paths(project: &Project, from_prefix: &str) -> (usize, usize) {
    let needle = from_prefix.trim().replace('\\', "/");
    let total = project
        .media_assets
        .iter()
        .filter(|a| !a.path.is_empty())
        .count();
    if needle.is_empty() {
        return (0, total);
    }
    let needle_lower = needle.to_ascii_lowercase();
    let matched = project
        .media_assets
        .iter()
        .filter(|a| {
            !a.path.is_empty()
                && a.path
                    .replace('\\', "/")
                    .to_ascii_lowercase()
                    .starts_with(&needle_lower)
        })
        .count();
    (matched, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nle_effects::MappingAction;

    #[test]
    fn effective_target_avoids_same_format() {
        assert_eq!(
            effective_target_for(NleFormat::PremierePro, NleFormat::PremierePro),
            NleFormat::DaVinciResolve
        );
        assert_eq!(
            effective_target_for(NleFormat::DaVinciResolve, NleFormat::DaVinciResolve),
            NleFormat::PremierePro
        );
        assert_eq!(
            effective_target_for(NleFormat::PremierePro, NleFormat::OpenTimelineIo),
            NleFormat::OpenTimelineIo
        );
    }

    #[test]
    fn demo_load_convert_and_preset_roundtrip() {
        let loaded = load_demo_project(NleFormat::DaVinciResolve).expect("load demo project");
        assert_eq!(loaded.inspection.sequences.len(), 1);
        assert!(!loaded.effect_plan.effects.is_empty());

        let dir = std::env::temp_dir().join("aagedal_nle_gui_backend_test");
        let _ = std::fs::create_dir_all(&dir);

        let drp_path = dir.join("gui_demo.drp");
        let opts = ConversionOptions {
            target_format: NleFormat::DaVinciResolve,
            project_name: Some(loaded.inspection.name.clone()),
            selected_sequence_ids: loaded
                .inspection
                .sequences
                .iter()
                .map(|s| s.id.clone())
                .collect(),
            ..Default::default()
        };
        let mut decisions = UserEffectDecisions::default();
        for fx in &loaded.effect_plan.effects {
            decisions
                .actions
                .insert(fx.key.clone(), fx.default_action.clone());
        }
        let report =
            run_conversion(&loaded.project, &drp_path, opts, &decisions).expect("convert to drp");
        assert_eq!(report.sequences_converted, 1);
        assert!(drp_path.exists());

        // Reload the written .drp through the same disk loader
        let reloaded = load_and_inspect_project(&drp_path, NleFormat::DaVinciResolve)
            .expect("reload written drp");
        assert_eq!(reloaded.inspection.source_format, NleFormat::DaVinciResolve);
        assert_eq!(reloaded.effect_plan.target_format, NleFormat::PremierePro);

        // Save and reload an effect mapping preset
        let preset_path = dir.join("preset.json");
        let mut preset_decisions = UserEffectDecisions::default();
        preset_decisions
            .actions
            .insert("ae.adbe subspacestabilizer".into(), MappingAction::Skip);
        let preset = EffectMappingPreset {
            name: "Studio Preset".into(),
            source_format: NleFormat::PremierePro,
            target_format: NleFormat::DaVinciResolve,
            decisions: preset_decisions,
        };
        save_effect_preset(&preset_path, &preset).expect("save preset");
        let loaded_preset = load_effect_preset(&preset_path).expect("load preset");
        assert_eq!(loaded_preset.name, "Studio Preset");
        assert_eq!(
            loaded_preset
                .decisions
                .actions
                .get("ae.adbe subspacestabilizer"),
            Some(&MappingAction::Skip)
        );

        // Also verify raw UserEffectDecisions JSON loads cleanly and relink prefix matching works
        let raw_path = dir.join("raw_decisions.json");
        std::fs::write(
            &raw_path,
            serde_json::to_string_pretty(&loaded_preset.decisions).unwrap(),
        )
        .expect("write raw decisions");
        let loaded_raw = load_effect_preset(&raw_path).expect("load raw decisions");
        assert_eq!(
            loaded_raw.decisions.actions.get("ae.adbe subspacestabilizer"),
            Some(&MappingAction::Skip)
        );

        let (matched, total) = count_matching_relink_paths(&loaded.project, "/Media");
        assert!(total > 0);
        assert!(matched > 0);
        let (zero_matched, _) = count_matching_relink_paths(&loaded.project, "/NonExistentDrive");
        assert_eq!(zero_matched, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
