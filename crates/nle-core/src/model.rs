use crate::time::Rational;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Supported NLE project / interchange formats.
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

    pub fn short_name(self) -> &'static str {
        match self {
            Self::PremierePro => "Premiere Pro",
            Self::DaVinciResolve => "DaVinci Resolve",
            Self::FinalCutPro => "Final Cut Pro",
            Self::Kdenlive => "Kdenlive",
            Self::Fcp7Xml => "FCP7 XML",
            Self::OpenTimelineIo => "OpenTimelineIO",
        }
    }

    pub fn default_extension(self) -> &'static str {
        match self {
            Self::PremierePro => "prproj",
            Self::DaVinciResolve => "drp",
            Self::FinalCutPro => "fcpxml",
            Self::Kdenlive => "kdenlive",
            Self::Fcp7Xml => "xml",
            Self::OpenTimelineIo => "otio",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "prproj" => Some(Self::PremierePro),
            "drp" | "drt" => Some(Self::DaVinciResolve),
            "fcpxml" | "fcpxmld" => Some(Self::FinalCutPro),
            "kdenlive" => Some(Self::Kdenlive),
            "xml" => Some(Self::Fcp7Xml),
            "otio" => Some(Self::OpenTimelineIo),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectCategory {
    Transform,
    Color,
    Blur,
    Keying,
    Stabilization,
    Distort,
    Stylize,
    Transition,
    Audio,
    Title,
    Plugin,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerScope {
    Sequence,
    Track,
    Clip,
}

/// Canonical project representation shared by every NLE reader & writer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub source_format: NleFormat,
    pub source_path: Option<String>,
    pub color_config: ColorConfig,
    pub scale_to_frame_size: bool,
    pub bins: Vec<ProjectBin>,
    pub media_assets: Vec<MediaAsset>,
    pub sequences: Vec<Sequence>,
    pub metadata: BTreeMap<String, String>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "Untitled Project".to_string(),
            source_format: NleFormat::PremierePro,
            source_path: None,
            color_config: ColorConfig::default(),
            scale_to_frame_size: true,
            bins: Vec::new(),
            media_assets: Vec::new(),
            sequences: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColorConfig {
    pub working_space: Option<String>,
    pub output_space: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectBin {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub media_paths: Vec<String>,
    pub sequence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: String,
    pub name: String,
    pub path: String,
    pub media_kind: MediaKind,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    pub duration_frames: i64,
    pub source_tc_frame: i64,
    pub source_tc_string: String,
    pub audio_channels: u32,
    pub audio_sample_rate: u32,
    pub color_space: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sequence {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    pub duration_frames: i64,
    pub mark_in: Option<i64>,
    pub mark_out: Option<i64>,
    pub video_tracks: Vec<Track>,
    pub audio_tracks: Vec<Track>,
    pub markers: Vec<Marker>,
    pub metadata: BTreeMap<String, String>,
}

impl Sequence {
    pub fn total_clip_count(&self) -> usize {
        self.video_tracks.iter().map(|t| t.clips.len()).sum::<usize>()
            + self.audio_tracks.iter().map(|t| t.clips.len()).sum::<usize>()
    }

    pub fn computed_duration_frames(&self) -> i64 {
        let clip_max = self
            .video_tracks
            .iter()
            .chain(self.audio_tracks.iter())
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end_frame)
            .max()
            .unwrap_or(0);
        self.duration_frames.max(clip_max)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub index: usize,
    pub kind: TrackKind,
    pub name: String,
    pub enabled: bool,
    pub locked: bool,
    pub muted: bool,
    pub solo: bool,
    pub clips: Vec<Clip>,
    pub transitions: Vec<Transition>,
    pub markers: Vec<Marker>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub name: String,
    pub media_id: String,
    pub media_path: String,
    pub media_kind: MediaKind,
    pub start_frame: i64,
    pub end_frame: i64,
    pub source_in_frame: i64,
    pub source_out_frame: i64,
    pub source_in_ticks: Option<i64>,
    pub source_out_ticks: Option<i64>,
    pub source_fps: Rational,
    pub source_width: u32,
    pub source_height: u32,
    pub speed_percent: f64,
    pub reversed: bool,
    pub enabled: bool,
    pub audio_channel: u32,
    pub link_group_id: Option<String>,
    pub label_name: Option<String>,
    pub label_preset_index: Option<u8>,
    pub motion: MotionTransform,
    pub opacity: f64,
    pub lumetri_effects: Vec<LumetriData>,
    pub effects: Vec<EffectInstance>,
    pub markers: Vec<Marker>,
}

impl Clip {
    pub fn duration_frames(&self) -> i64 {
        (self.end_frame - self.start_frame).max(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MotionTransform {
    pub scale: f64,
    pub scale_y: Option<f64>,
    pub position_x: f64,
    pub position_y: f64,
    pub rotation: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub crop_left: f64,
    pub crop_right: f64,
    pub crop_top: f64,
    pub crop_bottom: f64,
    pub has_keyframes: bool,
}

impl Default for MotionTransform {
    fn default() -> Self {
        Self {
            scale: 100.0,
            scale_y: None,
            position_x: 0.0,
            position_y: 0.0,
            rotation: 0.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            crop_left: 0.0,
            crop_right: 0.0,
            crop_top: 0.0,
            crop_bottom: 0.0,
            has_keyframes: false,
        }
    }
}

impl MotionTransform {
    pub fn is_identity(&self) -> bool {
        (self.scale - 100.0).abs() < 1e-6
            && self.scale_y.is_none()
            && self.position_x.abs() < 1e-6
            && self.position_y.abs() < 1e-6
            && self.rotation.abs() < 1e-6
            && self.anchor_x.abs() < 1e-6
            && self.anchor_y.abs() < 1e-6
            && self.crop_left.abs() < 1e-6
            && self.crop_right.abs() < 1e-6
            && self.crop_top.abs() < 1e-6
            && self.crop_bottom.abs() < 1e-6
            && !self.has_keyframes
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LumetriData {
    pub order: usize,
    pub display_name: String,
    pub exposure: Option<f64>,
    pub contrast: Option<f64>,
    pub saturation: Option<f64>,
    pub temperature: Option<f64>,
    pub tint: Option<f64>,
    pub highlights: Option<f64>,
    pub shadows: Option<f64>,
    pub whites: Option<f64>,
    pub blacks: Option<f64>,
    pub is_animated: bool,
    pub unmapped_controls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectInstance {
    pub id: String,
    pub match_name: String,
    pub display_name: String,
    pub category: EffectCategory,
    pub media_kind: TrackKind,
    pub enabled: bool,
    pub is_intrinsic: bool,
    pub is_animated: bool,
    pub parameters: Vec<EffectParameter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectParameter {
    pub id: String,
    pub name: String,
    pub value: String,
    pub is_animated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub name: String,
    pub effect_id: String,
    pub start_frame: i64,
    pub end_frame: i64,
    pub alignment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub frame: i64,
    pub duration_frames: i64,
    pub name: String,
    pub comment: String,
    pub color: String,
    pub scope: MarkerScope,
}
