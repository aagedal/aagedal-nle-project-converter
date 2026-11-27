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
    Edl,
    Aaf,
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
            Self::Edl => "CMX 3600 EDL (.edl)",
            Self::Aaf => "AAF (.aaf)",
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
            Self::Edl => "EDL",
            Self::Aaf => "AAF",
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
            Self::Edl => "edl",
            Self::Aaf => "aaf",
        }
    }

    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "prproj" => Some(Self::PremierePro),
            "drp" | "drt" => Some(Self::DaVinciResolve),
            "fcpxml" | "fcpxmld" | "fcpbundle" => Some(Self::FinalCutPro),
            "kdenlive" => Some(Self::Kdenlive),
            "xml" => Some(Self::Fcp7Xml),
            "otio" | "otiod" | "otioz" => Some(Self::OpenTimelineIo),
            "edl" => Some(Self::Edl),
            "aaf" => Some(Self::Aaf),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequenceKind {
    #[default]
    Timeline,
    CompoundClip,
    MulticamClip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipSourceKind {
    #[default]
    Media,
    NestedSequence,
    CompoundClip,
    MulticamClip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Default,
    None,
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataLevels {
    #[default]
    Auto,
    Video,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    DarkerColor,
    Lighten,
    Screen,
    ColorDodge,
    LinearDodgeAdd,
    LighterColor,
    Overlay,
    SoftLight,
    HardLight,
    VividLight,
    LinearLight,
    PinLight,
    HardMix,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub fn is_normal(self) -> bool {
        self == Self::Normal
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Darken => "Darken",
            Self::Multiply => "Multiply",
            Self::ColorBurn => "Color Burn",
            Self::LinearBurn => "Linear Burn",
            Self::DarkerColor => "Darker Color",
            Self::Lighten => "Lighten",
            Self::Screen => "Screen",
            Self::ColorDodge => "Color Dodge",
            Self::LinearDodgeAdd => "Add",
            Self::LighterColor => "Lighter Color",
            Self::Overlay => "Overlay",
            Self::SoftLight => "Soft Light",
            Self::HardLight => "Hard Light",
            Self::VividLight => "Vivid Light",
            Self::LinearLight => "Linear Light",
            Self::PinLight => "Pin Light",
            Self::HardMix => "Hard Mix",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
            Self::Subtract => "Subtract",
            Self::Divide => "Divide",
            Self::Hue => "Hue",
            Self::Saturation => "Saturation",
            Self::Color => "Color",
            Self::Luminosity => "Luminosity",
        }
    }

    /// Parse FCPXML `<adjust-blend mode="...">` (e.g. `"14 (Overlay)"`, `"Screen"`, `"Add"`)
    /// or any human-readable blend mode name.
    pub fn parse_name(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Extract parenthesized label if present, e.g. "14 (Overlay)" -> "Overlay"
        let inner = if let (Some(l), Some(r)) = (trimmed.find('('), trimmed.rfind(')')) {
            if r > l + 1 {
                &trimmed[l + 1..r]
            } else {
                trimmed
            }
        } else {
            trimmed
        };
        let norm: String = inner
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match norm.as_str() {
            "normal" => Some(Self::Normal),
            "darken" => Some(Self::Darken),
            "multiply" => Some(Self::Multiply),
            "colorburn" => Some(Self::ColorBurn),
            "linearburn" => Some(Self::LinearBurn),
            "darkercolor" => Some(Self::DarkerColor),
            "lighten" => Some(Self::Lighten),
            "screen" => Some(Self::Screen),
            "colordodge" => Some(Self::ColorDodge),
            "add" | "lineardodge" | "lineardodgeadd" => Some(Self::LinearDodgeAdd),
            "lightercolor" => Some(Self::LighterColor),
            "overlay" => Some(Self::Overlay),
            "softlight" => Some(Self::SoftLight),
            "hardlight" => Some(Self::HardLight),
            "vividlight" => Some(Self::VividLight),
            "linearlight" => Some(Self::LinearLight),
            "pinlight" => Some(Self::PinLight),
            "hardmix" => Some(Self::HardMix),
            "difference" => Some(Self::Difference),
            "exclusion" => Some(Self::Exclusion),
            "subtract" => Some(Self::Subtract),
            "divide" => Some(Self::Divide),
            "hue" => Some(Self::Hue),
            "saturation" | "saturate" => Some(Self::Saturation),
            "color" | "colorize" => Some(Self::Color),
            "luminosity" | "luminance" => Some(Self::Luminosity),
            _ => {
                // Fallback: leading numeric FCP OZML blend code (e.g. "14" or "14 (Overlay)")
                trimmed
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<i64>().ok())
                    .and_then(Self::from_fcp_code)
            }
        }
    }

    /// Decode Final Cut Pro OZML / Motion numeric blend mode code.
    pub fn from_fcp_code(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Normal),
            2 => Some(Self::Subtract),
            3 => Some(Self::Multiply),
            4 => Some(Self::Screen),
            5 => Some(Self::Darken),
            6 => Some(Self::Lighten),
            7 => Some(Self::LinearDodgeAdd),
            14 => Some(Self::Overlay),
            15 => Some(Self::SoftLight),
            16 => Some(Self::HardLight),
            17 => Some(Self::ColorDodge),
            18 => Some(Self::ColorBurn),
            21 => Some(Self::Difference),
            22 => Some(Self::Exclusion),
            23 => Some(Self::Hue),
            24 => Some(Self::Saturation),
            25 => Some(Self::Color),
            26 => Some(Self::Luminosity),
            27 => Some(Self::VividLight),
            28 => Some(Self::LinearLight),
            29 => Some(Self::PinLight),
            30 => Some(Self::HardMix),
            31 => Some(Self::LinearBurn),
            32 => Some(Self::DarkerColor),
            33 => Some(Self::LighterColor),
            34 => Some(Self::Divide),
            _ => None,
        }
    }

    /// FCPXML `<adjust-blend mode="...">` string value.
    pub fn to_fcpxml_mode(self) -> &'static str {
        self.display_name()
    }

    /// Decode DaVinci Resolve `EffectFiltersBA` Filter 2 Param 0 (`CompositeMode`) enum.
    pub fn from_resolve_code(code: u64) -> Option<Self> {
        match code {
            0 => Some(Self::Normal),
            1 | 19 => Some(Self::LinearDodgeAdd),
            2 => Some(Self::Subtract),
            3 => Some(Self::Difference),
            4 => Some(Self::Multiply),
            5 => Some(Self::Screen),
            6 => Some(Self::Overlay),
            7 => Some(Self::HardLight),
            8 => Some(Self::SoftLight),
            9 => Some(Self::Darken),
            10 => Some(Self::Lighten),
            11 => Some(Self::ColorDodge),
            12 => Some(Self::ColorBurn),
            13 => Some(Self::Exclusion),
            14 => Some(Self::Hue),
            15 => Some(Self::Saturation),
            16 => Some(Self::Color),
            17 => Some(Self::Luminosity),
            18 => Some(Self::Divide),
            20 => Some(Self::LinearBurn),
            21 => Some(Self::LinearLight),
            22 => Some(Self::VividLight),
            23 => Some(Self::PinLight),
            24 => Some(Self::HardMix),
            25 => Some(Self::LighterColor),
            26 => Some(Self::DarkerColor),
            _ => None,
        }
    }

    /// Encode to DaVinci Resolve `EffectFiltersBA` Filter 2 Param 0 (`CompositeMode`) enum.
    pub fn to_resolve_code(self) -> u64 {
        match self {
            Self::Normal => 0,
            Self::LinearDodgeAdd => 1,
            Self::Subtract => 2,
            Self::Difference => 3,
            Self::Multiply => 4,
            Self::Screen => 5,
            Self::Overlay => 6,
            Self::HardLight => 7,
            Self::SoftLight => 8,
            Self::Darken => 9,
            Self::Lighten => 10,
            Self::ColorDodge => 11,
            Self::ColorBurn => 12,
            Self::Exclusion => 13,
            Self::Hue => 14,
            Self::Saturation => 15,
            Self::Color => 16,
            Self::Luminosity => 17,
            Self::Divide => 18,
            Self::LinearBurn => 20,
            Self::LinearLight => 21,
            Self::VividLight => 22,
            Self::PinLight => 23,
            Self::HardMix => 24,
            Self::LighterColor => 25,
            Self::DarkerColor => 26,
        }
    }

    /// Decode Premiere Pro `AE.ADBE Opacity` `ParameterID=3` (UI Blend Mode enum).
    pub fn from_premiere_param3(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Normal),
            2 => Some(Self::Darken),
            3 => Some(Self::Multiply),
            4 => Some(Self::ColorBurn),
            5 => Some(Self::LinearBurn),
            6 => Some(Self::DarkerColor),
            7 => Some(Self::Lighten),
            8 => Some(Self::Screen),
            9 => Some(Self::ColorDodge),
            10 => Some(Self::LinearDodgeAdd),
            11 => Some(Self::LighterColor),
            12 => Some(Self::Overlay),
            13 => Some(Self::SoftLight),
            14 => Some(Self::HardLight),
            15 => Some(Self::VividLight),
            16 => Some(Self::LinearLight),
            17 => Some(Self::PinLight),
            18 => Some(Self::HardMix),
            19 => Some(Self::Difference),
            20 => Some(Self::Exclusion),
            21 => Some(Self::Subtract),
            22 => Some(Self::Divide),
            23 => Some(Self::Hue),
            24 => Some(Self::Saturation),
            25 => Some(Self::Color),
            26 => Some(Self::Luminosity),
            _ => None,
        }
    }

    /// Decode Premiere Pro `AE.ADBE Opacity` `ParameterID=2` (legacy internal alphabetical enum).
    pub fn from_premiere_param2(code: i64) -> Option<Self> {
        match code {
            0 => Some(Self::Color),
            1 => Some(Self::ColorBurn),
            2 => Some(Self::ColorDodge),
            3 => Some(Self::Darken),
            4 => Some(Self::DarkerColor),
            5 => Some(Self::Difference),
            7 => Some(Self::Exclusion),
            8 => Some(Self::HardLight),
            9 => Some(Self::HardMix),
            10 => Some(Self::Hue),
            11 => Some(Self::Lighten),
            12 => Some(Self::LighterColor),
            13 => Some(Self::LinearBurn),
            14 => Some(Self::LinearDodgeAdd),
            15 => Some(Self::LinearLight),
            16 => Some(Self::Luminosity),
            17 => Some(Self::Multiply),
            18 => Some(Self::Normal),
            19 => Some(Self::Overlay),
            20 => Some(Self::PinLight),
            21 => Some(Self::Saturation),
            22 => Some(Self::Screen),
            23 => Some(Self::SoftLight),
            24 => Some(Self::VividLight),
            25 => Some(Self::Subtract),
            26 => Some(Self::Divide),
            _ => None,
        }
    }

    /// Encode to Premiere Pro `AE.ADBE Opacity` `(ParameterID=2, ParameterID=3)` enum pair.
    pub fn to_premiere_params(self) -> (i64, i64) {
        match self {
            Self::Normal => (18, 0),
            Self::Darken => (3, 2),
            Self::Multiply => (17, 3),
            Self::ColorBurn => (1, 4),
            Self::LinearBurn => (13, 5),
            Self::DarkerColor => (4, 6),
            Self::Lighten => (11, 7),
            Self::Screen => (22, 8),
            Self::ColorDodge => (2, 9),
            Self::LinearDodgeAdd => (14, 10),
            Self::LighterColor => (12, 11),
            Self::Overlay => (19, 12),
            Self::SoftLight => (23, 13),
            Self::HardLight => (8, 14),
            Self::VividLight => (24, 15),
            Self::LinearLight => (15, 16),
            Self::PinLight => (20, 17),
            Self::HardMix => (9, 18),
            Self::Difference => (5, 19),
            Self::Exclusion => (7, 20),
            Self::Subtract => (25, 21),
            Self::Divide => (26, 22),
            Self::Hue => (10, 23),
            Self::Saturation => (21, 24),
            Self::Color => (0, 25),
            Self::Luminosity => (16, 26),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetimeInterpolation {
    #[default]
    Default,
    FrameSampling,
    FrameBlending,
    OpticalFlow,
}

/// Control point on a clip's variable speed ramp curve mapping clip-local timeline seconds
/// to source media seconds (0-based from media start).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpeedKeyframe {
    pub timeline_sec: f64,
    pub source_sec: f64,
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
    /// Custom project metadata schema definitions (`property_name -> display_label`),
    /// preserved across NLEs that support custom/third-party clip metadata (Premiere Pro ↔ Resolve).
    #[serde(default)]
    pub custom_metadata_schema: BTreeMap<String, String>,
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
            custom_metadata_schema: BTreeMap::new(),
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
    /// Custom / third-party metadata attached to this bin (e.g. external asset manager folder IDs).
    #[serde(default)]
    pub custom_metadata: BTreeMap<String, String>,
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
    /// Native video codec identifier (e.g. `"avc1"`, `"hvc1"`, `"apcn"`, `"apch"`), if known.
    #[serde(default)]
    pub video_codec: Option<String>,
    /// Native audio codec identifier (e.g. `"Linear PCM"`, `"AAC"`, `"MP3"`, `"FLAC"`), if known.
    #[serde(default)]
    pub audio_codec: Option<String>,
    /// Clip Attributes alpha interpretation (`Default`, `None` / Ignore Alpha, `Straight`, `Premultiplied`).
    #[serde(default)]
    pub alpha_mode: AlphaMode,
    /// Clip Attributes video data levels (`Auto`, `Video` / legal range, `Full` range).
    #[serde(default)]
    pub data_levels: DataLevels,
    /// Optional overridden pixel aspect ratio (`1.0` = square, `1.3333` = anamorphic, etc.).
    #[serde(default)]
    pub pixel_aspect_ratio: Option<f64>,
    /// Standard file/clip metadata (e.g. Description, Comments, Keywords, Scene, Shot, Take,
    /// Angle, Camera #, Director, Location, Tape Name, etc.).
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// Custom / third-party plugin metadata (Premiere private project metadata ↔ Resolve
    /// third-party clip metadata). Not exported to generic interchange formats.
    #[serde(default)]
    pub custom_metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioBus {
    pub id: String,
    pub name: String,
    /// Channel width (1 = mono, 2 = stereo, 6 = 5.1, etc.). Defaults to 2 (stereo).
    #[serde(default = "default_bus_channels")]
    pub channels: u32,
    /// True for the primary/master output bus (e.g. Resolve Main bus / Premiere MasterTrack).
    #[serde(default)]
    pub is_main: bool,
    /// Optional parent bus ID when this bus routes into another bus (e.g. a submix routed to Main).
    #[serde(default)]
    pub parent_bus_id: Option<String>,
}

fn default_bus_channels() -> u32 {
    2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sequence {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: SequenceKind,
    pub width: u32,
    pub height: u32,
    pub frame_rate: Rational,
    pub duration_frames: i64,
    /// Timeline start timecode offset in sequence frames (e.g. 86400 for 01:00:00:00 @ 24fps).
    /// All clip/transition/marker frame coordinates inside `Sequence` are 0-based relative to this offset.
    #[serde(default)]
    pub start_tc_frame: i64,
    pub mark_in: Option<i64>,
    pub mark_out: Option<i64>,
    pub video_tracks: Vec<Track>,
    pub audio_tracks: Vec<Track>,
    /// Audio mixer buses / submixes (main mix + submix stems) and their bus-to-bus routing.
    #[serde(default)]
    pub audio_buses: Vec<AudioBus>,
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
    /// Audio track channel count (0 = unspecified, 1 = mono, 2 = stereo, 6 = 5.1). Ignored for video tracks.
    #[serde(default)]
    pub audio_channels: u32,
    /// Optional ID of the `AudioBus` this audio track routes to (`None` = main/master bus).
    #[serde(default)]
    pub output_bus_id: Option<String>,
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
    #[serde(default)]
    pub source_kind: ClipSourceKind,
    /// Active multicam camera/angle name or ID when `source_kind == ClipSourceKind::MulticamClip`.
    #[serde(default)]
    pub active_angle: Option<String>,
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
    /// Optional multi-point variable speed ramp curve (`timeline_sec -> source_sec`).
    /// Empty for constant-speed / freeze clips.
    #[serde(default)]
    pub speed_keyframes: Vec<SpeedKeyframe>,
    /// Retime frame interpolation mode (`FrameSampling` / `FrameBlending` / `OpticalFlow`).
    #[serde(default)]
    pub retime_interpolation: RetimeInterpolation,
    pub enabled: bool,
    pub audio_channel: u32,
    pub link_group_id: Option<String>,
    pub label_name: Option<String>,
    pub label_preset_index: Option<u8>,
    pub motion: MotionTransform,
    pub opacity: f64,
    #[serde(default)]
    pub blend_mode: BlendMode,
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

/// Single three-way color wheel value in Premiere Lumetri HSL polar coordinates.
/// Neutral is `hue_deg = 0.0`, `luma = 0.5`, `saturation = 0.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorWheel {
    /// Hue angle in degrees `[0.0, 360.0)` (`0° = Red, 120° = Green, 240° = Blue`).
    pub hue_deg: f64,
    /// Luminance slider `[0.0, 1.0]`, neutral `0.5`.
    pub luma: f64,
    /// Chroma radius / saturation `[0.0, 1.0]`, neutral `0.0`.
    pub saturation: f64,
}

impl Default for ColorWheel {
    fn default() -> Self {
        Self {
            hue_deg: 0.0,
            luma: 0.5,
            saturation: 0.0,
        }
    }
}

impl ColorWheel {
    pub fn is_neutral(&self) -> bool {
        (self.luma - 0.5).abs() < 1e-6 && self.saturation.abs() < 1e-6
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
    /// Creative Vibrance (`-100..100`) ↔ Resolve Primary Color Boost (`-100..100`).
    #[serde(default)]
    pub vibrance: Option<f64>,
    /// Creative Sharpen (`-100..100`) ↔ Resolve Primary Mid/Detail (`-100..100`).
    #[serde(default)]
    pub sharpen: Option<f64>,
    /// Color Wheels & Match Shadows wheel ↔ Resolve Primary Lift wheel.
    #[serde(default)]
    pub shadows_wheel: Option<ColorWheel>,
    /// Color Wheels & Match Midtones wheel ↔ Resolve Primary Gamma wheel.
    #[serde(default)]
    pub midtones_wheel: Option<ColorWheel>,
    /// Color Wheels & Match Highlights wheel ↔ Resolve Primary Gain wheel.
    #[serde(default)]
    pub highlights_wheel: Option<ColorWheel>,
    /// Optional Resolve Primary Offset RGB `[dR, dG, dB]` in DRX space (`(ui - 25.0) / 25.0`).
    #[serde(default)]
    pub offset_rgb: Option<[f64; 3]>,
    pub is_animated: bool,
    pub unmapped_controls: Vec<String>,
}

impl LumetriData {
    pub fn has_primary_adjustments(&self) -> bool {
        self.exposure.is_some_and(|v| v.abs() > 1e-6)
            || self.contrast.is_some_and(|v| v.abs() > 1e-6)
            || self.saturation.is_some_and(|v| (v - 100.0).abs() > 1e-6)
            || self.temperature.is_some_and(|v| v.abs() > 1e-6)
            || self.tint.is_some_and(|v| v.abs() > 1e-6)
            || self.highlights.is_some_and(|v| v.abs() > 1e-6)
            || self.shadows.is_some_and(|v| v.abs() > 1e-6)
            || self.whites.is_some_and(|v| v.abs() > 1e-6)
            || self.blacks.is_some_and(|v| v.abs() > 1e-6)
            || self.vibrance.is_some_and(|v| v.abs() > 1e-6)
            || self.sharpen.is_some_and(|v| v.abs() > 1e-6)
            || self.shadows_wheel.is_some_and(|w| !w.is_neutral())
            || self.midtones_wheel.is_some_and(|w| !w.is_neutral())
            || self.highlights_wheel.is_some_and(|w| !w.is_neutral())
            || self
                .offset_rgb
                .is_some_and(|rgb| rgb.iter().any(|v| v.abs() > 1e-6))
    }
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
