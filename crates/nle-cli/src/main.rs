use clap::{Parser, Subcommand};
use nle_core::{ConversionOptions, NleFormat, Project, ProjectInspection};
use nle_effects::{analyze_project_effects, apply_effect_decisions, UserEffectDecisions};
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
    /// Inspect a .prproj or .drp project and list sequences, media, and detected effects
    Inspect {
        input: PathBuf,
        #[arg(long, default_value = "drp")]
        target: String,
    },
    /// Convert a .prproj or .drp directly to .prproj, .drp, .fcpxml, .otio, or .xml
    Convert {
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        /// Optional JSON effect mapping decisions / preset file
        #[arg(long)]
        preset: Option<PathBuf>,
        /// Optional path relink rule in FROM=TO format
        #[arg(long)]
        relink: Vec<String>,
    },
    /// Generate a sample .prproj file containing bins, Lumetri, Motion, and app-exclusive effects
    Demo {
        output: PathBuf,
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
        _ => Err(format!("Unsupported input format '.{ext}'. Supported: .prproj, .drp").into()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Demo { output } => {
            nle_premiere::write_sample_prproj(&output)?;
            println!("Wrote demo Premiere Pro project to {}", output.display());
        }
        Commands::Inspect { input, target } => {
            let project = load_project(&input)?;
            let inspection = ProjectInspection::from_project(&project);
            let target_fmt = NleFormat::from_extension(&target).unwrap_or(NleFormat::DaVinciResolve);
            let plan = analyze_project_effects(&project, &[], target_fmt);

            println!("Project: {} ({})", inspection.name, inspection.source_format.display_name());
            println!(
                "Bins: {} | Media: {} ({} missing) | Sequences: {}",
                inspection.bin_count,
                inspection.media_count,
                inspection.missing_media_count,
                inspection.sequences.len()
            );
            for (idx, seq) in inspection.sequences.iter().enumerate() {
                let raw_seq = project.sequences.get(idx);
                let kind_tag = match raw_seq.map(|s| s.kind) {
                    Some(nle_core::SequenceKind::CompoundClip) => " [Compound]",
                    Some(nle_core::SequenceKind::MulticamClip) => " [Multicam]",
                    _ => "",
                };
                let trans_count: usize = raw_seq
                    .map(|s| {
                        s.video_tracks
                            .iter()
                            .chain(s.audio_tracks.iter())
                            .map(|t| t.transitions.len())
                            .sum()
                    })
                    .unwrap_or(0);
                let retimed_count: usize = raw_seq
                    .map(|s| {
                        s.video_tracks
                            .iter()
                            .flat_map(|t| t.clips.iter())
                            .filter(|c| (c.speed_percent - 100.0).abs() > 0.01 || c.reversed)
                            .count()
                    })
                    .unwrap_or(0);
                let nested_count: usize = raw_seq
                    .map(|s| {
                        s.video_tracks
                            .iter()
                            .flat_map(|t| t.clips.iter())
                            .filter(|c| c.source_kind != nle_core::ClipSourceKind::Media)
                            .count()
                    })
                    .unwrap_or(0);
                let mut extras = String::new();
                if retimed_count > 0 {
                    extras.push_str(&format!(" | {retimed_count} retimed"));
                }
                if nested_count > 0 {
                    extras.push_str(&format!(" | {nested_count} compound/multicam/nested"));
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
                    trans_count,
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
        } => {
            let mut project = load_project(&input)?;
            let target_fmt = output
                .extension()
                .and_then(|s| s.to_str())
                .and_then(NleFormat::from_extension)
                .unwrap_or(NleFormat::DaVinciResolve);

            let mut options = ConversionOptions {
                target_format: target_fmt,
                ..Default::default()
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

            let plan = analyze_project_effects(&project, &options.selected_sequence_ids, target_fmt);
            let decisions = if let Some(preset_path) = preset {
                let text = std::fs::read_to_string(preset_path)?;
                serde_json::from_str::<UserEffectDecisions>(&text)?
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
            report.messages.extend(fx_summary.messages);

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
        }
    }
    Ok(())
}
