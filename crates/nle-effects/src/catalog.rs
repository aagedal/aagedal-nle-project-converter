use nle_core::{EffectCategory, NleFormat};
use serde::{Deserialize, Serialize};

/// How well an effect can be translated into the target NLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingFidelity {
    /// Natively encoded directly into the output project file (e.g. Motion, Lumetri Basic, Speed).
    NativeAutomatic,
    /// App-exclusive effect with a known equivalent in the target NLE — asks the user for confirmation.
    AskUserSuggested,
    /// No known built-in equivalent (e.g. third-party plugin) — user can place a marker or skip.
    Unmapped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetEffectOption {
    pub id: String,
    pub display_name: String,
    pub target_nle: NleFormat,
    pub category: EffectCategory,
    pub studio_only: bool,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectRule {
    pub id: String,
    pub source_nle: NleFormat,
    /// Case-insensitive substring or exact match against Premiere `MatchName` / effect id.
    pub match_patterns: Vec<String>,
    pub source_display_name: String,
    pub category: EffectCategory,
    pub fidelity: MappingFidelity,
    pub notes: String,
    pub targets: Vec<TargetEffectOption>,
}

/// Returns the built-in cross-NLE effect equivalency catalog.
pub fn builtin_effect_catalog() -> Vec<EffectRule> {
    vec![
        // ── Native Automatic ──────────────────────────────────────────
        EffectRule {
            id: "pr_motion".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["adbe motion".into()],
            source_display_name: "Motion (Transform)".into(),
            category: EffectCategory::Transform,
            fidelity: MappingFidelity::NativeAutomatic,
            notes: "Static Scale, Position, and Rotation are natively encoded into Resolve EffectFiltersBA (with fit-scaling compensation).".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.inspector.transform".into(),
                    display_name: "Inspector → Transform (Zoom / Position / Rotation)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Transform,
                    studio_only: false,
                    notes: "Written natively into Sm2TiVideoClip EffectFiltersBA.".into(),
                },
                TargetEffectOption {
                    id: "fcp.transform".into(),
                    display_name: "Video Inspector → Transform".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Transform,
                    studio_only: false,
                    notes: "Written as FCPXML <adjust-transform>.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_lumetri".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["lumetri".into()],
            source_display_name: "Lumetri Color".into(),
            category: EffectCategory::Color,
            fidelity: MappingFidelity::NativeAutomatic,
            notes: "Basic Correction (Temp, Tint, Exposure, Contrast, Highlights, Shadows, Whites, Blacks, Saturation), Creative Vibrance/Sharpen, and Color Wheels (Shadows/Midtones/Highlights → Lift/Gamma/Gain) are encoded natively as Resolve Color Page serial nodes (Primaries - Color Wheels). Curves/HSL/LUTs are flagged for review.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.color.serial_node".into(),
                    display_name: "Color Page → Serial Node (Primaries - Color Wheels)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Color,
                    studio_only: false,
                    notes: "Maps Basic Correction + Shadows/Midtones/Highlights Color Wheels to Resolve Primaries (Lift, Gamma, Gain, Temp, Tint, Contrast, Highlights, Shadows, Sat, Color Boost, Mid/Detail).".into(),
                },
                TargetEffectOption {
                    id: "fcp.color_wheels".into(),
                    display_name: "Color Inspector → Color Wheels".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Color,
                    studio_only: false,
                    notes: "Annotated via timeline marker with original Lumetri values.".into(),
                },
            ],
        },
        // ── App-Exclusive Video Effects (Ask User) ────────────────────
        EffectRule {
            id: "pr_warp_stabilizer".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["subspacestabilizer".into(), "warp stabilizer".into()],
            source_display_name: "Warp Stabilizer".into(),
            category: EffectCategory::Stabilization,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Premiere-exclusive analysis data cannot be transferred verbatim; target NLE must re-analyze clip.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.inspector.stabilization".into(),
                    display_name: "Inspector → Stabilization (Perspective / Similarity)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Stabilization,
                    studio_only: false,
                    notes: "Activates Resolve Inspector Stabilization (ready to click Stabilize) and adds a review marker with Premiere smoothness settings.".into(),
                },
                TargetEffectOption {
                    id: "resolve.fusion.planar_tracker".into(),
                    display_name: "Fusion → Planar Tracker / Camera Lock".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Stabilization,
                    studio_only: false,
                    notes: "Best for difficult handheld shots where Inspector Stabilization warps.".into(),
                },
                TargetEffectOption {
                    id: "fcp.stabilization".into(),
                    display_name: "Video Inspector → Stabilization (InertiaCam)".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Stabilization,
                    studio_only: false,
                    notes: "Built-in Final Cut Pro stabilizer.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_gaussian_blur".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["gaussian blur".into(), "fast blur".into(), "adbe gaussian".into()],
            source_display_name: "Gaussian Blur".into(),
            category: EffectCategory::Blur,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Maps Premiere Blurriness & Repeat Edge Pixels to target blur effect.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.ofx.gaussian_blur".into(),
                    display_name: "Resolve FX Blur → Gaussian Blur".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Blur,
                    studio_only: false,
                    notes: "Direct equivalent; supports horizontal/vertical strength and border mirroring.".into(),
                },
                TargetEffectOption {
                    id: "resolve.ofx.lens_blur".into(),
                    display_name: "Resolve FX Blur → Lens Blur".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Blur,
                    studio_only: true,
                    notes: "Optical bokeh look (Resolve Studio).".into(),
                },
                TargetEffectOption {
                    id: "fcp.gaussian_blur".into(),
                    display_name: "Effects → Blur → Gaussian".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Blur,
                    studio_only: false,
                    notes: "Native FCP Gaussian Blur.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_ultra_key".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["ultra key".into(), "ultrakey".into(), "keylight".into(), "color key".into()],
            source_display_name: "Ultra Key / Chroma Key".into(),
            category: EffectCategory::Keying,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Chroma keying parameters differ across engines; target keyer needs quick key-color sample.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.ofx.3d_keyer".into(),
                    display_name: "Resolve FX Key → 3D Keyer".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Keying,
                    studio_only: false,
                    notes: "Fastest Edit-page chroma keyer in Resolve with spill suppression.".into(),
                },
                TargetEffectOption {
                    id: "resolve.fusion.delta_keyer".into(),
                    display_name: "Fusion → Delta Keyer".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Keying,
                    studio_only: false,
                    notes: "Production-grade keying for hair/fine edges.".into(),
                },
                TargetEffectOption {
                    id: "fcp.keyer".into(),
                    display_name: "Effects → Keying → Keyer".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Keying,
                    studio_only: false,
                    notes: "Automatic green/blue screen keyer in FCP.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_crop".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["adbe crop".into()],
            source_display_name: "Crop".into(),
            category: EffectCategory::Transform,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Premiere uses percentage edge crops; Resolve Inspector Cropping uses pixel offsets.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.inspector.cropping".into(),
                    display_name: "Inspector → Cropping (L/R/T/B + Softness)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Transform,
                    studio_only: false,
                    notes: "Pixel equivalents calculated from sequence resolution and recorded in marker.".into(),
                },
                TargetEffectOption {
                    id: "fcp.crop".into(),
                    display_name: "Video Inspector → Crop (Trim)".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Transform,
                    studio_only: false,
                    notes: "Native FCP Trim crop.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_opacity".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["adbe opacity".into()],
            source_display_name: "Opacity & Blend Mode".into(),
            category: EffectCategory::Transform,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Non-100% opacity, blend modes, or opacity masks.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.inspector.composite".into(),
                    display_name: "Inspector → Composite (Opacity & Composite Mode)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Transform,
                    studio_only: false,
                    notes: "Maps opacity % and blend mode.".into(),
                },
                TargetEffectOption {
                    id: "fcp.compositing".into(),
                    display_name: "Video Inspector → Compositing".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Transform,
                    studio_only: false,
                    notes: "Native FCP opacity & blend mode.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_sharpen".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["adbe sharpen".into(), "unsharp mask".into()],
            source_display_name: "Sharpen / Unsharp Mask".into(),
            category: EffectCategory::Stylize,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Maps sharpening intensity to target sharpening filter.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.ofx.sharpen".into(),
                    display_name: "Resolve FX Sharpen → Sharpen".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Stylize,
                    studio_only: false,
                    notes: "Edit/Color page OpenFX sharpen.".into(),
                },
                TargetEffectOption {
                    id: "resolve.color.blur_sharpen".into(),
                    display_name: "Color Page → Blur/Sharpen Radius".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Stylize,
                    studio_only: false,
                    notes: "Zero-overhead GPU sharpening on the Color page.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_essential_graphics".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["graphic".into(), "ae.adbe text".into(), "mogrt".into()],
            source_display_name: "Essential Graphics / Text".into(),
            category: EffectCategory::Title,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Premiere MOGRTs & Essential Graphics layers are app-exclusive vector templates.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.titles.text_plus".into(),
                    display_name: "Titles → Text+ (Fusion Title)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Title,
                    studio_only: false,
                    notes: "Full typography & animation control in Resolve.".into(),
                },
                TargetEffectOption {
                    id: "resolve.titles.basic".into(),
                    display_name: "Titles → Text (Basic Title)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Title,
                    studio_only: false,
                    notes: "Lightweight Edit-page title.".into(),
                },
                TargetEffectOption {
                    id: "fcp.basic_title".into(),
                    display_name: "Titles → Basic Title".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Title,
                    studio_only: false,
                    notes: "Native FCP title generator.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_morph_cut".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["morph cut".into(), "morphcut".into()],
            source_display_name: "Morph Cut".into(),
            category: EffectCategory::Transition,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Optical-flow jump-cut smoothing transition.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.transition.smooth_cut".into(),
                    display_name: "Video Transitions → Smooth Cut".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Transition,
                    studio_only: false,
                    notes: "Direct DaVinci Resolve optical-flow equivalent to Morph Cut.".into(),
                },
                TargetEffectOption {
                    id: "fcp.flow".into(),
                    display_name: "Transitions → Dissolves → Flow".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Transition,
                    studio_only: false,
                    notes: "Direct Final Cut Pro optical-flow equivalent.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_tint_bw".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["adbe tint".into(), "black & white".into(), "black and white".into()],
            source_display_name: "Tint / Black & White".into(),
            category: EffectCategory::Color,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Monochrome or two-tone color remapping.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.color.monochrome".into(),
                    display_name: "Color Page → RGB Mixer (Monochrome) / Tint".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Color,
                    studio_only: false,
                    notes: "Clean node-based monochrome conversion.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_corner_pin".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["corner pin".into()],
            source_display_name: "Corner Pin".into(),
            category: EffectCategory::Distort,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Four-point perspective screen replacement / warp.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.ofx.transform_pin".into(),
                    display_name: "Resolve FX Transform → Corner Positioner".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Distort,
                    studio_only: false,
                    notes: "Edit-page four-corner warp.".into(),
                },
                TargetEffectOption {
                    id: "resolve.fusion.corner_positioner".into(),
                    display_name: "Fusion → Corner Positioner".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Distort,
                    studio_only: false,
                    notes: "Full sub-pixel perspective tracking & pinning.".into(),
                },
            ],
        },
        // ── Audio Effects (Ask User) ──────────────────────────────────
        EffectRule {
            id: "pr_essential_sound".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["premiere.essentialsound".into(), "essential sound".into()],
            source_display_name: "Essential Sound".into(),
            category: EffectCategory::Audio,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Premiere Essential Sound panel processing (Dialogue / Music / SFX / Ambience restoration, clarity, loudness).".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.fairlight.voice_isolation".into(),
                    display_name: "Fairlight → Voice Isolation & Track EQ / Dynamics".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Rebuilt using Fairlight Voice Isolation / Noise Reduction, EQ, and Dynamics.".into(),
                },
                TargetEffectOption {
                    id: "fcp.voice_isolation".into(),
                    display_name: "Audio Inspector → Voice Isolation & Channel EQ".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Built-in FCP Voice Isolation & EQ.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_denoise".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["denoise".into(), "de-noise".into(), "enhance speech".into()],
            source_display_name: "DeNoise / Enhance Speech".into(),
            category: EffectCategory::Audio,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "AI / spectral dialogue cleanup.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.fairlight.voice_isolation".into(),
                    display_name: "Fairlight → Voice Isolation (AI)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Audio,
                    studio_only: true,
                    notes: "DaVinci Resolve Studio neural voice isolation.".into(),
                },
                TargetEffectOption {
                    id: "resolve.fairlight.noise_reduction".into(),
                    display_name: "Fairlight FX → Noise Reduction".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Auto speech / manual noise print reduction.".into(),
                },
                TargetEffectOption {
                    id: "fcp.voice_isolation".into(),
                    display_name: "Audio Inspector → Voice Isolation".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Built-in FCP Voice Isolation.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_parametric_eq".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["parametric eq".into(), "eq".into(), "highpass".into(), "lowpass".into()],
            source_display_name: "Parametric Equalizer".into(),
            category: EffectCategory::Audio,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Maps EQ band settings to target mixer EQ.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.fairlight.eq".into(),
                    display_name: "Fairlight → Track / Clip Parametric EQ".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "6-band built-in Fairlight EQ.".into(),
                },
                TargetEffectOption {
                    id: "fcp.channel_eq".into(),
                    display_name: "Audio Effects → Channel EQ".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Built-in Logic/FCP Channel EQ.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_dynamics".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["dynamics".into(), "compressor".into(), "limiter".into(), "multiband".into()],
            source_display_name: "Dynamics / Compressor / Limiter".into(),
            category: EffectCategory::Audio,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Maps dynamics processing to target Fairlight/Audio dynamics.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.fairlight.dynamics".into(),
                    display_name: "Fairlight → Track Dynamics (Compressor / Limiter)".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Built-in per-track Fairlight Dynamics processor.".into(),
                },
            ],
        },
        EffectRule {
            id: "resolve_stabilization".into(),
            source_nle: NleFormat::DaVinciResolve,
            match_patterns: vec!["resolve.inspector.stabilization".into(), "stabilization".into()],
            source_display_name: "Stabilization".into(),
            category: EffectCategory::Stabilization,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Resolve per-frame stabilization analysis cannot be transferred verbatim; target NLE must re-analyze clip.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "premiere.warp_stabilizer".into(),
                    display_name: "Warp Stabilizer (AE.ADBE SubspaceStabilizer)".into(),
                    target_nle: NleFormat::PremierePro,
                    category: EffectCategory::Stabilization,
                    studio_only: false,
                    notes: "Attaches Warp Stabilizer onto the clip in Premiere (ready to click Analyze) and adds a review marker.".into(),
                },
                TargetEffectOption {
                    id: "fcp.stabilization".into(),
                    display_name: "Video Inspector → Stabilization (InertiaCam)".into(),
                    target_nle: NleFormat::FinalCutPro,
                    category: EffectCategory::Stabilization,
                    studio_only: false,
                    notes: "Built-in Final Cut Pro stabilizer.".into(),
                },
            ],
        },
        EffectRule {
            id: "pr_reverb".into(),
            source_nle: NleFormat::PremierePro,
            match_patterns: vec!["reverb".into()],
            source_display_name: "Studio / Convolution Reverb".into(),
            category: EffectCategory::Audio,
            fidelity: MappingFidelity::AskUserSuggested,
            notes: "Maps room reverb to Fairlight / FCP Reverb.".into(),
            targets: vec![
                TargetEffectOption {
                    id: "resolve.fairlight.reverb".into(),
                    display_name: "Fairlight FX → Reverb".into(),
                    target_nle: NleFormat::DaVinciResolve,
                    category: EffectCategory::Audio,
                    studio_only: false,
                    notes: "Built-in Fairlight Reverb.".into(),
                },
            ],
        },
    ]
}

/// Find the best matching catalog rule for an effect match_name / display_name.
pub fn find_catalog_rule<'a>(
    catalog: &'a [EffectRule],
    match_name: &str,
    display_name: &str,
) -> Option<&'a EffectRule> {
    let m_lower = match_name.to_ascii_lowercase();
    let d_lower = display_name.to_ascii_lowercase();
    catalog.iter().find(|rule| {
        rule.match_patterns
            .iter()
            .any(|pat| m_lower.contains(pat) || d_lower.contains(pat))
    })
}
