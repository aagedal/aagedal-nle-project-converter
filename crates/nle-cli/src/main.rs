use clap::{Parser, Subcommand};
use nle_core::{
    flatten_nested_and_multicam_clips, ConversionOptions, NleFormat, Project, ProjectInspection,
    SequenceKind,
};
use nle_effects::{
    analyze_project_effects, apply_effect_decisions, EffectMappingPreset, InstalledPluginRegistry,
    UserEffectDecisions,
};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "nle-convert",
    about = "Direct open-source NLE project converter (Premiere Pro ↔ DaVinci Resolve, FCPXML, OTIO)"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect a .prproj, .drp, .fcpxml/.fcpxmld, or .fcpbundle project and list sequences, media, and detected effects
    Inspect {
        input: PathBuf,
        #[arg(long, default_value = "drp")]
        target: String,
        /// Emit machine-readable JSON inspection and effect plan output
        #[arg(long)]
        json: bool,
        /// Write an editable EffectMappingPreset JSON file with the recommended defaults for this project
        #[arg(long)]
        save_preset: Option<PathBuf>,
    },
    /// Convert a .prproj, .drp, .fcpxml/.fcpxmld, or .fcpbundle directly to .prproj, .drp, .fcpxml, .otio, or .xml
    Convert {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Optional JSON effect mapping decisions or GUI-saved EffectMappingPreset file
        #[arg(long)]
        preset: Option<PathBuf>,
        /// Optional path relink rule in FROM=TO format
        #[arg(long)]
        relink: Vec<String>,
        /// Optional output project name override
        #[arg(long)]
        project_name: Option<String>,
        /// Optional sequence ID or exact name to convert (repeatable; defaults to all sequences)
        #[arg(long = "sequence")]
        sequences: Vec<String>,
        /// Flatten nested sequences, compound clips, and multicam cuts into source media clips
        #[arg(long)]
        flatten_nested: bool,
        /// Do not recreate project bins / Media Pool folders
        #[arg(long)]
        no_bins: bool,
        /// Do not include unplaced project bin media assets
        #[arg(long)]
        no_unused_media: bool,
        /// Do not preserve timeline markers
        #[arg(long)]
        no_markers: bool,
        /// Do not preserve clip label colors
        #[arg(long)]
        no_clip_colors: bool,
        /// Do not translate Lumetri / Resolve primary color grades
        #[arg(long)]
        no_grades: bool,
        /// Do not inject review markers for mapped/unmapped effects
        #[arg(long)]
        no_fx_markers: bool,
        /// Emit machine-readable JSON ConversionReport output
        #[arg(long)]
        json: bool,
    },
    /// Generate a sample .prproj file containing bins, Lumetri, Motion, and app-exclusive effects
    Demo {
        output: PathBuf,
    },
    /// Scan locally installed third-party NLE plugins (Premiere MediaCore, Resolve OFX, FCP FxPlug, AU/VST3)
    Plugins {
        /// Show all installed third-party plugins (by default shows plugins installed in 2+ NLEs)
        #[arg(long)]
        all: bool,
        /// Emit machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
}

fn load_project(path: &Path) -> Result<Project, Box<dyn std::error::Error>> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "prproj" => Ok(nle_premiere::read_prproj_file(path)?),
        "drp" | "drt" => Ok(nle_resolve::read_drp_file(path)?),
        "fcpxml" | "fcpxmld" => Ok(nle_interchange::read_fcpxml_file(path)?),
        "fcpbundle" => Ok(nle_interchange::read_fcpbundle_file(path)?),
        _ => Err(format!(
            "Unsupported input format '.{ext}'. Supported: .prproj, .drp, .drt, .fcpxml, .fcpxmld, .fcpbundle"
        )
        .into()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Demo { output } => {
            nle_premiere::write_sample_prproj(&output)?;
            println!("Wrote demo Premiere Pro project to {}", output.display());
        }
        Commands::Plugins { all, json } => {
            let reg = InstalledPluginRegistry::scan_system();
            let min_nles = if all { 1 } else { 2 };
            let groups = reg.grouped_plugins(min_nles);
            if json {
                println!("{}", serde_json::to_string_pretty(&groups)?);
                return Ok(());
            }
            if groups.is_empty() {
                if all {
                    println!("No third-party NLE plugins detected in standard system folders.");
                } else {
                    println!(
                        "No cross-NLE third-party plugins (installed in 2+ NLEs) detected. Run with --all to list single-NLE plugins."
                    );
                }
                return Ok(());
            }
            println!(
                "Detected {} {} third-party plugin(s) ({} total installations scanned):",
                groups.len(),
                if all { "installed" } else { "cross-NLE" },
                reg.plugins().len()
            );
            for g in &groups {
                println!("  • {} [{:?}]", g.name, g.media_kind);
                for inst in &g.installations {
                    let ver = inst
                        .version
                        .as_deref()
                        .map(|v| format!(" v{v}"))
                        .unwrap_or_default();
                    println!(
                        "      - {} ({}{ver}): {}",
                        inst.target_nle.short_name(),
                        inst.format.short_label(),
                        inst.native_id
                    );
                }
            }
        }
        Commands::Inspect {
            input,
            target,
            json,
            save_preset,
        } => {
            let project = load_project(&input)?;
            let inspection = ProjectInspection::from_project(&project);
            let target_fmt = NleFormat::from_extension(&target).ok_or_else(|| {
                format!(
                    "Unsupported target format '{target}'. Supported: drp, prproj, fcpxml, otio, xml"
                )
            })?;
            let plan = analyze_project_effects(&project, &[], target_fmt);

            if let Some(preset_path) = save_preset {
                let mut actions = std::collections::BTreeMap::new();
                for fx in &plan.effects {
                    actions.insert(fx.key.clone(), fx.default_action.clone());
                }
                let preset = EffectMappingPreset {
                    name: format!("{} Mapping Preset", inspection.name),
                    source_format: inspection.source_format,
                    target_format: target_fmt,
                    decisions: UserEffectDecisions { actions },
                };
                if let Some(parent) = preset_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(&preset_path, serde_json::to_string_pretty(&preset)?)?;
                if !json {
                    println!("Saved effect mapping preset to {}", preset_path.display());
                }
            }

            if json {
                let payload = serde_json::json!({
                    "inspection": inspection,
                    "effect_plan": plan,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
                return Ok(());
            }

            println!("Project: {} ({})", inspection.name, inspection.source_format.display_name());
            println!(
                "Bins: {} | Media: {} ({} missing) | Sequences: {}",
                inspection.bin_count,
                inspection.media_count,
                inspection.missing_media_count,
                inspection.sequences.len()
            );
            for seq in &inspection.sequences {
                let kind_tag = match seq.kind {
                    SequenceKind::CompoundClip => " [Compound]",
                    SequenceKind::MulticamClip => " [Multicam]",
                    SequenceKind::Timeline => "",
                };
                let mut extras = String::new();
                if seq.retimed_clip_count > 0 {
                    extras.push_str(&format!(" | {} retimed", seq.retimed_clip_count));
                }
                if seq.nested_clip_count > 0 {
                    extras.push_str(&format!(
                        " | {} compound/multicam/nested",
                        seq.nested_clip_count
                    ));
                }
                println!(
                    "  • {}{} — {}x{} @ {} fps | {} | V:{} ({} clips) A:{} ({} clips) | {} transitions | {} effects{}",
                    seq.name,
                    kind_tag,
                    seq.width,
                    seq.height,
                    seq.fps_label,
                    seq.duration_tc,
                    seq.video_track_count,
                    seq.video_clip_count,
                    seq.audio_track_count,
                    seq.audio_clip_count,
                    seq.transition_count,
                    seq.effect_count,
                    extras
                );
            }
            if !plan.effects.is_empty() {
                println!("\nDetected Effects (target: {}):", target_fmt.short_name());
                for fx in &plan.effects {
                    println!(
                        "  [{:?}] {} ({}) × {} clips",
                        fx.fidelity, fx.display_name, fx.match_name, fx.clip_count
                    );
                }
            }
        }
        Commands::Convert {
            input,
            output,
            preset,
            relink,
            project_name,
            sequences,
            flatten_nested,
            no_bins,
            no_unused_media,
            no_markers,
            no_clip_colors,
            no_grades,
            no_fx_markers,
            json,
        } => {
            let mut project = load_project(&input)?;
            let out_ext = output
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let target_fmt = NleFormat::from_extension(out_ext).ok_or_else(|| {
                format!(
                    "Unsupported output extension '.{out_ext}'. Supported: .drp, .prproj, .fcpxml, .otio, .xml"
                )
            })?;

            let selected_sequence_ids: Vec<String> = if sequences.is_empty() {
                Vec::new()
            } else {
                project
                    .sequences
                    .iter()
                    .filter(|s| {
                        sequences
                            .iter()
                            .any(|q| q == &s.id || q.eq_ignore_ascii_case(&s.name))
                    })
                    .map(|s| s.id.clone())
                    .collect()
            };
            if !sequences.is_empty() && selected_sequence_ids.is_empty() {
                return Err(format!(
                    "No sequences matched --sequence filter: {}",
                    sequences.join(", ")
                )
                .into());
            }

            let mut options = ConversionOptions {
                target_format: target_fmt,
                project_name,
                selected_sequence_ids,
                include_bins: !no_bins,
                include_unused_media: !no_unused_media,
                include_markers: !no_markers,
                include_clip_colors: !no_clip_colors,
                include_lumetri_grades: !no_grades,
                inject_effect_review_markers: !no_fx_markers,
                flatten_nested_and_multicam: flatten_nested,
                relink_rules: Vec::new(),
            };
            for r in relink {
                if let Some((from, to)) = r.split_once('=') {
                    options.relink_rules.push(nle_core::PathRelinkRule {
                        from_prefix: from.to_string(),
                        to_prefix: to.to_string(),
                    });
                }
            }
            nle_core::apply_relink_rules(&mut project, &options.relink_rules);

            let mut pre_messages = Vec::new();
            if options.flatten_nested_and_multicam {
                let flat = flatten_nested_and_multicam_clips(&mut project);
                pre_messages.extend(flat.messages);
                options.flatten_nested_and_multicam = false;
            }

            let plan = analyze_project_effects(&project, &options.selected_sequence_ids, target_fmt);
            let decisions = if let Some(preset_path) = preset {
                let text = std::fs::read_to_string(preset_path)?;
                UserEffectDecisions::from_json_str(&text)?
            } else {
                UserEffectDecisions::default()
            };
            let fx_summary = apply_effect_decisions(
                &mut project,
                &plan,
                &decisions,
                options.inject_effect_review_markers,
            );
            if fx_summary.disable_lumetri_native {
                options.include_lumetri_grades = false;
            }

            let mut report = match target_fmt {
                NleFormat::PremierePro => nle_premiere::write_prproj_file(&project, &output, &options)?,
                NleFormat::DaVinciResolve => nle_resolve::write_drp_file(&project, &output, &options)?,
                NleFormat::FinalCutPro => nle_interchange::write_fcpxml_file(&project, &output, &options)?,
                NleFormat::OpenTimelineIo => nle_interchange::write_otio_file(&project, &output, &options)?,
                NleFormat::Fcp7Xml => nle_interchange::write_fcp7_xml_file(&project, &output, &options)?,
                other => {
                    return Err(format!("Direct writing to {} is not implemented yet.", other.display_name()).into());
                }
            };
            report.effect_markers_injected = fx_summary.markers_injected;
            pre_messages.append(&mut report.messages);
            pre_messages.extend(fx_summary.messages);
            report.messages = pre_messages;

            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "Converted {} -> {} ({} sequences, {} video clips, {} audio clips, {} grades, {} markers, {} effect review markers)",
                    input.display(),
                    report.output_path,
                    report.sequences_converted,
                    report.video_clips_converted,
                    report.audio_clips_converted,
                    report.grades_attached,
                    report.markers_written,
                    report.effect_markers_injected
                );
                for msg in &report.messages {
                    println!("  [{:?}] {} ({})", msg.severity, msg.message, msg.code);
                }
            }
        }
    }
    Ok(())
}
