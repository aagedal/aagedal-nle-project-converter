use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NleFormat {
    PremierePro,
    DaVinciResolve,
    FinalCutPro,
    Kdenlive,
    Fcp7Xml,
    OpenTimelineIo,
}

impl NleFormat {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::PremierePro => "Adobe Premiere Pro (.prproj)",
            Self::DaVinciResolve => "DaVinci Resolve (.drp)",
            Self::FinalCutPro => "Final Cut Pro (.fcpxml)",
            Self::Kdenlive => "Kdenlive (.kdenlive)",
            Self::Fcp7Xml => "FCP7 XML (.xml)",
            Self::OpenTimelineIo => "OpenTimelineIO (.otio)",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PremierePro => "premiere_pro",
            Self::DaVinciResolve => "da_vinci_resolve",
            Self::FinalCutPro => "final_cut_pro",
            Self::Kdenlive => "kdenlive",
            Self::Fcp7Xml => "fcp7_xml",
            Self::OpenTimelineIo => "open_timeline_io",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "premiere_pro" => Self::PremierePro,
            "final_cut_pro" => Self::FinalCutPro,
            "open_timeline_io" => Self::OpenTimelineIo,
            "fcp7_xml" => Self::Fcp7Xml,
            "kdenlive" => Self::Kdenlive,
            _ => Self::DaVinciResolve,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingFidelity {
    NativeAutomatic,
    AskUserSuggested,
    Unmapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetEffectOption {
    pub id: String,
    pub display_name: String,
    pub target_nle: NleFormat,
    pub studio_only: bool,
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MappingAction {
    NativeConvert,
    MapToEquivalent {
        target_effect_id: String,
        target_display_name: String,
        add_review_marker: bool,
    },
    PlaceMarkerOnly,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedEffectGroup {
    pub key: String,
    pub match_name: String,
    pub display_name: String,
    pub clip_count: usize,
    pub sample_clips: Vec<String>,
    pub has_keyframes: bool,
    pub fidelity: MappingFidelity,
    pub notes: String,
    pub available_targets: Vec<TargetEffectOption>,
    pub default_action: MappingAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectMappingPlan {
    pub source_format: NleFormat,
    pub target_format: NleFormat,
    pub effects: Vec<DetectedEffectGroup>,
    pub native_auto_count: usize,
    pub needs_user_review_count: usize,
    pub unmapped_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserEffectDecisions {
    pub actions: BTreeMap<String, MappingAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectMappingPreset {
    pub name: String,
    pub source_format: NleFormat,
    pub target_format: NleFormat,
    pub decisions: UserEffectDecisions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedProjectPayload {
    pub inspection: ProjectInspection,
    pub effect_plan: EffectMappingPlan,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionMessage {
    pub severity: Severity,
    pub code: String,
    pub sequence: Option<String>,
    pub clip: Option<String>,
    pub message: String,
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
