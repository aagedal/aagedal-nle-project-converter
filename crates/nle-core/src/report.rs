use crate::model::{NleFormat, Project};
use crate::time::frames_to_timecode;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionMessage {
    pub severity: Severity,
    pub code: String,
    pub sequence: Option<String>,
    pub clip: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRelinkRule {
    pub from_prefix: String,
    pub to_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionOptions {
    pub target_format: NleFormat,
    pub project_name: Option<String>,
    pub selected_sequence_ids: Vec<String>,
    pub include_bins: bool,
    pub include_markers: bool,
    pub include_clip_colors: bool,
    pub include_lumetri_grades: bool,
    pub inject_effect_review_markers: bool,
    pub relink_rules: Vec<PathRelinkRule>,
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            target_format: NleFormat::DaVinciResolve,
            project_name: None,
            selected_sequence_ids: Vec::new(),
            include_bins: true,
            include_markers: true,
            include_clip_colors: true,
            include_lumetri_grades: true,
            inject_effect_review_markers: true,
            relink_rules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionReport {
    pub output_path: String,
    pub source_format: NleFormat,
    pub target_format: NleFormat,
    pub sequences_converted: usize,
    pub video_clips_converted: usize,
    pub audio_clips_converted: usize,
    pub media_assets_linked: usize,
    pub bins_created: usize,
    pub markers_written: usize,
    pub grades_attached: usize,
    pub effect_markers_injected: usize,
    pub messages: Vec<ConversionMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SequenceSummary {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps_label: String,
    pub duration_tc: String,
    pub duration_frames: i64,
    pub video_track_count: usize,
    pub audio_track_count: usize,
    pub video_clip_count: usize,
    pub audio_clip_count: usize,
    pub marker_count: usize,
    pub effect_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSummary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub resolution: String,
    pub fps_label: String,
    pub exists_on_disk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInspection {
    pub name: String,
    pub source_format: NleFormat,
    pub source_path: Option<String>,
    pub working_color_space: Option<String>,
    pub output_color_space: Option<String>,
    pub bin_count: usize,
    pub media_count: usize,
    pub missing_media_count: usize,
    pub sequences: Vec<SequenceSummary>,
    pub media_preview: Vec<MediaSummary>,
}

impl ProjectInspection {
    pub fn from_project(project: &Project) -> Self {
        let sequences = project
            .sequences
            .iter()
            .map(|seq| {
                let dur = seq.computed_duration_frames();
                let video_clip_count: usize = seq.video_tracks.iter().map(|t| t.clips.len()).sum();
                let audio_clip_count: usize = seq.audio_tracks.iter().map(|t| t.clips.len()).sum();
                let effect_count: usize = seq
                    .video_tracks
                    .iter()
                    .chain(seq.audio_tracks.iter())
                    .flat_map(|t| t.clips.iter())
                    .map(|c| c.effects.iter().filter(|e| !e.is_intrinsic).count() + c.lumetri_effects.len())
                    .sum();
                SequenceSummary {
                    id: seq.id.clone(),
                    name: seq.name.clone(),
                    width: seq.width,
                    height: seq.height,
                    fps_label: seq.frame_rate.label(),
                    duration_tc: frames_to_timecode(dur, seq.frame_rate),
                    duration_frames: dur,
                    video_track_count: seq.video_tracks.len(),
                    audio_track_count: seq.audio_tracks.len(),
                    video_clip_count,
                    audio_clip_count,
                    marker_count: seq.markers.len(),
                    effect_count,
                }
            })
            .collect();

        let mut missing_media_count = 0usize;
        let media_preview: Vec<MediaSummary> = project
            .media_assets
            .iter()
            .map(|asset| {
                let exists = !asset.path.is_empty() && Path::new(&asset.path).exists();
                if !exists {
                    missing_media_count += 1;
                }
                MediaSummary {
                    id: asset.id.clone(),
                    name: asset.name.clone(),
                    path: asset.path.clone(),
                    resolution: if asset.width > 0 && asset.height > 0 {
                        format!("{}×{}", asset.width, asset.height)
                    } else {
                        "Audio / Unknown".to_string()
                    },
                    fps_label: asset.frame_rate.label(),
                    exists_on_disk: exists,
                }
            })
            .collect();

        Self {
            name: project.name.clone(),
            source_format: project.source_format,
            source_path: project.source_path.clone(),
            working_color_space: project.color_config.working_space.clone(),
            output_color_space: project.color_config.output_space.clone(),
            bin_count: project.bins.len(),
            media_count: project.media_assets.len(),
            missing_media_count,
            sequences,
            media_preview,
        }
    }
}

/// Apply path prefix relinking rules to all media references in a project clone.
pub fn apply_relink_rules(project: &mut Project, rules: &[PathRelinkRule]) {
    if rules.is_empty() {
        return;
    }
    let rewrite = |path: &mut String| {
        for rule in rules {
            if !rule.from_prefix.is_empty() && path.starts_with(&rule.from_prefix) {
                *path = format!("{}{}", rule.to_prefix, &path[rule.from_prefix.len()..]);
                break;
            }
        }
    };
    for asset in &mut project.media_assets {
        rewrite(&mut asset.path);
    }
    for bin in &mut project.bins {
        for p in &mut bin.media_paths {
            rewrite(p);
        }
    }
    for seq in &mut project.sequences {
        for track in seq.video_tracks.iter_mut().chain(seq.audio_tracks.iter_mut()) {
            for clip in &mut track.clips {
                rewrite(&mut clip.media_path);
            }
        }
    }
}
