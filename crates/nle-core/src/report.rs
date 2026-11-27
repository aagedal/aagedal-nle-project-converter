use crate::model::{ClipSourceKind, NleFormat, Project, SequenceKind};
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
    #[serde(default = "default_true")]
    pub include_unused_media: bool,
    pub include_markers: bool,
    pub include_clip_colors: bool,
    pub include_lumetri_grades: bool,
    pub inject_effect_review_markers: bool,
    /// When true, flatten nested sequences, compound clips, and multicam cuts into underlying source media clips.
    #[serde(default)]
    pub flatten_nested_and_multicam: bool,
    pub relink_rules: Vec<PathRelinkRule>,
}

fn default_true() -> bool {
    true
}

impl Default for ConversionOptions {
    fn default() -> Self {
        Self {
            target_format: NleFormat::DaVinciResolve,
            project_name: None,
            selected_sequence_ids: Vec::new(),
            include_bins: true,
            include_unused_media: true,
            include_markers: true,
            include_clip_colors: true,
            include_lumetri_grades: true,
            inject_effect_review_markers: true,
            flatten_nested_and_multicam: false,
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
    #[serde(default)]
    pub kind: SequenceKind,
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
    #[serde(default)]
    pub transition_count: usize,
    #[serde(default)]
    pub retimed_clip_count: usize,
    #[serde(default)]
    pub nested_clip_count: usize,
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
                let transition_count: usize = seq
                    .video_tracks
                    .iter()
                    .chain(seq.audio_tracks.iter())
                    .map(|t| t.transitions.len())
                    .sum();
                let retimed_clip_count: usize = seq
                    .video_tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .filter(|c| {
                        (c.speed_percent - 100.0).abs() > 0.01
                            || c.reversed
                            || c.speed_keyframes.len() > 2
                    })
                    .count();
                let nested_clip_count: usize = seq
                    .video_tracks
                    .iter()
                    .flat_map(|t| t.clips.iter())
                    .filter(|c| c.source_kind != ClipSourceKind::Media)
                    .count();
                SequenceSummary {
                    id: seq.id.clone(),
                    name: seq.name.clone(),
                    kind: seq.kind,
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
                    transition_count,
                    retimed_clip_count,
                    nested_clip_count,
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
/// Matches prefixes case-insensitively with normalized `/` / `\` separators so
/// Windows and POSIX paths relink reliably across platforms.
pub fn apply_relink_rules(project: &mut Project, rules: &[PathRelinkRule]) {
    if rules.is_empty() {
        return;
    }
    let rewrite = |path: &mut String| {
        if path.is_empty() {
            return;
        }
        let norm_path = path.replace('\\', "/");
        let norm_path_lower = norm_path.to_ascii_lowercase();
        for rule in rules {
            let from = rule.from_prefix.trim();
            if from.is_empty() {
                continue;
            }
            if path.starts_with(from) {
                let tail = &path[from.len()..];
                let to_uses_backslash =
                    rule.to_prefix.contains('\\') && !rule.to_prefix.contains('/');
                let formatted_tail = if to_uses_backslash {
                    tail.replace('/', "\\")
                } else {
                    tail.replace('\\', "/")
                };
                *path = format!("{}{}", rule.to_prefix, formatted_tail);
                break;
            }
            let norm_from = from.replace('\\', "/");
            let norm_from_lower = norm_from.to_ascii_lowercase();
            if norm_path_lower.starts_with(&norm_from_lower) {
                let tail = &norm_path[norm_from.len()..];
                let to_uses_backslash =
                    rule.to_prefix.contains('\\') && !rule.to_prefix.contains('/');
                let formatted_tail = if to_uses_backslash {
                    tail.replace('/', "\\")
                } else {
                    tail.to_string()
                };
                *path = format!("{}{}", rule.to_prefix, formatted_tail);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AlphaMode, DataLevels, MediaAsset, MediaKind};
    use crate::time::Rational;
    use std::collections::BTreeMap;

    #[test]
    fn relink_rules_handle_mixed_separators_and_case() {
        let mut project = Project {
            media_assets: vec![MediaAsset {
                id: "m1".into(),
                name: "Clip.mov".into(),
                path: "D:/Media/Shoot/Clip.mov".into(),
                media_kind: MediaKind::Video,
                width: 1920,
                height: 1080,
                frame_rate: Rational::FPS_25,
                duration_frames: 100,
                source_tc_frame: 0,
                source_tc_string: "00:00:00:00".into(),
                audio_channels: 2,
                audio_sample_rate: 48000,
                color_space: None,
                video_codec: None,
                audio_codec: None,
                alpha_mode: AlphaMode::Default,
                data_levels: DataLevels::Auto,
                pixel_aspect_ratio: None,
                metadata: BTreeMap::new(),
                custom_metadata: BTreeMap::new(),
            }],
            ..Default::default()
        };

        apply_relink_rules(
            &mut project,
            &[PathRelinkRule {
                from_prefix: r"d:\Media".into(),
                to_prefix: "/Volumes/RAID/Media".into(),
            }],
        );
        assert_eq!(project.media_assets[0].path, "/Volumes/RAID/Media/Shoot/Clip.mov");

        apply_relink_rules(
            &mut project,
            &[PathRelinkRule {
                from_prefix: "/Volumes/RAID/Media".into(),
                to_prefix: r"E:\Edit\Media".into(),
            }],
        );
        assert_eq!(project.media_assets[0].path, r"E:\Edit\Media\Shoot\Clip.mov");
    }
}
