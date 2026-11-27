use flate2::write::GzEncoder;
use flate2::Compression;
use nle_core::{
    flatten_nested_and_multicam_clips, AlphaMode, BlendMode, Clip, ClipSourceKind, ConversionMessage,
    ConversionOptions, ConversionReport, DataLevels, LumetriData, Marker, MediaAsset, MediaKind,
    MotionTransform, NleError, NleFormat, NleResult, Project, ProjectWriter, Rational,
    RetimeInterpolation, Sequence, SequenceKind, Severity, Track, TrackKind, Transition,
    ADOBE_TICKS_PER_SECOND,
};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use uuid::Uuid;

// ── Adobe Premiere Pro 2026 ClassIDs & MediaType GUIDs ─────────────────

const MEDIA_TYPE_VIDEO: &str = "228cda18-3625-4d2d-951e-348879e4ed93";
const MEDIA_TYPE_AUDIO: &str = "80b8e3d5-6dca-4195-aefb-cb5f407ab009";
const MEDIA_TYPE_DATA: &str = "d8143ffe-eec4-4d2a-a909-d5f7bf094dc5";
const EDITING_MODE_GUID: &str = "795454d9-d3c2-429d-9474-923ab13b7018";
const DEF_MAPPING_ID: &str = "0551e2a0-6da1-4691-800e-b3ded5ca94f7";

const START_KF_SENTINEL: &str = "-91445760000000000";
const TICKS_PER_48KHZ_SAMPLE: i64 = 5_292_000;

const LAYOUT_MONO: &str = r#"[{"channellabel":0}]"#;
const LAYOUT_STEREO: &str = r#"[{"channellabel":100},{"channellabel":101}]"#;

const COLOR_SPACE_709_8BIT: &str = r#"{"baseColorProfile":{"colorProfileData":"AQAAAGQAAAA=","colorProfileName":"BT.709,8-bit,Display-Referred"},"baseProfileType":1,"colorSpaceMetadata":{"peakLuminance":100}}"#;
const COLOR_MGMT_PROJECT: &str =
    r#"{"enableLogColorManagement":2,"graphicsWhiteLuminance":203,"lutInterpolationMethod":1}"#;
const COLOR_MGMT_SEQ: &str = r#"{"autoToneMapEnabled":true,"enableLogColorManagement":2,"graphicsWhiteLuminance":203,"lutInterpolationMethod":1}"#;

pub struct PremiereWriter;

impl ProjectWriter for PremiereWriter {
    fn format(&self) -> NleFormat {
        NleFormat::PremierePro
    }

    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport> {
        write_prproj_file(project, output_path, options)
    }
}

/// Write a native gzip-compressed Adobe Premiere Pro 2026 `.prproj` file directly from the IR.
pub fn write_prproj_file(
    project: &Project,
    output_path: &Path,
    options: &ConversionOptions,
) -> NleResult<ConversionReport> {
    let (xml, mut report) = build_prproj_xml(project, options)?;
    report.output_path = output_path.to_string_lossy().into_owned();

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let file = File::create(output_path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder
        .write_all(xml.as_bytes())
        .map_err(|e| NleError::Archive(format!("Failed to gzip .prproj: {e}")))?;
    encoder
        .finish()
        .map_err(|e| NleError::Archive(format!("Failed to finish gzip .prproj: {e}")))?;

    Ok(report)
}

/// Build the full decompressed `<PremiereData Version="3">` XML document for Premiere Pro 2026.
pub fn build_prproj_xml(
    project: &Project,
    options: &ConversionOptions,
) -> NleResult<(String, ConversionReport)> {
    let mut pre_messages: Vec<ConversionMessage> = Vec::new();
    let project_cow: Cow<'_, Project> = if options.flatten_nested_and_multicam {
        let mut cloned = project.clone();
        let flat = flatten_nested_and_multicam_clips(&mut cloned);
        pre_messages.extend(flat.messages);
        Cow::Owned(cloned)
    } else {
        Cow::Borrowed(project)
    };
    let project = project_cow.as_ref();

    let sequences = collect_target_sequences(project, options);
    if sequences.is_empty() {
        return Err(NleError::InvalidFormat(
            "No sequences available to export to Premiere Pro (.prproj)".into(),
        ));
    }

    let mut ctx = PrprojBuilder::new(project, options);
    let mut report = ctx.build(project, &sequences)?;
    pre_messages.append(&mut report.messages);
    report.messages = pre_messages;
    Ok((ctx.finish_xml(), report))
}

/// Select requested sequences plus any transitively referenced nested/compound/multicam sequences
/// so `<VideoSequenceSource>` / `<AudioSequenceSource>` references never dangle.
fn collect_target_sequences<'a>(
    project: &'a Project,
    options: &ConversionOptions,
) -> Vec<&'a Sequence> {
    let by_id: HashMap<&str, &'a Sequence> = project
        .sequences
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();

    let mut chosen_ids: HashSet<&str> = if options.selected_sequence_ids.is_empty() {
        project.sequences.iter().map(|s| s.id.as_str()).collect()
    } else {
        options
            .selected_sequence_ids
            .iter()
            .map(|s| s.as_str())
            .filter(|id| by_id.contains_key(id))
            .collect()
    };

    // Transitive closure over nested / compound / multicam references.
    let mut queue: Vec<&str> = chosen_ids.iter().copied().collect();
    while let Some(sid) = queue.pop() {
        if let Some(seq) = by_id.get(sid) {
            for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
                for clip in &track.clips {
                    if clip.source_kind != ClipSourceKind::Media
                        && !clip.media_id.is_empty()
                        && by_id.contains_key(clip.media_id.as_str())
                        && chosen_ids.insert(clip.media_id.as_str())
                    {
                        queue.push(clip.media_id.as_str());
                    }
                }
            }
        }
    }

    project
        .sequences
        .iter()
        .filter(|s| chosen_ids.contains(s.id.as_str()))
        .collect()
}

// ── Internal object-graph records ──────────────────────────────────────

#[derive(Clone)]
struct MediaHandle {
    master_clip_uid: String,
    clip_project_item_uid: String,
    video_source_id: Option<u64>,
    audio_source_id: Option<u64>,
    markers_id: u64,
    is_stereo: bool,
    asset_duration_ticks: i64,
    pixel_aspect_ratio: Option<f64>,
}

#[derive(Clone)]
struct SeqHandle {
    seq_uid: String,
    master_clip_uid: String,
    clip_project_item_uid: String,
    video_source_id: u64,
    audio_source_id: u64,
    track_names: Vec<String>,
}

struct PrprojBuilder<'a> {
    options: &'a ConversionOptions,
    source_format: NleFormat,
    next_id: u64,
    elements: Vec<String>,
    /// Media lookup by asset ID, and fallback lookup by normalized path.
    media_by_id: HashMap<String, MediaHandle>,
    media_by_path: HashMap<String, MediaHandle>,
    /// Sequence lookup by original IR sequence ID.
    seq_handles: HashMap<String, SeqHandle>,
}

impl<'a> PrprojBuilder<'a> {
    fn new(project: &Project, options: &'a ConversionOptions) -> Self {
        Self {
            options,
            source_format: project.source_format,
            next_id: 2, // ObjectID 1 is reserved for <Project ObjectID="1">
            elements: Vec::new(),
            media_by_id: HashMap::new(),
            media_by_path: HashMap::new(),
            seq_handles: HashMap::new(),
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn push(&mut self, xml: String) {
        self.elements.push(xml);
    }

    fn finish_xml(self) -> String {
        let mut out = String::with_capacity(self.elements.len() * 256 + 1024);
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n");
        out.push_str("<PremiereData Version=\"3\">\n");
        out.push_str("\t<Project ObjectRef=\"1\" />\n");
        for el in self.elements {
            out.push_str(&el);
            if !el.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str("</PremiereData>\n");
        out
    }

    fn build(&mut self, project: &Project, sequences: &[&Sequence]) -> NleResult<ConversionReport> {
        let mut report = ConversionReport {
            output_path: String::new(),
            source_format: project.source_format,
            target_format: NleFormat::PremierePro,
            sequences_converted: 0,
            video_clips_converted: 0,
            audio_clips_converted: 0,
            media_assets_linked: 0,
            bins_created: 0,
            markers_written: 0,
            grades_attached: 0,
            effect_markers_injected: 0,
            messages: Vec::new(),
        };

        let root_bin_uid = Uuid::new_v4().to_string();
        let project_guid = Uuid::new_v4().to_string();

        // 1. Pre-assign UUIDs & sequence source IDs so nested/multicam clips can reference any sequence
        // regardless of sequence ordering.
        for seq in sequences {
            let seq_uid = normalize_uuid(&seq.id);
            let master_clip_uid = Uuid::new_v4().to_string();
            let clip_project_item_uid = Uuid::new_v4().to_string();
            let video_source_id = self.alloc_id();
            let audio_source_id = self.alloc_id();
            let track_names = seq.video_tracks.iter().map(|t| t.name.clone()).collect();
            self.seq_handles.insert(
                seq.id.clone(),
                SeqHandle {
                    seq_uid,
                    master_clip_uid,
                    clip_project_item_uid,
                    video_source_id,
                    audio_source_id,
                    track_names,
                },
            );
        }

        // Open main timelines (non-compound / non-multicam first, falling back to first sequence)
        let mut open_seq_uids: Vec<String> = sequences
            .iter()
            .filter(|s| s.kind == SequenceKind::Timeline)
            .filter_map(|s| self.seq_handles.get(&s.id).map(|h| h.seq_uid.clone()))
            .collect();
        if open_seq_uids.is_empty() {
            if let Some(first) = sequences.first().and_then(|s| self.seq_handles.get(&s.id)) {
                open_seq_uids.push(first.seq_uid.clone());
            }
        }

        // Build merged custom metadata schema (declared project schema + any asset/bin custom keys)
        let mut custom_schema = project.custom_metadata_schema.clone();
        for asset in &project.media_assets {
            for key in asset.custom_metadata.keys() {
                if is_valid_xml_tag_name(key) {
                    custom_schema
                        .entry(key.clone())
                        .or_insert_with(|| key.replace('.', " "));
                }
            }
        }
        for bin in &project.bins {
            for key in bin.custom_metadata.keys() {
                if is_valid_xml_tag_name(key) {
                    custom_schema
                        .entry(key.clone())
                        .or_insert_with(|| key.replace('.', " "));
                }
            }
        }

        // 2. Emit Project root + Settings + CompileSettings + ScratchDiskSettings + Ingest + Workspace
        let primary_fps = sequences
            .first()
            .map(|s| s.frame_rate)
            .unwrap_or(Rational::FPS_25);
        self.emit_project_scaffold(
            &root_bin_uid,
            &project_guid,
            project.scale_to_frame_size,
            primary_fps,
            &open_seq_uids,
            &custom_schema,
        );

        // 3. Emit Media assets + MasterClips + ClipProjectItems
        // Collect declared assets plus any inline clip media paths not listed in media_assets.
        let mut all_assets: Vec<MediaAsset> = project.media_assets.clone();
        let mut seen_keys: HashSet<String> = all_assets
            .iter()
            .flat_map(|a| {
                let mut v = Vec::new();
                if !a.id.is_empty() {
                    v.push(a.id.clone());
                }
                if !a.path.is_empty() {
                    v.push(a.path.clone());
                }
                v
            })
            .collect();

        for seq in sequences {
            for track in seq.video_tracks.iter().chain(seq.audio_tracks.iter()) {
                for clip in &track.clips {
                    if clip.source_kind != ClipSourceKind::Media {
                        continue;
                    }
                    let id_key = if !clip.media_id.is_empty() {
                        clip.media_id.clone()
                    } else {
                        clip.media_path.clone()
                    };
                    if id_key.is_empty()
                        || seen_keys.contains(&id_key)
                        || (!clip.media_path.is_empty() && seen_keys.contains(&clip.media_path))
                    {
                        continue;
                    }
                    seen_keys.insert(id_key.clone());
                    if !clip.media_path.is_empty() {
                        seen_keys.insert(clip.media_path.clone());
                    }
                    let dur = clip
                        .source_out_frame
                        .max(clip.source_in_frame + clip.duration_frames())
                        .max(1);
                    all_assets.push(MediaAsset {
                        id: id_key,
                        name: clip.name.clone(),
                        path: clip.media_path.clone(),
                        media_kind: clip.media_kind,
                        width: clip.source_width,
                        height: clip.source_height,
                        frame_rate: clip.source_fps,
                        duration_frames: dur,
                        source_tc_frame: 0,
                        source_tc_string: "00:00:00:00".into(),
                        audio_channels: 2,
                        audio_sample_rate: 48_000,
                        color_space: None,
                        video_codec: None,
                        audio_codec: None,
                        alpha_mode: AlphaMode::Default,
                        data_levels: DataLevels::Auto,
                        pixel_aspect_ratio: None,
                        metadata: BTreeMap::new(),
                        custom_metadata: BTreeMap::new(),
                    });
                }
            }
        }

        // Determine which assets have audio clips placed on any sequence so video assets also get AudioMediaSource.
        let mut media_used_as_audio: HashSet<String> = HashSet::new();
        for seq in sequences {
            for at in &seq.audio_tracks {
                for clip in &at.clips {
                    if clip.source_kind == ClipSourceKind::Media {
                        if !clip.media_id.is_empty() {
                            media_used_as_audio.insert(clip.media_id.clone());
                        }
                        if !clip.media_path.is_empty() {
                            media_used_as_audio.insert(clip.media_path.clone());
                        }
                    }
                }
            }
        }

        for asset in &all_assets {
            let needs_audio = asset.media_kind == MediaKind::Audio
                || asset.audio_channels > 0
                || media_used_as_audio.contains(&asset.id)
                || media_used_as_audio.contains(&asset.path);
            let handle = self.emit_media_asset(asset, needs_audio);
            if !asset.id.is_empty() {
                self.media_by_id.insert(asset.id.clone(), handle.clone());
            }
            if !asset.path.is_empty() {
                self.media_by_path.insert(asset.path.clone(), handle);
            }
            report.media_assets_linked += 1;
        }

        // 4. Emit Sequences + their MasterClips + ClipProjectItems
        for seq in sequences {
            self.emit_sequence(seq, &mut report);
            report.sequences_converted += 1;
        }

        // 5. Emit BinProjectItem hierarchy and RootProjectItem
        self.emit_bins_and_root(project, sequences, &root_bin_uid, &mut report);

        Ok(report)
    }

    fn emit_project_scaffold(
        &mut self,
        root_bin_uid: &str,
        project_guid: &str,
        scale_to_frame_size: bool,
        primary_fps: Rational,
        open_seq_uids: &[String],
        custom_schema: &BTreeMap<String, String>,
    ) {
        let proj_settings_id = self.alloc_id();
        let movie_cs_id = self.alloc_id();
        let still_cs_id = self.alloc_id();
        let audio_cs_id = self.alloc_id();
        let custom_cs_id = self.alloc_id();
        let preview_cs_id = self.alloc_id();
        let scratch_id = self.alloc_id();
        let ingest_id = self.alloc_id();
        let workspace_id = self.alloc_id();

        let mut open_seq_props = String::new();
        for (i, uid) in open_seq_uids.iter().enumerate() {
            open_seq_props.push_str(&format!(
                "\t\t\t\t<MZ.PrefixKey.OpenSequenceGuidList.{idx}>{uid}</MZ.PrefixKey.OpenSequenceGuidList.{idx}>\n",
                idx = i + 1
            ));
        }

        let schema_prop = if custom_schema.is_empty() {
            String::new()
        } else {
            let mut xmp = String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<xmp_definitions>\n\t<xmp_schema namespace=\"http://ns.adobe.com/premierePrivateProjectMetaData/1.0/\" prefix=\"premierePrivateProjectMetaData\" label=\"$$$/Premiere/PrivateProProjectMetaData_label=Premiere Project Metadata\">\n",
            );
            for (name, label) in custom_schema {
                if is_valid_xml_tag_name(name) {
                    xmp.push_str(&format!(
                        "\t\t<xmp_property category=\"external\" name=\"{}\" type=\"text\" label=\"{}\" />\n",
                        xml_escape(name),
                        xml_escape(label)
                    ));
                }
            }
            xmp.push_str("\t</xmp_schema>\n</xmp_definitions>");
            format!(
                "\t\t\t\t<Project.Metadata.Schema>{}</Project.Metadata.Schema>\n",
                xml_escape(&xmp)
            )
        };

        self.push(format!(
            "\t<Project ObjectID=\"1\" ClassID=\"62ad66dd-0dcd-42da-a660-6d8fbde94876\" Version=\"45\">\n\
             \t\t<Node Version=\"1\">\n\
             \t\t\t<Properties Version=\"1\">\n\
             \t\t\t\t<MZ.Project.GUID>{project_guid}</MZ.Project.GUID>\n\
             \t\t\t\t<MZ.Project.ApplicationID>PremierePro</MZ.Project.ApplicationID>\n\
             \t\t\t\t<TL.PJSnappingState>true</TL.PJSnappingState>\n\
             {schema_prop}\
             {open_seq_props}\
             \t\t\t</Properties>\n\
             \t\t</Node>\n\
             \t\t<RootProjectItem ObjectURef=\"{root_bin_uid}\" />\n\
             \t\t<ProjectSettings ObjectRef=\"{proj_settings_id}\" />\n\
             \t\t<MovieCompileSettings ObjectRef=\"{movie_cs_id}\" />\n\
             \t\t<StillCompileSettings ObjectRef=\"{still_cs_id}\" />\n\
             \t\t<AudioCompileSettings ObjectRef=\"{audio_cs_id}\" />\n\
             \t\t<CustomCompileSettings ObjectRef=\"{custom_cs_id}\" />\n\
             \t\t<VideoPreviewCompileSettings ObjectRef=\"{preview_cs_id}\" />\n\
             \t\t<ScratchDiskSettings ObjectRef=\"{scratch_id}\" />\n\
             \t\t<IngestSettings ObjectRef=\"{ingest_id}\" />\n\
             \t\t<ProjectWorkspace ObjectRef=\"{workspace_id}\" />\n\
             \t\t<NextID>1000001</NextID>\n\
             \t</Project>"
        ));

        // ProjectSettings children
        let ps_vs_id = self.alloc_id();
        let ps_as_id = self.alloc_id();
        let ps_vcs_id = self.alloc_id();
        let ps_acs_id = self.alloc_id();
        let capture_id = self.alloc_id();
        let def_seq_id = self.alloc_id();
        let td = video_time_display_format(primary_fps);
        let scale_str = if scale_to_frame_size { "true" } else { "false" };

        self.push(format!(
            "\t<ProjectSettings ObjectID=\"{proj_settings_id}\" ClassID=\"50c16708-a1a1-4d2f-98d5-4e283ae28353\" Version=\"21\">\n\
             \t\t<VideoSettings ObjectRef=\"{ps_vs_id}\" />\n\
             \t\t<AudioSettings ObjectRef=\"{ps_as_id}\" />\n\
             \t\t<VideoCompileSettings ObjectRef=\"{ps_vcs_id}\" />\n\
             \t\t<AudioCompileSettings ObjectRef=\"{ps_acs_id}\" />\n\
             \t\t<CaptureSettings ObjectRef=\"{capture_id}\" />\n\
             \t\t<DefaultSequenceSettings ObjectRef=\"{def_seq_id}\" />\n\
             \t\t<ColorManagementSettings>{COLOR_MGMT_PROJECT}</ColorManagementSettings>\n\
             \t\t<AudioTimeDisplay>200</AudioTimeDisplay>\n\
             \t\t<VideoTimeDisplay>{td}</VideoTimeDisplay>\n\
             \t\t<VideoTimeDisplayInitial>{td}</VideoTimeDisplayInitial>\n\
             \t\t<ActionSafeWidth>10</ActionSafeWidth>\n\
             \t\t<ActionSafeHeight>10</ActionSafeHeight>\n\
             \t\t<TitleSafeWidth>20</TitleSafeWidth>\n\
             \t\t<TitleSafeHeight>20</TitleSafeHeight>\n\
             \t\t<ShouldScaleMedia>{scale_str}</ShouldScaleMedia>\n\
             \t\t<EditingModeID>00000000-0000-0000-0000-000000000000</EditingModeID>\n\
             \t\t<PreviewFileFormatID>00000000-0000-0000-0000-000000000000</PreviewFileFormatID>\n\
             \t\t<UsePreviewCache>false</UsePreviewCache>\n\
             \t\t<ColorAwareEffectsEnabled>0</ColorAwareEffectsEnabled>\n\
             \t</ProjectSettings>"
        ));

        self.emit_empty_video_audio_settings(ps_vs_id, ps_as_id);
        self.emit_video_audio_compile_settings(ps_vcs_id, ps_acs_id);

        for cs_id in [
            movie_cs_id,
            still_cs_id,
            audio_cs_id,
            custom_cs_id,
            preview_cs_id,
        ] {
            let vcs = self.alloc_id();
            let acs = self.alloc_id();
            self.push(format!(
                "\t<CompileSettings ObjectID=\"{cs_id}\" ClassID=\"18a35d66-597e-4157-b783-938b5bec3547\" Version=\"4\">\n\
                 \t\t<VideoCompileSettings ObjectRef=\"{vcs}\" />\n\
                 \t\t<AudioCompileSettings ObjectRef=\"{acs}\" />\n\
                 \t\t<CompilerClassIDFourCC>0</CompilerClassIDFourCC>\n\
                 \t\t<CompilerFourCC>0</CompilerFourCC>\n\
                 \t\t<ExportVideo>true</ExportVideo>\n\
                 \t\t<ExportAudio>true</ExportAudio>\n\
                 \t\t<AddToProjectWhenFinished>true</AddToProjectWhenFinished>\n\
                 \t\t<BeepWhenFinished>false</BeepWhenFinished>\n\
                 \t\t<ExportWorkAreaOnly>false</ExportWorkAreaOnly>\n\
                 \t\t<EmbedProjectLink>false</EmbedProjectLink>\n\
                 \t</CompileSettings>"
            ));
            self.emit_video_audio_compile_settings(vcs, acs);
        }

        self.push(format!(
            "\t<ScratchDiskSettings ObjectID=\"{scratch_id}\" ClassID=\"4c6ed82b-a81c-4df1-8bd0-750504c4b560\" Version=\"4\">\n\
             \t\t<AudioPreviewLocation0>SameAsProject</AudioPreviewLocation0>\n\
             \t\t<VideoPreviewLocation0>SameAsProject</VideoPreviewLocation0>\n\
             \t\t<CapturedVideoLocation0>SameAsProject</CapturedVideoLocation0>\n\
             \t\t<CapsuleMediaLocation0>SameAsProject</CapsuleMediaLocation0>\n\
             \t\t<CCLibrariesLocation0>SameAsProject</CCLibrariesLocation0>\n\
             \t\t<DVDEncodingLocation0>SameAsProject</DVDEncodingLocation0>\n\
             \t\t<AutoSaveLocation0>SameAsProject</AutoSaveLocation0>\n\
             \t\t<TransferMediaLocation0>SameAsProject</TransferMediaLocation0>\n\
             \t</ScratchDiskSettings>"
        ));
        self.push(format!(
            "\t<IngestSettings ObjectID=\"{ingest_id}\" ClassID=\"2db8f76b-2c37-48ee-925d-9a4f7278152d\" Version=\"2\">\n\
             \t\t<Action>copy</Action>\n\
             \t\t<Enabled>false</Enabled>\n\
             \t</IngestSettings>"
        ));
        self.push(format!(
            "\t<WorkspaceSettings ObjectID=\"{workspace_id}\" ClassID=\"c4372273-e1aa-4683-98aa-a2ceadf3066c\" Version=\"1\" />"
        ));
        self.push(format!(
            "\t<DummyCaptureSettings ObjectID=\"{capture_id}\" ClassID=\"328c2aa2-47f9-4211-805b-b6a6dbd4ca29\" Version=\"1\" />"
        ));
        self.push(format!(
            "\t<DefaultSequenceSettings ObjectID=\"{def_seq_id}\" ClassID=\"567bdf53-d6d9-4d61-b2f1-f4834bebea9b\" Version=\"2\">\n\
             \t\t<TotalVideoTracks>1</TotalVideoTracks>\n\
             \t\t<DefaultAudioStandardMonoTracks>0</DefaultAudioStandardMonoTracks>\n\
             \t\t<DefaultAudioStandardStereoTracks>1</DefaultAudioStandardStereoTracks>\n\
             \t\t<DefaultAudioStandard51Tracks>0</DefaultAudioStandard51Tracks>\n\
             \t\t<DefaultAudioSubmixMonoTracks>0</DefaultAudioSubmixMonoTracks>\n\
             \t\t<DefaultAudioSubmixStereoTracks>0</DefaultAudioSubmixStereoTracks>\n\
             \t\t<DefaultAudioSubmix51Tracks>0</DefaultAudioSubmix51Tracks>\n\
             \t</DefaultSequenceSettings>"
        ));
    }

    fn emit_empty_video_audio_settings(&mut self, vs_id: u64, as_id: u64) {
        self.push(format!(
            "\t<VideoSettings ObjectID=\"{vs_id}\" ClassID=\"58474264-30c4-43a2-bba5-dc0812df8a3a\" Version=\"10\" />"
        ));
        self.push(format!(
            "\t<AudioSettings ObjectID=\"{as_id}\" ClassID=\"6baf5521-b132-4634-840e-13cec5bc86a4\" Version=\"8\" />"
        ));
    }

    fn emit_video_audio_compile_settings(&mut self, vcs_id: u64, acs_id: u64) {
        let vs_id = self.alloc_id();
        let as_id = self.alloc_id();
        self.emit_empty_video_audio_settings(vs_id, as_id);
        self.push(format!(
            "\t<VideoCompileSettings ObjectID=\"{vcs_id}\" ClassID=\"db372db5-7de2-4d3c-98ae-f42659d77b22\" Version=\"9\">\n\
             \t\t<VideoSettings ObjectRef=\"{vs_id}\" />\n\
             \t\t<Compressor>1685480224</Compressor>\n\
             \t\t<VideoCompilerClassIDFourCC>1061109567</VideoCompilerClassIDFourCC>\n\
             \t\t<VideoFileTypeFourCC>1299148630</VideoFileTypeFourCC>\n\
             \t\t<Depth>24</Depth>\n\
             \t\t<RenderDepth>0</RenderDepth>\n\
             \t\t<Aspect43>false</Aspect43>\n\
             \t\t<Quality>100</Quality>\n\
             \t\t<UseDataRate>false</UseDataRate>\n\
             \t\t<DataRate>3500</DataRate>\n\
             \t\t<ForceRecompress>true</ForceRecompress>\n\
             \t\t<ForceRecompressValue>2</ForceRecompressValue>\n\
             \t\t<Deinterlace>false</Deinterlace>\n\
             \t\t<IgnoreVideoFilters>false</IgnoreVideoFilters>\n\
             \t\t<OptimizeStills>false</OptimizeStills>\n\
             \t\t<FramesAtMarkers>false</FramesAtMarkers>\n\
             \t\t<RealTimePreview>true</RealTimePreview>\n\
             \t\t<VideoFieldType>0</VideoFieldType>\n\
             \t\t<DoKeyframeEveryNFrames>false</DoKeyframeEveryNFrames>\n\
             \t\t<DoKeyframeEveryNFramesValue>0</DoKeyframeEveryNFramesValue>\n\
             \t\t<AddKeyframesAtMarkers>false</AddKeyframesAtMarkers>\n\
             \t\t<AddKeyframesAtEdits>false</AddKeyframesAtEdits>\n\
             \t\t<RelativeFrameSize>1</RelativeFrameSize>\n\
             \t</VideoCompileSettings>"
        ));
        self.push(format!(
            "\t<AudioCompileSettings ObjectID=\"{acs_id}\" ClassID=\"34b10007-ab6d-49a7-bac5-7b60d919e387\" Version=\"6\">\n\
             \t\t<AudioSettings ObjectRef=\"{as_id}\" />\n\
             \t\t<Interleave>1</Interleave>\n\
             \t\t<SampleType>3</SampleType>\n\
             \t\t<Compressor>1380013856</Compressor>\n\
             \t</AudioCompileSettings>"
        ));
    }

    fn emit_media_asset(&mut self, asset: &MediaAsset, has_audio: bool) -> MediaHandle {
        let media_uid = Uuid::new_v4().to_string();
        let master_clip_uid = Uuid::new_v4().to_string();
        let clip_project_item_uid = Uuid::new_v4().to_string();

        let fps = if asset.frame_rate.num > 0 && asset.frame_rate.den > 0 {
            asset.frame_rate
        } else {
            Rational::FPS_25
        };
        let frame_ticks = fps.frames_to_adobe_ticks(1);
        let dur_frames = asset.duration_frames.max(250);
        let dur_ticks = fps.frames_to_adobe_ticks(dur_frames);
        let start_tc_ticks = fps.frames_to_adobe_ticks(asset.source_tc_frame.max(0));
        let is_video = asset.media_kind != MediaKind::Audio;
        let is_stereo = asset.audio_channels != 1;
        let audio_layout = if is_stereo { LAYOUT_STEREO } else { LAYOUT_MONO };
        let audio_channel_type = if is_stereo { 1 } else { 0 };
        let sample_rate = asset.audio_sample_rate.max(8_000) as i64;
        let sample_ticks = ADOBE_TICKS_PER_SECOND / sample_rate;
        let w = if asset.width > 0 { asset.width } else { 1920 };
        let h = if asset.height > 0 { asset.height } else { 1080 };

        let video_stream_id = if is_video {
            let vs_id = self.alloc_id();
            let (ignore_alpha_str, alpha_type_num) = match asset.alpha_mode {
                AlphaMode::Straight => ("false", 1),
                AlphaMode::Premultiplied => ("false", 2),
                AlphaMode::None => ("true", 0),
                AlphaMode::Default => ("true", 3),
            };
            let par_xml = if let Some(par) = asset.pixel_aspect_ratio {
                if par.is_finite() && par > 0.0 && (par - 1.0).abs() > 1e-4 {
                    let num = (par * 10_000.0).round() as i64;
                    format!(
                        "\t\t<IsPAROverridden>true</IsPAROverridden>\n\
                         \t\t<PixelAspectRatio>{num},10000</PixelAspectRatio>\n"
                    )
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            self.push(format!(
                "\t<VideoStream ObjectID=\"{vs_id}\" ClassID=\"a36e4719-3ec6-4a0c-ab11-8b4aab377aa5\" Version=\"23\">\n\
                 \t\t<Duration>{dur_ticks}</Duration>\n\
                 \t\t<IgnoreAlpha>{ignore_alpha_str}</IgnoreAlpha>\n\
                 \t\t<FieldTypeIsUncertain>true</FieldTypeIsUncertain>\n\
                 \t\t<OriginalImageOrientationType>1</OriginalImageOrientationType>\n\
                 \t\t<FrameRate>{frame_ticks}</FrameRate>\n\
                 \t\t<FrameRect>0,0,{w},{h}</FrameRect>\n\
                 {par_xml}\
                 \t\t<AlphaType>{alpha_type_num}</AlphaType>\n\
                 \t</VideoStream>"
            ));
            Some(vs_id)
        } else {
            None
        };

        let audio_stream_id = if has_audio {
            let as_id = self.alloc_id();
            self.push(format!(
                "\t<AudioStream ObjectID=\"{as_id}\" ClassID=\"0b5cf52f-2b85-4863-890b-8844b64ecfe9\" Version=\"8\">\n\
                 \t\t<AudioChannelLayout>{audio_layout}</AudioChannelLayout>\n\
                 \t\t<Duration>{dur_ticks}</Duration>\n\
                 \t\t<FrameRate>{sample_ticks}</FrameRate>\n\
                 \t</AudioStream>"
            ));
            Some(as_id)
        } else {
            None
        };

        let escaped_path = xml_escape(&asset.path);
        let display_name = if asset.name.is_empty() {
            Path::new(&asset.path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Media")
                .to_string()
        } else {
            asset.name.clone()
        };
        let escaped_name = xml_escape(&display_name);

        let mut media_xml = format!(
            "\t<Media ObjectUID=\"{media_uid}\" ClassID=\"7a5c103e-f3ac-4391-b6b4-7cc3d2f9a7ff\" Version=\"30\">\n"
        );
        if let Some(as_id) = audio_stream_id {
            media_xml.push_str(&format!("\t\t<AudioStream ObjectRef=\"{as_id}\" />\n"));
        }
        if let Some(vs_id) = video_stream_id {
            media_xml.push_str(&format!("\t\t<VideoStream ObjectRef=\"{vs_id}\" />\n"));
        }
        media_xml.push_str(&format!(
            "\t\t<FileKey>{}</FileKey>\n\
             \t\t<ConformedAudioRate>{sample_ticks}</ConformedAudioRate>\n\
             \t\t<ActualMediaFilePath>{escaped_path}</ActualMediaFilePath>\n\
             \t\t<FilePath>{escaped_path}</FilePath>\n\
             \t\t<Title>{escaped_name}</Title>\n",
            Uuid::new_v4()
        ));
        if start_tc_ticks > 0 {
            media_xml.push_str(&format!(
                "\t\t<AlternateStart>{start_tc_ticks}</AlternateStart>\n\
                 \t\t<UseAlternateStart>true</UseAlternateStart>\n"
            ));
        }
        media_xml.push_str("\t</Media>");
        self.push(media_xml);

        // MediaSources
        let video_source_id = if is_video {
            let vms_id = self.alloc_id();
            self.push(format!(
                "\t<VideoMediaSource ObjectID=\"{vms_id}\" ClassID=\"e64ddf74-8fac-4682-8aa8-0e0ca2248949\" Version=\"2\">\n\
                 \t\t<MediaSource Version=\"4\">\n\
                 \t\t\t<Content Version=\"10\" />\n\
                 \t\t\t<Media ObjectURef=\"{media_uid}\" />\n\
                 \t\t</MediaSource>\n\
                 \t\t<OriginalDuration>{dur_ticks}</OriginalDuration>\n\
                 \t</VideoMediaSource>"
            ));
            Some(vms_id)
        } else {
            None
        };

        let audio_source_id = if has_audio {
            let ams_id = self.alloc_id();
            self.push(format!(
                "\t<AudioMediaSource ObjectID=\"{ams_id}\" ClassID=\"f588da05-fc2a-4fbc-9383-74d653b379e3\" Version=\"2\">\n\
                 \t\t<MediaSource Version=\"4\">\n\
                 \t\t\t<Content Version=\"10\" />\n\
                 \t\t\t<Media ObjectURef=\"{media_uid}\" />\n\
                 \t\t</MediaSource>\n\
                 \t\t<OriginalDuration>{dur_ticks}</OriginalDuration>\n\
                 \t</AudioMediaSource>"
            ));
            Some(ams_id)
        } else {
            None
        };

        // Shared empty Markers container for this MasterClip
        let markers_id = self.emit_markers(&[], fps);

        // ClipLoggingInfo
        let log_id = self.alloc_id();
        let td = video_time_display_format(fps);
        let media_out_ticks = start_tc_ticks + dur_ticks;
        let log_rate = if is_video { frame_ticks } else { sample_ticks };
        let mut extra_log = String::new();
        if let Some(desc) = asset.metadata.get("Description").filter(|s| !s.is_empty()) {
            extra_log.push_str(&format!("\t\t<Description>{}</Description>\n", xml_escape(desc)));
        }
        if let Some(tape) = asset.metadata.get("Tape Name").filter(|s| !s.is_empty()) {
            extra_log.push_str(&format!("\t\t<TapeName>{}</TapeName>\n", xml_escape(tape)));
        }
        self.push(format!(
            "\t<ClipLoggingInfo ObjectID=\"{log_id}\" ClassID=\"77ab7fdd-dcdf-465d-9906-7a330ca1e738\" Version=\"10\">\n\
             \t\t<ClipName>{escaped_name}</ClipName>\n\
             {extra_log}\
             \t\t<MediaInPoint>{start_tc_ticks}</MediaInPoint>\n\
             \t\t<MediaOutPoint>{media_out_ticks}</MediaOutPoint>\n\
             \t\t<MediaFrameRate>{log_rate}</MediaFrameRate>\n\
             \t\t<TimecodeFormat>{td}</TimecodeFormat>\n\
             \t</ClipLoggingInfo>"
        ));

        // MasterClip clips + audio channel groups
        let mut master_clips: Vec<u64> = Vec::new();
        if let Some(vms_id) = video_source_id {
            let vc_id = self.alloc_id();
            self.push(format!(
                "\t<VideoClip ObjectID=\"{vc_id}\" ClassID=\"9308dbef-2440-4acb-9ab2-953b9a4e82ec\" Version=\"11\">\n\
                 \t\t<Clip Version=\"18\">\n\
                 \t\t\t<MarkerOwner Version=\"1\"><Markers ObjectRef=\"{markers_id}\" /></MarkerOwner>\n\
                 \t\t\t<Source ObjectRef=\"{vms_id}\" />\n\
                 \t\t\t<ClipID>{}</ClipID>\n\
                 \t\t\t<InUse>false</InUse>\n\
                 \t\t\t<InPoint>0</InPoint>\n\
                 \t\t\t<OutPoint>{dur_ticks}</OutPoint>\n\
                 \t\t</Clip>\n\
                 \t</VideoClip>",
                Uuid::new_v4()
            ));
            master_clips.push(vc_id);
        }

        let mut master_audio_chain_id = None;
        let mut master_channel_groups_id = None;
        if let Some(ams_id) = audio_source_id {
            let ac_id = self.alloc_id();
            let sec_ids = self.emit_secondary_contents(ams_id, if is_stereo { 2 } else { 1 }, 0);
            let mut sec_xml = String::new();
            for (idx, sid) in sec_ids.iter().enumerate() {
                sec_xml.push_str(&format!(
                    "\t\t\t<SecondaryContentItem Index=\"{idx}\" ObjectRef=\"{sid}\" />\n"
                ));
            }
            self.push(format!(
                "\t<AudioClip ObjectID=\"{ac_id}\" ClassID=\"b8830d03-de02-41ee-84ec-fe566dc70cd9\" Version=\"8\">\n\
                 \t\t<Clip Version=\"18\">\n\
                 \t\t\t<MarkerOwner Version=\"1\"><Markers ObjectRef=\"{markers_id}\" /></MarkerOwner>\n\
                 \t\t\t<Source ObjectRef=\"{ams_id}\" />\n\
                 \t\t\t<ClipID>{}</ClipID>\n\
                 \t\t\t<InUse>false</InUse>\n\
                 \t\t\t<InPoint>0</InPoint>\n\
                 \t\t\t<OutPoint>{dur_ticks}</OutPoint>\n\
                 \t\t</Clip>\n\
                 \t\t<SecondaryContents Version=\"1\">\n\
                 {sec_xml}\
                 \t\t</SecondaryContents>\n\
                 \t\t<AudioChannelLayout>{audio_layout}</AudioChannelLayout>\n\
                 \t</AudioClip>",
                Uuid::new_v4()
            ));
            master_clips.push(ac_id);

            let ach_id = self.alloc_id();
            self.push(format!(
                "\t<AudioComponentChain ObjectID=\"{ach_id}\" ClassID=\"3cb131d1-d3c0-47ae-a19a-bdf75ea11674\" Version=\"4\">\n\
                 \t\t<DefaultVol>true</DefaultVol>\n\
                 \t\t<DefaultVolumeComponentID>1</DefaultVolumeComponentID>\n\
                 \t\t<DefaultChannelVolumeComponentID>2</DefaultChannelVolumeComponentID>\n\
                 \t\t<ComponentChain Version=\"3\" />\n\
                 \t\t<AudioChannelLayout>{audio_layout}</AudioChannelLayout>\n\
                 \t\t<ChannelType>{audio_channel_type}</ChannelType>\n\
                 \t</AudioComponentChain>"
            ));
            master_audio_chain_id = Some(ach_id);
            master_channel_groups_id = Some(self.emit_channel_groups(is_stereo));
        }

        let mut mc_xml = format!(
            "\t<MasterClip ObjectUID=\"{master_clip_uid}\" ClassID=\"fb11c33a-b0a9-4465-aa94-b6d5db2628cf\" Version=\"12\">\n\
             \t\t<LoggingInfo ObjectRef=\"{log_id}\" />\n"
        );
        if let Some(ach_id) = master_audio_chain_id {
            mc_xml.push_str(&format!(
                "\t\t<AudioComponentChains Version=\"1\">\n\
                 \t\t\t<AudioComponentChain Index=\"0\" ObjectRef=\"{ach_id}\" />\n\
                 \t\t</AudioComponentChains>\n"
            ));
        }
        mc_xml.push_str("\t\t<Clips Version=\"1\">\n");
        for (idx, cid) in master_clips.iter().enumerate() {
            mc_xml.push_str(&format!("\t\t\t<Clip Index=\"{idx}\" ObjectRef=\"{cid}\" />\n"));
        }
        mc_xml.push_str("\t\t</Clips>\n");
        if let Some(cg_id) = master_channel_groups_id {
            mc_xml.push_str(&format!(
                "\t\t<AudioClipChannelGroups ObjectRef=\"{cg_id}\" />\n"
            ));
        }
        mc_xml.push_str(&format!(
            "\t\t<Name>{escaped_name}</Name>\n\
             \t\t<DefMappingID>{DEF_MAPPING_ID}</DefMappingID>\n\
             \t\t<MasterClipChangeVersion>1</MasterClipChangeVersion>\n\
             \t</MasterClip>"
        ));
        self.push(mc_xml);

        let default_label = if is_video {
            "BE.Prefs.LabelColors.1"
        } else {
            "BE.Prefs.LabelColors.2"
        };
        let mut cpi_props = format!(
            "\t\t\t\t\t<Column.PropertyText.Label>{default_label}</Column.PropertyText.Label>\n"
        );
        for (k, v) in &asset.metadata {
            if v.is_empty() {
                continue;
            }
            if let Some(col_tag) = canonical_meta_to_premiere_column_tag(k) {
                cpi_props.push_str(&format!(
                    "\t\t\t\t\t<{col_tag}>{}</{col_tag}>\n",
                    xml_escape(v)
                ));
            }
        }
        for (k, v) in &asset.custom_metadata {
            if !v.is_empty() && is_valid_xml_tag_name(k) {
                cpi_props.push_str(&format!("\t\t\t\t\t<{k}>{}</{k}>\n", xml_escape(v)));
            }
        }
        self.push(format!(
            "\t<ClipProjectItem ObjectUID=\"{clip_project_item_uid}\" ClassID=\"cb4e0ed7-aca1-4171-8525-e3658dec06dd\" Version=\"1\">\n\
             \t\t<ProjectItem Version=\"1\">\n\
             \t\t\t<Node Version=\"1\">\n\
             \t\t\t\t<Properties Version=\"1\">\n\
             {cpi_props}\
             \t\t\t\t</Properties>\n\
             \t\t\t</Node>\n\
             \t\t\t<Name>{escaped_name}</Name>\n\
             \t\t</ProjectItem>\n\
             \t\t<MasterClip ObjectURef=\"{master_clip_uid}\" />\n\
             \t</ClipProjectItem>"
        ));

        MediaHandle {
            master_clip_uid,
            clip_project_item_uid,
            video_source_id,
            audio_source_id,
            markers_id,
            is_stereo,
            asset_duration_ticks: dur_ticks,
            pixel_aspect_ratio: asset.pixel_aspect_ratio,
        }
    }

    fn emit_secondary_contents(
        &mut self,
        source_id: u64,
        channel_count: u32,
        start_channel_index: u32,
    ) -> Vec<u64> {
        let mut ids = Vec::with_capacity(channel_count as usize);
        for offset in 0..channel_count {
            let sid = self.alloc_id();
            let ch_idx = start_channel_index + offset;
            self.push(format!(
                "\t<SecondaryContent ObjectID=\"{sid}\" ClassID=\"f9d004b5-cb04-4e2f-af6f-64fadc2c4be9\" Version=\"1\">\n\
                 \t\t<Content ObjectRef=\"{source_id}\" />\n\
                 \t\t<ChannelIndex>{ch_idx}</ChannelIndex>\n\
                 \t</SecondaryContent>"
            ));
            ids.push(sid);
        }
        ids
    }

    fn emit_channel_groups(&mut self, is_stereo: bool) -> u64 {
        let ch_count = if is_stereo { 2u32 } else { 1u32 };
        let ch_type = if is_stereo { 1u32 } else { 0u32 };
        let mut ch_ids = Vec::new();
        for ch in 0..ch_count {
            let cid = self.alloc_id();
            self.push(format!(
                "\t<ClipChannelSerializer ObjectID=\"{cid}\" ClassID=\"5c89aa7a-89a6-4483-becd-f2b1def42316\" Version=\"1\">\n\
                 \t\t<mSourceChannelIndex>{ch}</mSourceChannelIndex>\n\
                 \t\t<SourceClipIndex>0</SourceClipIndex>\n\
                 \t</ClipChannelSerializer>"
            ));
            ch_ids.push(cid);
        }
        let vec_id = self.alloc_id();
        let mut ch_items = String::new();
        for (i, cid) in ch_ids.iter().enumerate() {
            ch_items.push_str(&format!(
                "\t\t\t<ClipChannelItem Index=\"{i}\" ObjectRef=\"{cid}\" />\n"
            ));
        }
        self.push(format!(
            "\t<ClipChannelVectorSerializer ObjectID=\"{vec_id}\" ClassID=\"333d203b-3a53-4195-8894-fc7523ff3dc7\" Version=\"1\">\n\
             \t\t<ClipChannels Version=\"1\">\n\
             {ch_items}\
             \t\t</ClipChannels>\n\
             \t\t<ChannelType>{ch_type}</ChannelType>\n\
             \t</ClipChannelVectorSerializer>"
        ));
        let grp_id = self.alloc_id();
        self.push(format!(
            "\t<ClipChannelGroupVectorSerializer ObjectID=\"{grp_id}\" ClassID=\"a3127a8c-95d4-456e-a7f5-171b3f922426\" Version=\"1\">\n\
             \t\t<ClipChannelVectors Version=\"1\">\n\
             \t\t\t<ClipChannelVectorItem Index=\"0\" ObjectRef=\"{vec_id}\" />\n\
             \t\t</ClipChannelVectors>\n\
             \t</ClipChannelGroupVectorSerializer>"
        ));
        grp_id
    }

    fn emit_sequence(&mut self, seq: &Sequence, report: &mut ConversionReport) {
        let seq_handle = self.seq_handles[&seq.id].clone();
        let fps = if seq.frame_rate.num > 0 && seq.frame_rate.den > 0 {
            seq.frame_rate
        } else {
            Rational::FPS_25
        };
        let frame_ticks = fps.frames_to_adobe_ticks(1);
        let dur_frames = seq.computed_duration_frames().max(1);
        let dur_ticks = fps.frames_to_adobe_ticks(dur_frames);
        let zero_point_ticks = fps.frames_to_adobe_ticks(seq.start_tc_frame.max(0));
        let w = seq.width.max(16);
        let h = seq.height.max(16);
        let td = video_time_display_format(fps);
        let escaped_name = xml_escape(&seq.name);

        // Collect links by group_id across all video + audio track items in this sequence
        let mut link_groups: BTreeMap<String, Vec<u64>> = BTreeMap::new();

        // Video tracks (ensure at least 1 track so Premiere opens cleanly)
        let default_vtracks;
        let vtracks: &[Track] = if seq.video_tracks.is_empty() {
            default_vtracks = vec![empty_track(1, TrackKind::Video, "V1")];
            &default_vtracks
        } else {
            &seq.video_tracks
        };

        let mut vtrack_uids = Vec::with_capacity(vtracks.len());
        for (idx, track) in vtracks.iter().enumerate() {
            let uid = self.emit_video_track(
                track,
                idx,
                seq,
                fps,
                w,
                h,
                &mut link_groups,
                report,
            );
            vtrack_uids.push(uid);
        }

        // Audio tracks (ensure at least 1 track so AudioMixTrack has an active source)
        let default_atracks;
        let atracks: &[Track] = if seq.audio_tracks.is_empty() {
            default_atracks = vec![empty_track(1, TrackKind::Audio, "A1")];
            &default_atracks
        } else {
            &seq.audio_tracks
        };

        // Plan Master + SubMixTrack IDs before emitting AudioClipTracks so each track knows its <Assign> target.
        let main_bus = seq
            .audio_buses
            .iter()
            .find(|b| b.is_main || b.parent_bus_id.is_none());
        let main_bus_id = main_bus.map(|b| b.id.as_str());
        let submix_buses: Vec<&nle_core::AudioBus> = seq
            .audio_buses
            .iter()
            .filter(|b| Some(b.id.as_str()) != main_bus_id)
            .collect();

        // Assign unique numeric Track.IDs (Master = 1, ClipTracks = 2..=atracks.len()+1, SubMixes = atracks.len()+2..)
        let mut submix_track_num_by_bus_id: HashMap<&str, usize> = HashMap::new();
        let mut submix_obj_id_by_bus_id: HashMap<&str, u64> = HashMap::new();
        for (s_idx, bus) in submix_buses.iter().enumerate() {
            let track_num = atracks.len() + 2 + s_idx;
            let obj_id = self.alloc_id();
            submix_track_num_by_bus_id.insert(bus.id.as_str(), track_num);
            submix_obj_id_by_bus_id.insert(bus.id.as_str(), obj_id);
        }

        let mut atrack_uids = Vec::with_capacity(atracks.len());
        let mut master_sources: Vec<(bool, String)> = Vec::new(); // (is_uref, id_str)
        let mut submix_sources: HashMap<&str, Vec<(bool, String)>> = HashMap::new();
        for (idx, track) in atracks.iter().enumerate() {
            let assign_track_num = track
                .output_bus_id
                .as_deref()
                .and_then(|bid| submix_track_num_by_bus_id.get(bid).copied());
            let uid = self.emit_audio_track(
                track,
                idx,
                seq,
                fps,
                assign_track_num,
                &mut link_groups,
                report,
            );
            if let Some(bid) = track
                .output_bus_id
                .as_deref()
                .filter(|bid| submix_track_num_by_bus_id.contains_key(*bid))
            {
                submix_sources.entry(bid).or_default().push((true, uid.clone()));
            } else {
                master_sources.push((true, uid.clone()));
            }
            atrack_uids.push(uid);
        }

        // Wire submix-to-submix or submix-to-master routing into their parent Inlet Sources
        for bus in &submix_buses {
            let child_obj = submix_obj_id_by_bus_id[bus.id.as_str()].to_string();
            if let Some(pid) = bus
                .parent_bus_id
                .as_deref()
                .filter(|pid| *pid != bus.id.as_str() && submix_track_num_by_bus_id.contains_key(*pid))
            {
                submix_sources.entry(pid).or_default().push((false, child_obj));
            } else {
                master_sources.push((false, child_obj));
            }
        }

        // VideoTrackGroup
        let vtg_id = self.alloc_id();
        let vtg_chain_id = self.alloc_id();
        self.push(format!(
            "\t<VideoComponentChain ObjectID=\"{vtg_chain_id}\" ClassID=\"0970e08a-f58f-4108-b29a-1a717b8e12e2\" Version=\"3\">\n\
             \t\t<ComponentChain Version=\"3\" />\n\
             \t</VideoComponentChain>"
        ));
        let mut vtg_tracks_xml = String::new();
        for (i, uid) in vtrack_uids.iter().enumerate() {
            vtg_tracks_xml.push_str(&format!("\t\t\t\t<Track Index=\"{i}\" ObjectURef=\"{uid}\" />\n"));
        }
        self.push(format!(
            "\t<VideoTrackGroup ObjectID=\"{vtg_id}\" ClassID=\"9e9abf7a-0918-49c2-91ae-991b5dde77bb\" Version=\"13\">\n\
             \t\t<TrackGroup Version=\"1\">\n\
             \t\t\t<Tracks Version=\"1\">\n\
             {vtg_tracks_xml}\
             \t\t\t</Tracks>\n\
             \t\t\t<NextTrackID>{}</NextTrackID>\n\
             \t\t\t<FrameRate>{frame_ticks}</FrameRate>\n\
             \t\t</TrackGroup>\n\
             \t\t<ColorManagementSettings>{COLOR_MGMT_SEQ}</ColorManagementSettings>\n\
             \t\t<AutoInputGamutCompressionEnabled>true</AutoInputGamutCompressionEnabled>\n\
             \t\t<ToneMappingDesaturation>0.5</ToneMappingDesaturation>\n\
             \t\t<IsGraphicsWhiteSameAsProject>false</IsGraphicsWhiteSameAsProject>\n\
             \t\t<IsColorAwareEffectsEnabledSameAsProject>false</IsColorAwareEffectsEnabledSameAsProject>\n\
             \t\t<FrameRect>0,0,{w},{h}</FrameRect>\n\
             \t\t<OutputColorSpace>{COLOR_SPACE_709_8BIT}</OutputColorSpace>\n\
             \t\t<ComponentOwner Version=\"1\">\n\
             \t\t\t<Components ObjectRef=\"{vtg_chain_id}\" />\n\
             \t\t</ComponentOwner>\n\
             \t</VideoTrackGroup>",
            vtrack_uids.len() + 1
        ));

        // AudioTrackGroup + AudioMixTrack (Master + optional SubMixTracks)
        let main_bus_name = main_bus
            .map(|b| b.name.as_str())
            .filter(|n| !n.is_empty() && !n.eq_ignore_ascii_case("master") && !n.eq_ignore_ascii_case("bus 1"));
        let master_obj_id = self.alloc_id();
        self.emit_audio_mix_track(
            master_obj_id,
            true,
            1,
            0,
            main_bus_name,
            None,
            &master_sources,
        );

        let mut submix_tracks_xml = String::new();
        if !submix_buses.is_empty() {
            submix_tracks_xml.push_str("\t\t<SubMixTracks Version=\"1\">\n");
            for (s_idx, bus) in submix_buses.iter().enumerate() {
                let obj_id = submix_obj_id_by_bus_id[bus.id.as_str()];
                let track_num = submix_track_num_by_bus_id[bus.id.as_str()];
                let assign_parent = bus
                    .parent_bus_id
                    .as_deref()
                    .filter(|pid| *pid != bus.id.as_str())
                    .and_then(|pid| submix_track_num_by_bus_id.get(pid).copied());
                let sources = submix_sources
                    .get(bus.id.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                self.emit_audio_mix_track(
                    obj_id,
                    false,
                    track_num,
                    s_idx,
                    Some(bus.name.as_str()),
                    assign_parent,
                    sources,
                );
                submix_tracks_xml.push_str(&format!(
                    "\t\t\t<SubMixTrack Index=\"{s_idx}\" ObjectRef=\"{obj_id}\" />\n"
                ));
            }
            submix_tracks_xml.push_str("\t\t</SubMixTracks>\n");
        }

        let atg_id = self.alloc_id();
        let mut atg_tracks_xml = String::new();
        for (i, uid) in atrack_uids.iter().enumerate() {
            atg_tracks_xml.push_str(&format!("\t\t\t\t<Track Index=\"{i}\" ObjectURef=\"{uid}\" />\n"));
        }
        self.push(format!(
            "\t<AudioTrackGroup ObjectID=\"{atg_id}\" ClassID=\"9b9238b9-53a8-4cc3-b03f-b36246d052e6\" Version=\"6\">\n\
             \t\t<TrackGroup Version=\"1\">\n\
             \t\t\t<Tracks Version=\"1\">\n\
             {atg_tracks_xml}\
             \t\t\t</Tracks>\n\
             \t\t\t<NextTrackID>{}</NextTrackID>\n\
             \t\t\t<FrameRate>{TICKS_PER_48KHZ_SAMPLE}</FrameRate>\n\
             \t\t</TrackGroup>\n\
             \t\t<MasterTrack ObjectRef=\"{master_obj_id}\" />\n\
             {submix_tracks_xml}\
             \t\t<ID>{}</ID>\n\
             \t\t<AutomationSafeFlags>0</AutomationSafeFlags>\n\
             \t\t<NumAdaptiveChannels>2</NumAdaptiveChannels>\n\
             \t</AudioTrackGroup>",
            atrack_uids.len() + 2 + submix_buses.len(),
            Uuid::new_v4()
        ));

        // DataTrackGroup
        let dtg_id = self.alloc_id();
        self.push(format!(
            "\t<DataTrackGroup ObjectID=\"{dtg_id}\" ClassID=\"b714b71d-6838-48dd-9b77-db19088ced7e\" Version=\"1\">\n\
             \t\t<TrackGroup Version=\"1\">\n\
             \t\t\t<NextTrackID>1</NextTrackID>\n\
             \t\t\t<FrameRate>{frame_ticks}</FrameRate>\n\
             \t\t</TrackGroup>\n\
             \t</DataTrackGroup>"
        ));

        // Sequence markers
        let markers_to_write: &[Marker] = if self.options.include_markers {
            &seq.markers
        } else {
            &[]
        };
        report.markers_written += markers_to_write.len();
        let seq_markers_id = self.emit_markers(markers_to_write, fps);

        // Links
        let mut link_ids = Vec::new();
        for item_ids in link_groups.values() {
            if item_ids.len() < 2 {
                continue;
            }
            let lid = self.alloc_id();
            let mut items_xml = String::new();
            for (i, tid) in item_ids.iter().enumerate() {
                items_xml.push_str(&format!("\t\t\t\t<TrackItem Index=\"{i}\" ObjectRef=\"{tid}\" />\n"));
            }
            self.push(format!(
                "\t<Link ObjectID=\"{lid}\" ClassID=\"149d4ea5-a7d4-4b34-9bb7-16d783904bf2\" Version=\"1\">\n\
                 \t\t<TrackItemGroup Version=\"1\">\n\
                 \t\t\t<TrackItems Version=\"1\">\n\
                 {items_xml}\
                 \t\t\t</TrackItems>\n\
                 \t\t</TrackItemGroup>\n\
                 \t</Link>"
            ));
            link_ids.push(lid);
        }

        let mut links_section = String::new();
        if !link_ids.is_empty() {
            links_section.push_str("\t\t<PersistentGroupContainer Version=\"1\">\n\t\t\t<LinkContainer Version=\"1\">\n\t\t\t\t<Links Version=\"1\">\n");
            for (i, lid) in link_ids.iter().enumerate() {
                links_section.push_str(&format!("\t\t\t\t\t<Link Index=\"{i}\" ObjectRef=\"{lid}\" />\n"));
            }
            links_section.push_str("\t\t\t\t</Links>\n\t\t\t</LinkContainer>\n\t\t</PersistentGroupContainer>\n");
        }

        let mut extra_props = String::new();
        if let Some(mi) = seq.mark_in {
            extra_props.push_str(&format!(
                "\t\t\t\t<MZ.InPoint>{}</MZ.InPoint>\n",
                fps.frames_to_adobe_ticks(mi)
            ));
        }
        if let Some(mo) = seq.mark_out {
            extra_props.push_str(&format!(
                "\t\t\t\t<MZ.OutPoint>{}</MZ.OutPoint>\n",
                fps.frames_to_adobe_ticks(mo)
            ));
        }

        let work_out = dur_ticks.max(fps.frames_to_adobe_ticks(250));
        self.push(format!(
            "\t<Sequence ObjectUID=\"{}\" ClassID=\"6a15d903-8739-11d5-af2d-9b7855ad8974\" Version=\"12\">\n\
             \t\t<Node Version=\"1\">\n\
             \t\t\t<Properties Version=\"1\">\n\
             \t\t\t\t<AMM.CurrentSolo>[]</AMM.CurrentSolo>\n\
             \t\t\t\t<MZ.EditLine>0</MZ.EditLine>\n\
             \t\t\t\t<MZ.Sequence.AudioTimeDisplayFormat>200</MZ.Sequence.AudioTimeDisplayFormat>\n\
             \t\t\t\t<MZ.Sequence.VideoTimeDisplayFormat>{td}</MZ.Sequence.VideoTimeDisplayFormat>\n\
             \t\t\t\t<MZ.WorkInPoint>0</MZ.WorkInPoint>\n\
             \t\t\t\t<MZ.WorkOutPoint>{work_out}</MZ.WorkOutPoint>\n\
             \t\t\t\t<Monitor.ProgramZoomIn>0</Monitor.ProgramZoomIn>\n\
             \t\t\t\t<Monitor.ProgramZoomOut>{dur_ticks}</Monitor.ProgramZoomOut>\n\
             \t\t\t\t<TL.SQAVDividerPosition>0.5</TL.SQAVDividerPosition>\n\
             \t\t\t\t<TL.SQHeaderWidth>180</TL.SQHeaderWidth>\n\
             \t\t\t\t<TL.SQTimePerPixel>0.015503875968992248</TL.SQTimePerPixel>\n\
             \t\t\t\t<MZ.Sequence.EditingModeGUID>{EDITING_MODE_GUID}</MZ.Sequence.EditingModeGUID>\n\
             \t\t\t\t<MZ.Sequence.PreviewUseMaxBitDepth>false</MZ.Sequence.PreviewUseMaxBitDepth>\n\
             \t\t\t\t<MZ.Sequence.PreviewUseMaxRenderQuality>false</MZ.Sequence.PreviewUseMaxRenderQuality>\n\
             \t\t\t\t<MZ.Sequence.PreviewFrameSizeWidth>{w}</MZ.Sequence.PreviewFrameSizeWidth>\n\
             \t\t\t\t<MZ.Sequence.PreviewFrameSizeHeight>{h}</MZ.Sequence.PreviewFrameSizeHeight>\n\
             \t\t\t\t<MZ.ZeroPoint>{zero_point_ticks}</MZ.ZeroPoint>\n\
             {extra_props}\
             \t\t\t</Properties>\n\
             \t\t</Node>\n\
             \t\t<MarkerOwner Version=\"1\">\n\
             \t\t\t<Markers ObjectRef=\"{seq_markers_id}\" />\n\
             \t\t</MarkerOwner>\n\
             {links_section}\
             \t\t<TrackGroups Version=\"1\">\n\
             \t\t\t<TrackGroup Version=\"1\" Index=\"0\">\n\
             \t\t\t\t<First>{MEDIA_TYPE_VIDEO}</First>\n\
             \t\t\t\t<Second ObjectRef=\"{vtg_id}\" />\n\
             \t\t\t</TrackGroup>\n\
             \t\t\t<TrackGroup Version=\"1\" Index=\"1\">\n\
             \t\t\t\t<First>{MEDIA_TYPE_AUDIO}</First>\n\
             \t\t\t\t<Second ObjectRef=\"{atg_id}\" />\n\
             \t\t\t</TrackGroup>\n\
             \t\t\t<TrackGroup Version=\"1\" Index=\"2\">\n\
             \t\t\t\t<First>{MEDIA_TYPE_DATA}</First>\n\
             \t\t\t\t<Second ObjectRef=\"{dtg_id}\" />\n\
             \t\t\t</TrackGroup>\n\
             \t\t</TrackGroups>\n\
             \t\t<Name>{escaped_name}</Name>\n\
             \t</Sequence>",
            seq_handle.seq_uid
        ));

        // Emit Sequence's VideoSequenceSource, AudioSequenceSource, MasterClip, and ClipProjectItem
        self.push(format!(
            "\t<VideoSequenceSource ObjectID=\"{}\" ClassID=\"4752dfa9-7a7e-4a3b-a25b-cafde1a8d036\" Version=\"3\">\n\
             \t\t<SequenceSource Version=\"4\">\n\
             \t\t\t<Content Version=\"10\" />\n\
             \t\t\t<Sequence ObjectURef=\"{}\" />\n\
             \t\t</SequenceSource>\n\
             \t\t<OriginalDuration>{dur_ticks}</OriginalDuration>\n\
             \t</VideoSequenceSource>",
            seq_handle.video_source_id, seq_handle.seq_uid
        ));
        self.push(format!(
            "\t<AudioSequenceSource ObjectID=\"{}\" ClassID=\"e8d4cc83-38cb-491f-9d94-e5f7e3b205ee\" Version=\"7\">\n\
             \t\t<SequenceSource Version=\"4\">\n\
             \t\t\t<Content Version=\"10\" />\n\
             \t\t\t<Sequence ObjectURef=\"{}\" />\n\
             \t\t</SequenceSource>\n\
             \t\t<OriginalDuration>{dur_ticks}</OriginalDuration>\n\
             \t</AudioSequenceSource>",
            seq_handle.audio_source_id, seq_handle.seq_uid
        ));

        let seq_log_id = self.alloc_id();
        self.push(format!(
            "\t<ClipLoggingInfo ObjectID=\"{seq_log_id}\" ClassID=\"77ab7fdd-dcdf-465d-9906-7a330ca1e738\" Version=\"10\" />"
        ));
        let seq_ac_chain_id = self.alloc_id();
        self.push(format!(
            "\t<AudioComponentChain ObjectID=\"{seq_ac_chain_id}\" ClassID=\"3cb131d1-d3c0-47ae-a19a-bdf75ea11674\" Version=\"4\">\n\
             \t\t<DefaultVol>true</DefaultVol>\n\
             \t\t<DefaultVolumeComponentID>1</DefaultVolumeComponentID>\n\
             \t\t<DefaultChannelVolumeComponentID>2</DefaultChannelVolumeComponentID>\n\
             \t\t<ComponentChain Version=\"3\" />\n\
             \t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t\t<ChannelType>1</ChannelType>\n\
             \t</AudioComponentChain>"
        ));
        let seq_audio_clip_id = self.alloc_id();
        let seq_sec_ids = self.emit_secondary_contents(seq_handle.audio_source_id, 2, 0);
        self.push(format!(
            "\t<AudioClip ObjectID=\"{seq_audio_clip_id}\" ClassID=\"b8830d03-de02-41ee-84ec-fe566dc70cd9\" Version=\"8\">\n\
             \t\t<Clip Version=\"18\">\n\
             \t\t\t<Node Version=\"1\"><Properties Version=\"1\">\n\
             \t\t\t\t<asl.clip.label.color>19005</asl.clip.label.color>\n\
             \t\t\t\t<asl.clip.label.name>BE.Prefs.LabelColors.5</asl.clip.label.name>\n\
             \t\t\t</Properties></Node>\n\
             \t\t\t<Source ObjectRef=\"{}\" />\n\
             \t\t\t<ClipID>{}</ClipID>\n\
             \t\t\t<InUse>false</InUse>\n\
             \t\t</Clip>\n\
             \t\t<SecondaryContents Version=\"1\">\n\
             \t\t\t<SecondaryContentItem Index=\"0\" ObjectRef=\"{}\" />\n\
             \t\t\t<SecondaryContentItem Index=\"1\" ObjectRef=\"{}\" />\n\
             \t\t</SecondaryContents>\n\
             \t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t</AudioClip>",
            seq_handle.audio_source_id,
            Uuid::new_v4(),
            seq_sec_ids[0],
            seq_sec_ids[1]
        ));
        let seq_video_clip_id = self.alloc_id();
        self.push(format!(
            "\t<VideoClip ObjectID=\"{seq_video_clip_id}\" ClassID=\"9308dbef-2440-4acb-9ab2-953b9a4e82ec\" Version=\"11\">\n\
             \t\t<Clip Version=\"18\">\n\
             \t\t\t<Node Version=\"1\"><Properties Version=\"1\">\n\
             \t\t\t\t<asl.clip.label.color>19005</asl.clip.label.color>\n\
             \t\t\t\t<asl.clip.label.name>BE.Prefs.LabelColors.5</asl.clip.label.name>\n\
             \t\t\t</Properties></Node>\n\
             \t\t\t<Source ObjectRef=\"{}\" />\n\
             \t\t\t<ClipID>{}</ClipID>\n\
             \t\t\t<InUse>false</InUse>\n\
             \t\t</Clip>\n\
             \t</VideoClip>",
            seq_handle.video_source_id,
            Uuid::new_v4()
        ));
        let seq_cg_id = self.emit_channel_groups(true);
        self.push(format!(
            "\t<MasterClip ObjectUID=\"{}\" ClassID=\"fb11c33a-b0a9-4465-aa94-b6d5db2628cf\" Version=\"12\">\n\
             \t\t<LoggingInfo ObjectRef=\"{seq_log_id}\" />\n\
             \t\t<AudioComponentChains Version=\"1\">\n\
             \t\t\t<AudioComponentChain Index=\"0\" ObjectRef=\"{seq_ac_chain_id}\" />\n\
             \t\t</AudioComponentChains>\n\
             \t\t<Clips Version=\"1\">\n\
             \t\t\t<Clip Index=\"0\" ObjectRef=\"{seq_audio_clip_id}\" />\n\
             \t\t\t<Clip Index=\"1\" ObjectRef=\"{seq_video_clip_id}\" />\n\
             \t\t</Clips>\n\
             \t\t<AudioClipChannelGroups ObjectRef=\"{seq_cg_id}\" />\n\
             \t\t<Name>{escaped_name}</Name>\n\
             \t\t<MasterClipChangeVersion>1</MasterClipChangeVersion>\n\
             \t</MasterClip>",
            seq_handle.master_clip_uid
        ));
        self.push(format!(
            "\t<ClipProjectItem ObjectUID=\"{}\" ClassID=\"cb4e0ed7-aca1-4171-8525-e3658dec06dd\" Version=\"1\">\n\
             \t\t<ProjectItem Version=\"1\">\n\
             \t\t\t<Node Version=\"1\"><Properties Version=\"1\">\n\
             \t\t\t\t<Column.PropertyText.Label>BE.Prefs.LabelColors.5</Column.PropertyText.Label>\n\
             \t\t\t</Properties></Node>\n\
             \t\t\t<Name>{escaped_name}</Name>\n\
             \t\t</ProjectItem>\n\
             \t\t<MasterClip ObjectURef=\"{}\" />\n\
             \t</ClipProjectItem>",
            seq_handle.clip_project_item_uid, seq_handle.master_clip_uid
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_video_track(
        &mut self,
        track: &Track,
        track_idx: usize,
        seq: &Sequence,
        fps: Rational,
        seq_w: u32,
        seq_h: u32,
        link_groups: &mut BTreeMap<String, Vec<u64>>,
        report: &mut ConversionReport,
    ) -> String {
        let track_uid = Uuid::new_v4().to_string();

        // Pre-allocate transition IDs and match HeadTransition / TailTransition to clips
        let (trans_ids, head_by_clip, tail_by_clip) =
            self.emit_track_transitions(track, TrackKind::Video, fps);

        let mut item_ids = Vec::with_capacity(track.clips.len());
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if let Some(ti_id) = self.emit_video_clip_item(
                clip,
                seq,
                fps,
                seq_w,
                seq_h,
                head_by_clip.get(&clip_idx).copied(),
                tail_by_clip.get(&clip_idx).copied(),
                report,
            ) {
                if let Some(ref gid) = clip.link_group_id {
                    link_groups.entry(gid.clone()).or_default().push(ti_id);
                }
                item_ids.push(ti_id);
                report.video_clips_converted += 1;
            }
        }

        let mut clips_xml = String::new();
        for (i, id) in item_ids.iter().enumerate() {
            clips_xml.push_str(&format!("\t\t\t\t\t<TrackItem Index=\"{i}\" ObjectRef=\"{id}\" />\n"));
        }
        let mut trans_xml = String::new();
        for (i, id) in trans_ids.iter().enumerate() {
            trans_xml.push_str(&format!("\t\t\t\t\t<TrackItem Index=\"{i}\" ObjectRef=\"{id}\" />\n"));
        }

        let mut props_xml = format!(
            "\t\t\t\t\t\t<TL.SQTrackExpanded>0</TL.SQTrackExpanded>\n\
             \t\t\t\t\t\t<TL.SQTrackExpandedHeight>41</TL.SQTrackExpandedHeight>\n\
             \t\t\t\t\t\t<TL.SQTrackShy>0</TL.SQTrackShy>\n"
        );
        if track_idx == 0 {
            props_xml.push_str("\t\t\t\t\t\t<MZ.TrackTargeted>1</MZ.TrackTargeted>\n");
        }
        if !track.name.is_empty() && track.name != format!("V{}", track_idx + 1) {
            props_xml.push_str(&format!(
                "\t\t\t\t\t\t<MZ.TrackName>{}</MZ.TrackName>\n",
                xml_escape(&track.name)
            ));
        }

        let muted_el = if track.muted || !track.enabled {
            "\t\t\t\t<IsMuted>true</IsMuted>\n"
        } else {
            ""
        };
        let locked_el = if track.locked {
            "\t\t\t\t<IsLocked>true</IsLocked>\n"
        } else {
            ""
        };

        self.push(format!(
            "\t<VideoClipTrack ObjectUID=\"{track_uid}\" ClassID=\"f68dcd81-8805-11d5-af2d-9bfa89d4ddd4\" Version=\"1\">\n\
             \t\t<ClipTrack Version=\"2\">\n\
             \t\t\t<Track Version=\"4\">\n\
             \t\t\t\t<Node Version=\"1\">\n\
             \t\t\t\t\t<Properties Version=\"1\">\n\
             {props_xml}\
             \t\t\t\t\t</Properties>\n\
             \t\t\t\t</Node>\n\
             {muted_el}\
             {locked_el}\
             \t\t\t\t<ID>{}</ID>\n\
             \t\t\t\t<MediaType>{MEDIA_TYPE_VIDEO}</MediaType>\n\
             \t\t\t\t<Index>{track_idx}</Index>\n\
             \t\t\t</Track>\n\
             \t\t\t<ClipItems Version=\"3\">\n\
             \t\t\t\t<TrackItems Version=\"1\">\n\
             {clips_xml}\
             \t\t\t\t</TrackItems>\n\
             \t\t\t\t<MediaType>{MEDIA_TYPE_VIDEO}</MediaType>\n\
             \t\t\t\t<Index>{track_idx}</Index>\n\
             \t\t\t</ClipItems>\n\
             \t\t\t<TransitionItems Version=\"3\">\n\
             \t\t\t\t<TrackItems Version=\"1\">\n\
             {trans_xml}\
             \t\t\t\t</TrackItems>\n\
             \t\t\t\t<MediaType>{MEDIA_TYPE_VIDEO}</MediaType>\n\
             \t\t\t\t<Index>{track_idx}</Index>\n\
             \t\t\t</TransitionItems>\n\
             \t\t</ClipTrack>\n\
             \t</VideoClipTrack>",
            track_idx + 1
        ));

        track_uid
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_audio_track(
        &mut self,
        track: &Track,
        track_idx: usize,
        seq: &Sequence,
        fps: Rational,
        assign_track_num: Option<usize>,
        link_groups: &mut BTreeMap<String, Vec<u64>>,
        report: &mut ConversionReport,
    ) -> String {
        let track_uid = Uuid::new_v4().to_string();

        let (trans_ids, head_by_clip, tail_by_clip) =
            self.emit_track_transitions(track, TrackKind::Audio, fps);

        let mut item_ids = Vec::with_capacity(track.clips.len());
        for (clip_idx, clip) in track.clips.iter().enumerate() {
            if let Some(ti_id) = self.emit_audio_clip_item(
                clip,
                seq,
                fps,
                head_by_clip.get(&clip_idx).copied(),
                tail_by_clip.get(&clip_idx).copied(),
                report,
            ) {
                if let Some(ref gid) = clip.link_group_id {
                    link_groups.entry(gid.clone()).or_default().push(ti_id);
                }
                item_ids.push(ti_id);
                report.audio_clips_converted += 1;
            }
        }

        let (track_chain_id, panner_id) = self.emit_audio_track_mixer_components(false);

        let mut clips_xml = String::new();
        for (i, id) in item_ids.iter().enumerate() {
            clips_xml.push_str(&format!("\t\t\t\t\t<TrackItem Index=\"{i}\" ObjectRef=\"{id}\" />\n"));
        }
        let mut trans_xml = String::new();
        for (i, id) in trans_ids.iter().enumerate() {
            trans_xml.push_str(&format!("\t\t\t\t\t<TrackItem Index=\"{i}\" ObjectRef=\"{id}\" />\n"));
        }

        let mut props_xml = format!(
            "\t\t\t\t\t\t<TL.SQTrackExpanded>0</TL.SQTrackExpanded>\n\
             \t\t\t\t\t\t<TL.SQTrackExpandedHeight>41</TL.SQTrackExpandedHeight>\n\
             \t\t\t\t\t\t<TL.SQTrackAudioKeyframeStyle>0</TL.SQTrackAudioKeyframeStyle>\n\
             \t\t\t\t\t\t<TL.SQTrackShy>0</TL.SQTrackShy>\n\
             \t\t\t\t\t\t<CM.KeyframeMode>true</CM.KeyframeMode>\n"
        );
        if track_idx == 0 {
            props_xml.push_str("\t\t\t\t\t\t<MZ.TrackTargeted>1</MZ.TrackTargeted>\n");
        }
        if !track.name.is_empty() && track.name != format!("A{}", track_idx + 1) {
            props_xml.push_str(&format!(
                "\t\t\t\t\t\t<MZ.TrackName>{}</MZ.TrackName>\n",
                xml_escape(&track.name)
            ));
        }

        let muted_el = if track.muted || !track.enabled {
            "\t\t\t\t<IsMuted>true</IsMuted>\n"
        } else {
            ""
        };
        let locked_el = if track.locked {
            "\t\t\t\t<IsLocked>true</IsLocked>\n"
        } else {
            ""
        };
        let solo_el = if track.solo {
            "\t\t\t\t<Solo>1</Solo>\n"
        } else {
            ""
        };

        let assign_el = if let Some(target_id) = assign_track_num {
            format!("\t\t\t<Assign>{target_id}</Assign>\n")
        } else {
            String::new()
        };
        let channel_type_el = if track.audio_channels == 1 {
            "\t\t\t<ChannelType>0</ChannelType>\n"
        } else {
            ""
        };

        self.push(format!(
            "\t<AudioClipTrack ObjectUID=\"{track_uid}\" ClassID=\"097f6203-99ae-11d5-84f2-8cf14bde7040\" Version=\"7\">\n\
             \t\t<ClipTrack Version=\"2\">\n\
             \t\t\t<Track Version=\"4\">\n\
             \t\t\t\t<Node Version=\"1\">\n\
             \t\t\t\t\t<Properties Version=\"1\">\n\
             {props_xml}\
             \t\t\t\t\t</Properties>\n\
             \t\t\t\t</Node>\n\
             {muted_el}\
             {locked_el}\
             {solo_el}\
             \t\t\t\t<ID>{}</ID>\n\
             \t\t\t\t<MediaType>{MEDIA_TYPE_AUDIO}</MediaType>\n\
             \t\t\t\t<Index>{track_idx}</Index>\n\
             \t\t\t</Track>\n\
             \t\t\t<ClipItems Version=\"3\">\n\
             \t\t\t\t<TrackItems Version=\"1\">\n\
             {clips_xml}\
             \t\t\t\t</TrackItems>\n\
             \t\t\t\t<MediaType>{MEDIA_TYPE_AUDIO}</MediaType>\n\
             \t\t\t\t<Index>{track_idx}</Index>\n\
             \t\t\t</ClipItems>\n\
             \t\t\t<TransitionItems Version=\"3\">\n\
             \t\t\t\t<TrackItems Version=\"1\">\n\
             {trans_xml}\
             \t\t\t\t</TrackItems>\n\
             \t\t\t\t<MediaType>{MEDIA_TYPE_AUDIO}</MediaType>\n\
             \t\t\t\t<Index>{track_idx}</Index>\n\
             \t\t\t</TransitionItems>\n\
             \t\t</ClipTrack>\n\
             \t\t<AudioTrack Version=\"12\">\n\
             \t\t\t<ComponentOwner Version=\"1\">\n\
             \t\t\t\t<Components ObjectRef=\"{track_chain_id}\" />\n\
             \t\t\t</ComponentOwner>\n\
             \t\t\t<Panner ObjectRef=\"{panner_id}\" />\n\
             \t\t\t<ID>{}</ID>\n\
             {assign_el}\
             \t\t\t<NextPannerID>4294967279</NextPannerID>\n\
             {channel_type_el}\
             \t\t</AudioTrack>\n\
             \t</AudioClipTrack>",
            track_idx + 2,
            Uuid::new_v4()
        ));

        track_uid
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_audio_mix_track(
        &mut self,
        mix_id: u64,
        is_master: bool,
        track_num_id: usize,
        track_index: usize,
        name: Option<&str>,
        assign_parent: Option<usize>,
        sources: &[(bool, String)],
    ) {
        let (chain_id, panner_id) = self.emit_audio_track_mixer_components(is_master);
        let inlet_id = self.alloc_id();
        let mut sources_xml = String::new();
        for (i, (is_uref, id)) in sources.iter().enumerate() {
            if *is_uref {
                sources_xml.push_str(&format!("\t\t\t<Source Index=\"{i}\" ObjectURef=\"{id}\" />\n"));
            } else {
                sources_xml.push_str(&format!("\t\t\t<Source Index=\"{i}\" ObjectRef=\"{id}\" />\n"));
            }
        }
        self.push(format!(
            "\t<AudioTrackInlet ObjectID=\"{inlet_id}\" ClassID=\"be3af080-e8c6-11d5-abc4-a1c6d5dee670\" Version=\"4\">\n\
             \t\t<Sources Version=\"1\">\n\
             {sources_xml}\
             \t\t</Sources>\n\
             \t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t</AudioTrackInlet>"
        ));
        let sub_type = if is_master { 3 } else { 2 };
        let assign_el = if is_master {
            "\t\t\t<Assign>0</Assign>\n".to_string()
        } else if let Some(parent_num) = assign_parent {
            format!("\t\t\t<Assign>{parent_num}</Assign>\n")
        } else {
            String::new()
        };
        let name_prop = name
            .filter(|n| !n.trim().is_empty())
            .map(|n| format!("\t\t\t\t\t<MZ.TrackName>{}</MZ.TrackName>\n", xml_escape(n)))
            .unwrap_or_default();
        self.push(format!(
            "\t<AudioMixTrack ObjectID=\"{mix_id}\" ClassID=\"4b1d8400-e89e-11d5-abc4-a1a13b1e80a0\" Version=\"4\">\n\
             \t\t<AudioTrack Version=\"12\">\n\
             \t\t\t<ComponentOwner Version=\"1\">\n\
             \t\t\t\t<Components ObjectRef=\"{chain_id}\" />\n\
             \t\t\t</ComponentOwner>\n\
             \t\t\t<Panner ObjectRef=\"{panner_id}\" />\n\
             \t\t\t<ID>{}</ID>\n\
             \t\t\t<SubType>{sub_type}</SubType>\n\
             {assign_el}\
             \t\t\t<NextPannerID>4294967279</NextPannerID>\n\
             \t\t</AudioTrack>\n\
             \t\t<Track Version=\"4\">\n\
             \t\t\t<Node Version=\"1\">\n\
             \t\t\t\t<Properties Version=\"1\">\n\
             \t\t\t\t\t<TL.SQTrackExpanded>0</TL.SQTrackExpanded>\n\
             \t\t\t\t\t<TL.SQTrackExpandedHeight>41</TL.SQTrackExpandedHeight>\n\
             {name_prop}\
             \t\t\t\t</Properties>\n\
             \t\t\t</Node>\n\
             \t\t\t<ID>{track_num_id}</ID>\n\
             \t\t\t<MediaType>{MEDIA_TYPE_AUDIO}</MediaType>\n\
             \t\t\t<Index>{track_index}</Index>\n\
             \t\t</Track>\n\
             \t\t<Inlet ObjectRef=\"{inlet_id}\" />\n\
             \t</AudioMixTrack>",
            Uuid::new_v4()
        ));
    }

    fn emit_audio_track_mixer_components(&mut self, is_master: bool) -> (u64, u64) {
        let vol_param = self.alloc_id();
        self.push(format!(
            "\t<AudioComponentParam ObjectID=\"{vol_param}\" ClassID=\"a714635e-a628-4b27-9d59-77eba47dbc1a\" Version=\"10\">\n\
             \t\t<Name>Volume</Name>\n\
             \t\t<RangeLocked>false</RangeLocked>\n\
             \t\t<UnitsString>dB</UnitsString>\n\
             \t\t<UpperBound>5.6234130859375</UpperBound>\n\
             \t</AudioComponentParam>"
        ));
        let mute_param = self.alloc_id();
        self.push(format!(
            "\t<AudioComponentParam ObjectID=\"{mute_param}\" ClassID=\"32657501-3aa4-445f-a49b-d09ecb9fa1ae\" Version=\"10\">\n\
             \t\t<Name>Mute</Name>\n\
             \t\t<RangeLocked>false</RangeLocked>\n\
             \t</AudioComponentParam>"
        ));
        let fader_id = self.alloc_id();
        self.push(format!(
            "\t<AudioFader ObjectID=\"{fader_id}\" ClassID=\"1a38c583-ed5c-11d5-abc4-c1cbf61ec590\" Version=\"3\">\n\
             \t\t<AudioComponent Version=\"3\">\n\
             \t\t\t<Component Version=\"7\">\n\
             \t\t\t\t<Params Version=\"1\">\n\
             \t\t\t\t\t<Param Index=\"0\" ObjectRef=\"{vol_param}\" />\n\
             \t\t\t\t\t<Param Index=\"1\" ObjectRef=\"{mute_param}\" />\n\
             \t\t\t\t</Params>\n\
             \t\t\t\t<ID>1</ID>\n\
             \t\t\t</Component>\n\
             \t\t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t\t\t<AudioComponentType>1</AudioComponentType>\n\
             \t\t\t<FrameRate>{TICKS_PER_48KHZ_SAMPLE}</FrameRate>\n\
             \t\t\t<ChannelType>1</ChannelType>\n\
             \t\t</AudioComponent>\n\
             \t</AudioFader>"
        ));
        let meter_id = self.alloc_id();
        self.push(format!(
            "\t<AudioMeter ObjectID=\"{meter_id}\" ClassID=\"72ea4700-f615-11d5-abc4-c186585e63e0\" Version=\"2\">\n\
             \t\t<AudioComponent Version=\"3\">\n\
             \t\t\t<Component Version=\"7\"><ID>2</ID></Component>\n\
             \t\t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t\t\t<AudioComponentType>2</AudioComponentType>\n\
             \t\t\t<FrameRate>{TICKS_PER_48KHZ_SAMPLE}</FrameRate>\n\
             \t\t\t<ChannelType>1</ChannelType>\n\
             \t\t</AudioComponent>\n\
             \t</AudioMeter>"
        ));
        let chain_id = self.alloc_id();
        self.push(format!(
            "\t<AudioComponentChain ObjectID=\"{chain_id}\" ClassID=\"3cb131d1-d3c0-47ae-a19a-bdf75ea11674\" Version=\"4\">\n\
             \t\t<ComponentChain Version=\"3\">\n\
             \t\t\t<Components Version=\"1\">\n\
             \t\t\t\t<Component Index=\"0\" ObjectRef=\"{fader_id}\" />\n\
             \t\t\t\t<Component Index=\"1\" ObjectRef=\"{meter_id}\" />\n\
             \t\t\t</Components>\n\
             \t\t</ComponentChain>\n\
             \t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t\t<ChannelType>1</ChannelType>\n\
             \t</AudioComponentChain>"
        ));

        let panner_id = self.alloc_id();
        if is_master {
            self.push(format!(
                "\t<DefaultPanProcessor ObjectID=\"{panner_id}\" ClassID=\"33a94282-ee2c-11d5-abc4-c1cd7f9e3c10\" Version=\"2\">\n\
                 \t\t<PanProcessor Version=\"3\">\n\
                 \t\t\t<AudioComponent Version=\"3\">\n\
                 \t\t\t\t<Component Version=\"7\"><ID>4294967280</ID></Component>\n\
                 \t\t\t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
                 \t\t\t\t<AudioComponentType>0</AudioComponentType>\n\
                 \t\t\t\t<FrameRate>{TICKS_PER_48KHZ_SAMPLE}</FrameRate>\n\
                 \t\t\t\t<ChannelType>1</ChannelType>\n\
                 \t\t\t</AudioComponent>\n\
                 \t\t</PanProcessor>\n\
                 \t\t<DefaultPannerOutputChannelType>1</DefaultPannerOutputChannelType>\n\
                 \t\t<DefaultPannerInputChannelType>1</DefaultPannerInputChannelType>\n\
                 \t</DefaultPanProcessor>"
            ));
        } else {
            let bal_param = self.alloc_id();
            self.push(format!(
                "\t<AudioComponentParam ObjectID=\"{bal_param}\" ClassID=\"a714635e-a628-4b27-9d59-77eba47dbc1a\" Version=\"10\">\n\
                 \t\t<StartKeyframe>{START_KF_SENTINEL},0.5,0,0,0,0,0,0</StartKeyframe>\n\
                 \t\t<CurrentValue>0.5</CurrentValue>\n\
                 \t\t<Name>Balance</Name>\n\
                 \t\t<IsInverted>true</IsInverted>\n\
                 \t</AudioComponentParam>"
            ));
            self.push(format!(
                "\t<StereoToStereoPanProcessor ObjectID=\"{panner_id}\" ClassID=\"7bf86a01-efbe-11d5-abc4-c1ce2b1e9090\" Version=\"1\">\n\
                 \t\t<PanProcessor Version=\"3\">\n\
                 \t\t\t<AudioComponent Version=\"3\">\n\
                 \t\t\t\t<Component Version=\"7\">\n\
                 \t\t\t\t\t<Params Version=\"1\">\n\
                 \t\t\t\t\t\t<Param Index=\"0\" ObjectRef=\"{bal_param}\" />\n\
                 \t\t\t\t\t</Params>\n\
                 \t\t\t\t\t<ID>4294967280</ID>\n\
                 \t\t\t\t</Component>\n\
                 \t\t\t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
                 \t\t\t\t<AudioComponentType>0</AudioComponentType>\n\
                 \t\t\t\t<FrameRate>{TICKS_PER_48KHZ_SAMPLE}</FrameRate>\n\
                 \t\t\t\t<ChannelType>1</ChannelType>\n\
                 \t\t\t</AudioComponent>\n\
                 \t\t</PanProcessor>\n\
                 \t</StereoToStereoPanProcessor>"
            ));
        }

        (chain_id, panner_id)
    }

    fn emit_track_transitions(
        &mut self,
        track: &Track,
        kind: TrackKind,
        fps: Rational,
    ) -> (Vec<u64>, HashMap<usize, u64>, HashMap<usize, u64>) {
        let mut trans_ids = Vec::new();
        let mut head_by_clip: HashMap<usize, u64> = HashMap::new();
        let mut tail_by_clip: HashMap<usize, u64> = HashMap::new();

        for tr in &track.transitions {
            if tr.end_frame <= tr.start_frame {
                continue;
            }
            let start_ticks = fps.frames_to_adobe_ticks(tr.start_frame.max(0));
            let end_ticks = fps.frames_to_adobe_ticks(tr.end_frame);
            let dur_ticks = (end_ticks - start_ticks).max(1);

            // Find outgoing / incoming clips touching this transition span.
            let mut out_idx: Option<usize> = None;
            let mut in_idx: Option<usize> = None;
            for (ci, clip) in track.clips.iter().enumerate() {
                if clip.end_frame >= tr.start_frame && clip.end_frame <= tr.end_frame + 1 {
                    if tr.alignment != "start" {
                        out_idx = Some(ci);
                    }
                }
                if clip.start_frame >= tr.start_frame - 1 && clip.start_frame <= tr.end_frame {
                    if tr.alignment != "end" {
                        in_idx = Some(ci);
                    }
                }
            }

            let (has_out, has_in, alignment_ticks) = match (out_idx.is_some(), in_idx.is_some()) {
                (false, true) => (false, true, 0),
                (true, false) => (true, false, dur_ticks),
                (true, true) => {
                    let align = match tr.alignment.as_str() {
                        "start" => 0,
                        "end" => dur_ticks,
                        _ => dur_ticks / 2,
                    };
                    (true, true, align)
                }
                (false, false) => {
                    let ho = matches!(tr.alignment.as_str(), "center" | "end");
                    let hi = matches!(tr.alignment.as_str(), "center" | "start");
                    let align = match tr.alignment.as_str() {
                        "start" => 0,
                        "end" => dur_ticks,
                        _ => dur_ticks / 2,
                    };
                    (ho, hi, align)
                }
            };

            let ti_id = match kind {
                TrackKind::Video => self.emit_video_transition(
                    tr,
                    start_ticks,
                    end_ticks,
                    alignment_ticks,
                    has_out,
                    has_in,
                ),
                TrackKind::Audio => self.emit_audio_transition(
                    tr,
                    start_ticks,
                    end_ticks,
                    alignment_ticks,
                    has_out,
                    has_in,
                ),
            };

            if let Some(ci) = out_idx {
                tail_by_clip.entry(ci).or_insert(ti_id);
            }
            if let Some(ci) = in_idx {
                head_by_clip.entry(ci).or_insert(ti_id);
            }
            trans_ids.push(ti_id);
        }

        (trans_ids, head_by_clip, tail_by_clip)
    }

    fn emit_video_transition(
        &mut self,
        tr: &Transition,
        start_ticks: i64,
        end_ticks: i64,
        alignment_ticks: i64,
        has_out: bool,
        has_in: bool,
    ) -> u64 {
        let (display_name, match_name) = map_video_transition(&tr.name, &tr.effect_id);
        let ti_id = self.alloc_id();
        let start_el = if start_ticks > 0 {
            format!("\t\t\t\t<Start>{start_ticks}</Start>\n")
        } else {
            String::new()
        };
        self.push(format!(
            "\t<VideoTransitionTrackItem ObjectID=\"{ti_id}\" ClassID=\"3eeaed31-f78e-4144-b8e8-077656517181\" Version=\"6\">\n\
             \t\t<TransitionTrackItem Version=\"3\">\n\
             \t\t\t<TrackItem Version=\"4\">\n\
             {start_el}\
             \t\t\t\t<End>{end_ticks}</End>\n\
             \t\t\t</TrackItem>\n\
             \t\t\t<HasOutgoingClip>{has_out}</HasOutgoingClip>\n\
             \t\t\t<HasIncomingClip>{has_in}</HasIncomingClip>\n\
             \t\t\t<DisplayName>{}</DisplayName>\n\
             \t\t\t<MatchName>{}</MatchName>\n\
             \t\t\t<Alignment>{alignment_ticks}</Alignment>\n\
             \t\t</TransitionTrackItem>\n\
             \t</VideoTransitionTrackItem>",
            xml_escape(display_name),
            xml_escape(match_name)
        ));
        ti_id
    }

    fn emit_audio_transition(
        &mut self,
        tr: &Transition,
        start_ticks: i64,
        end_ticks: i64,
        alignment_ticks: i64,
        has_out: bool,
        has_in: bool,
    ) -> u64 {
        let (display_name, match_name) = map_audio_transition(&tr.name, &tr.effect_id);
        let ti_id = self.alloc_id();
        let start_el = if start_ticks > 0 {
            format!("\t\t\t\t<Start>{start_ticks}</Start>\n")
        } else {
            String::new()
        };
        self.push(format!(
            "\t<AudioTransitionTrackItem ObjectID=\"{ti_id}\" ClassID=\"23f687b6-9c5a-42f4-bea1-e7b3e28e7082\" Version=\"4\">\n\
             \t\t<TransitionTrackItem Version=\"3\">\n\
             \t\t\t<TrackItem Version=\"4\">\n\
             {start_el}\
             \t\t\t\t<End>{end_ticks}</End>\n\
             \t\t\t</TrackItem>\n\
             \t\t\t<HasOutgoingClip>{has_out}</HasOutgoingClip>\n\
             \t\t\t<HasIncomingClip>{has_in}</HasIncomingClip>\n\
             \t\t\t<DisplayName>{}</DisplayName>\n\
             \t\t\t<MatchName>{}</MatchName>\n\
             \t\t\t<Alignment>{alignment_ticks}</Alignment>\n\
             \t\t</TransitionTrackItem>\n\
             \t\t<AudioChannelLayout>{LAYOUT_STEREO}</AudioChannelLayout>\n\
             \t</AudioTransitionTrackItem>",
            xml_escape(display_name),
            xml_escape(match_name)
        ));
        ti_id
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_video_clip_item(
        &mut self,
        clip: &Clip,
        seq: &Sequence,
        fps: Rational,
        seq_w: u32,
        seq_h: u32,
        head_tr: Option<u64>,
        tail_tr: Option<u64>,
        report: &mut ConversionReport,
    ) -> Option<u64> {
        let (source_id, master_clip_uid, base_markers_id, asset_dur_ticks, mc_selected_idx) =
            self.resolve_clip_source(clip, TrackKind::Video)?;

        let start_ticks = fps.frames_to_adobe_ticks(clip.start_frame.max(0));
        let end_ticks = fps.frames_to_adobe_ticks(clip.end_frame.max(clip.start_frame + 1));
        let (in_ticks, out_ticks, speed_xml, reverse_xml) =
            self.compute_clip_source_timing(clip, fps, end_ticks - start_ticks, asset_dur_ticks);

        // Clip markers (if present and enabled)
        let markers_id = if self.options.include_markers && !clip.markers.is_empty() {
            report.markers_written += clip.markers.len();
            self.emit_markers(&clip.markers, fps)
        } else {
            base_markers_id
        };

        let label_xml = self.clip_label_xml(clip, seq_is_container(clip.source_kind));
        let multicam_xml = if clip.source_kind == ClipSourceKind::MulticamClip {
            let idx = mc_selected_idx.unwrap_or(0);
            format!("\t\t\t<SelectedTrackIndex>{idx}</SelectedTrackIndex>\n\t\t\t<IsMulticam>true</IsMulticam>\n")
        } else {
            String::new()
        };

        let time_interp_xml = match clip.retime_interpolation {
            RetimeInterpolation::FrameSampling => "\t\t<TimeInterpolationType>0</TimeInterpolationType>\n",
            RetimeInterpolation::FrameBlending => "\t\t<TimeInterpolationType>1</TimeInterpolationType>\n",
            RetimeInterpolation::OpticalFlow => "\t\t<TimeInterpolationType>2</TimeInterpolationType>\n",
            RetimeInterpolation::Default => "",
        };
        if clip.speed_keyframes.len() > 2 {
            report.messages.push(ConversionMessage {
                severity: Severity::Warning,
                code: "SPEED_RAMP".into(),
                sequence: None,
                clip: Some(clip.name.clone()),
                message: format!(
                    "Variable speed ramp ({} keyframes) approximated with average constant speed ({:.1}%) in Premiere Pro; review Time Remapping curve.",
                    clip.speed_keyframes.len(),
                    clip.speed_percent
                ),
            });
        }

        let vc_id = self.alloc_id();
        self.push(format!(
            "\t<VideoClip ObjectID=\"{vc_id}\" ClassID=\"9308dbef-2440-4acb-9ab2-953b9a4e82ec\" Version=\"11\">\n\
             \t\t<Clip Version=\"18\">\n\
             {label_xml}\
             \t\t\t<MarkerOwner Version=\"1\"><Markers ObjectRef=\"{markers_id}\" /></MarkerOwner>\n\
             \t\t\t<Source ObjectRef=\"{source_id}\" />\n\
             \t\t\t<ClipID>{}</ClipID>\n\
             {speed_xml}\
             {reverse_xml}\
             {multicam_xml}\
             \t\t\t<InPoint>{in_ticks}</InPoint>\n\
             \t\t\t<OutPoint>{out_ticks}</OutPoint>\n\
             \t\t</Clip>\n\
             {time_interp_xml}\
             \t</VideoClip>",
            Uuid::new_v4()
        ));

        let subclip_id = self.alloc_id();
        self.push(format!(
            "\t<SubClip ObjectID=\"{subclip_id}\" ClassID=\"e0c58dc9-dbdd-4166-aef7-5db7e3f22e84\" Version=\"6\">\n\
             \t\t<Clip ObjectRef=\"{vc_id}\" />\n\
             \t\t<MasterClip ObjectURef=\"{master_clip_uid}\" />\n\
             \t\t<Name>{}</Name>\n\
             \t\t<OrigChGrp>0</OrigChGrp>\n\
             \t</SubClip>",
            xml_escape(&clip.name)
        ));

        let chain_id = self.emit_video_component_chain(clip, report);

        let ti_id = self.alloc_id();
        let start_el = if start_ticks > 0 {
            format!("\t\t\t\t<Start>{start_ticks}</Start>\n")
        } else {
            String::new()
        };
        let deact_el = if !clip.enabled {
            "\t\t\t\t<Deactivated>true</Deactivated>\n"
        } else {
            ""
        };
        let head_el = head_tr
            .map(|id| format!("\t\t\t<HeadTransition ObjectRef=\"{id}\" />\n"))
            .unwrap_or_default();
        let tail_el = tail_tr
            .map(|id| format!("\t\t\t<TailTransition ObjectRef=\"{id}\" />\n"))
            .unwrap_or_default();
        let cw = if clip.source_width > 0 { clip.source_width } else { seq_w };
        let ch = if clip.source_height > 0 { clip.source_height } else { seq_h };
        let par_str = self
            .media_by_id
            .get(&clip.media_id)
            .or_else(|| self.media_by_path.get(&clip.media_path))
            .and_then(|h| h.pixel_aspect_ratio)
            .filter(|par| par.is_finite() && *par > 0.0 && (*par - 1.0).abs() > 1e-4)
            .map(|par| format!("{},10000", (par * 10_000.0).round() as i64))
            .unwrap_or_else(|| "1,1".to_string());

        self.push(format!(
            "\t<VideoClipTrackItem ObjectID=\"{ti_id}\" ClassID=\"368b0406-29e3-4923-9fcd-094fbf9a1089\" Version=\"8\">\n\
             \t\t<ClipTrackItem Version=\"8\">\n\
             \t\t\t<ComponentOwner Version=\"1\">\n\
             \t\t\t\t<Components ObjectRef=\"{chain_id}\" />\n\
             \t\t\t</ComponentOwner>\n\
             \t\t\t<TrackItem Version=\"4\">\n\
             {start_el}\
             \t\t\t\t<End>{end_ticks}</End>\n\
             {deact_el}\
             \t\t\t</TrackItem>\n\
             \t\t\t<SubClip ObjectRef=\"{subclip_id}\" />\n\
             {head_el}\
             {tail_el}\
             \t\t</ClipTrackItem>\n\
             \t\t<ToneMapSettings>{{\"peak\":-1,\"version\":3}}</ToneMapSettings>\n\
             \t\t<FrameRect>0,0,{cw},{ch}</FrameRect>\n\
             \t\t<PixelAspectRatio>{par_str}</PixelAspectRatio>\n\
             \t</VideoClipTrackItem>"
        ));

        let _ = seq;
        Some(ti_id)
    }

    fn emit_audio_clip_item(
        &mut self,
        clip: &Clip,
        _seq: &Sequence,
        fps: Rational,
        head_tr: Option<u64>,
        tail_tr: Option<u64>,
        report: &mut ConversionReport,
    ) -> Option<u64> {
        let (source_id, master_clip_uid, base_markers_id, asset_dur_ticks, mc_selected_idx) =
            self.resolve_clip_source(clip, TrackKind::Audio)?;

        let start_ticks = fps.frames_to_adobe_ticks(clip.start_frame.max(0));
        let end_ticks = fps.frames_to_adobe_ticks(clip.end_frame.max(clip.start_frame + 1));
        let (in_ticks, out_ticks, speed_xml, reverse_xml) =
            self.compute_clip_source_timing(clip, fps, end_ticks - start_ticks, asset_dur_ticks);

        let markers_id = if self.options.include_markers && !clip.markers.is_empty() {
            report.markers_written += clip.markers.len();
            self.emit_markers(&clip.markers, fps)
        } else {
            base_markers_id
        };

        let is_stereo = self
            .media_by_id
            .get(&clip.media_id)
            .or_else(|| self.media_by_path.get(&clip.media_path))
            .map(|h| h.is_stereo)
            .unwrap_or(true);
        let start_ch = clip.audio_channel.saturating_sub(1);
        let ch_count = if is_stereo && start_ch == 0 { 2 } else { 1 };
        let audio_layout = if ch_count >= 2 { LAYOUT_STEREO } else { LAYOUT_MONO };
        let sec_ids = self.emit_secondary_contents(source_id, ch_count, start_ch);
        let mut sec_xml = String::new();
        for (i, sid) in sec_ids.iter().enumerate() {
            sec_xml.push_str(&format!(
                "\t\t\t<SecondaryContentItem Index=\"{i}\" ObjectRef=\"{sid}\" />\n"
            ));
        }

        let label_xml = self.clip_label_xml(clip, seq_is_container(clip.source_kind));
        let multicam_xml = if clip.source_kind == ClipSourceKind::MulticamClip {
            let idx = mc_selected_idx.unwrap_or(0);
            format!("\t\t\t<SelectedTrackIndex>{idx}</SelectedTrackIndex>\n\t\t\t<IsMulticam>true</IsMulticam>\n")
        } else {
            String::new()
        };

        let ac_id = self.alloc_id();
        self.push(format!(
            "\t<AudioClip ObjectID=\"{ac_id}\" ClassID=\"b8830d03-de02-41ee-84ec-fe566dc70cd9\" Version=\"8\">\n\
             \t\t<Clip Version=\"18\">\n\
             {label_xml}\
             \t\t\t<MarkerOwner Version=\"1\"><Markers ObjectRef=\"{markers_id}\" /></MarkerOwner>\n\
             \t\t\t<Source ObjectRef=\"{source_id}\" />\n\
             \t\t\t<ClipID>{}</ClipID>\n\
             {speed_xml}\
             {reverse_xml}\
             {multicam_xml}\
             \t\t\t<InPoint>{in_ticks}</InPoint>\n\
             \t\t\t<OutPoint>{out_ticks}</OutPoint>\n\
             \t\t</Clip>\n\
             \t\t<SecondaryContents Version=\"1\">\n\
             {sec_xml}\
             \t\t</SecondaryContents>\n\
             \t\t<AudioChannelLayout>{audio_layout}</AudioChannelLayout>\n\
             \t</AudioClip>",
            Uuid::new_v4()
        ));

        let subclip_id = self.alloc_id();
        self.push(format!(
            "\t<SubClip ObjectID=\"{subclip_id}\" ClassID=\"e0c58dc9-dbdd-4166-aef7-5db7e3f22e84\" Version=\"6\">\n\
             \t\t<Clip ObjectRef=\"{ac_id}\" />\n\
             \t\t<MasterClip ObjectURef=\"{master_clip_uid}\" />\n\
             \t\t<Name>{}</Name>\n\
             \t\t<OrigChGrp>0</OrigChGrp>\n\
             \t</SubClip>",
            xml_escape(&clip.name)
        ));

        let chain_id = self.alloc_id();
        self.push(format!(
            "\t<AudioComponentChain ObjectID=\"{chain_id}\" ClassID=\"3cb131d1-d3c0-47ae-a19a-bdf75ea11674\" Version=\"4\">\n\
             \t\t<DefaultVol>true</DefaultVol>\n\
             \t\t<DefaultVolumeComponentID>1</DefaultVolumeComponentID>\n\
             \t\t<ComponentChain Version=\"3\">\n\
             \t\t\t<Node Version=\"1\">\n\
             \t\t\t\t<Properties Version=\"1\">\n\
             \t\t\t\t\t<MZ.ComponentChain.ActiveComponentID>1</MZ.ComponentChain.ActiveComponentID>\n\
             \t\t\t\t\t<MZ.ComponentChain.ActiveComponentParamIndex>4294967295</MZ.ComponentChain.ActiveComponentParamIndex>\n\
             \t\t\t\t</Properties>\n\
             \t\t\t</Node>\n\
             \t\t</ComponentChain>\n\
             \t</AudioComponentChain>"
        ));

        let ti_id = self.alloc_id();
        let start_el = if start_ticks > 0 {
            format!("\t\t\t\t<Start>{start_ticks}</Start>\n")
        } else {
            String::new()
        };
        let deact_el = if !clip.enabled {
            "\t\t\t\t<Deactivated>true</Deactivated>\n"
        } else {
            ""
        };
        let head_el = head_tr
            .map(|id| format!("\t\t\t<HeadTransition ObjectRef=\"{id}\" />\n"))
            .unwrap_or_default();
        let tail_el = tail_tr
            .map(|id| format!("\t\t\t<TailTransition ObjectRef=\"{id}\" />\n"))
            .unwrap_or_default();

        self.push(format!(
            "\t<AudioClipTrackItem ObjectID=\"{ti_id}\" ClassID=\"064ec682-9ba6-11d5-af2d-9ca32c7d6164\" Version=\"11\">\n\
             \t\t<ClipTrackItem Version=\"8\">\n\
             \t\t\t<ComponentOwner Version=\"1\">\n\
             \t\t\t\t<Components ObjectRef=\"{chain_id}\" />\n\
             \t\t\t</ComponentOwner>\n\
             \t\t\t<TrackItem Version=\"4\">\n\
             {start_el}\
             \t\t\t\t<End>{end_ticks}</End>\n\
             {deact_el}\
             \t\t\t</TrackItem>\n\
             \t\t\t<SubClip ObjectRef=\"{subclip_id}\" />\n\
             {head_el}\
             {tail_el}\
             \t\t</ClipTrackItem>\n\
             \t\t<ID>{}</ID>\n\
             \t\t<PreRenderComponentChainHashVersion>1</PreRenderComponentChainHashVersion>\n\
             \t</AudioClipTrackItem>",
            Uuid::new_v4()
        ));

        Some(ti_id)
    }

    /// Resolve `(source_id, master_clip_uid, base_markers_id, asset_dur_ticks, multicam_angle_idx)`.
    fn resolve_clip_source(
        &mut self,
        clip: &Clip,
        kind: TrackKind,
    ) -> Option<(u64, String, u64, i64, Option<u32>)> {
        if clip.source_kind != ClipSourceKind::Media {
            if let Some(sh) = self.seq_handles.get(&clip.media_id).cloned() {
                let src_id = if kind == TrackKind::Video {
                    sh.video_source_id
                } else {
                    sh.audio_source_id
                };
                let angle_idx = if clip.source_kind == ClipSourceKind::MulticamClip {
                    Some(resolve_multicam_angle_index(
                        clip.active_angle.as_deref(),
                        &sh.track_names,
                    ))
                } else {
                    None
                };
                let empty_markers = self.emit_markers(&[], Rational::FPS_25);
                return Some((src_id, sh.master_clip_uid, empty_markers, 0, angle_idx));
            }
        }

        let handle = self
            .media_by_id
            .get(&clip.media_id)
            .or_else(|| self.media_by_path.get(&clip.media_path))
            .cloned()?;
        let src_id = if kind == TrackKind::Video {
            handle.video_source_id.or(handle.audio_source_id)?
        } else {
            handle.audio_source_id.or(handle.video_source_id)?
        };
        Some((
            src_id,
            handle.master_clip_uid,
            handle.markers_id,
            handle.asset_duration_ticks,
            None,
        ))
    }

    fn compute_clip_source_timing(
        &self,
        clip: &Clip,
        seq_fps: Rational,
        timeline_dur_ticks: i64,
        asset_dur_ticks: i64,
    ) -> (i64, i64, String, String) {
        let is_reversed = clip.reversed || clip.speed_percent < 0.0;
        let abs_speed = clip.speed_percent.abs();
        let is_freeze = abs_speed < 0.01;
        let speed_factor = if is_freeze {
            0.005
        } else if abs_speed.is_finite() && abs_speed > 0.0 {
            abs_speed / 100.0
        } else {
            1.0
        };

        let speed_xml = if (speed_factor - 1.0).abs() > 1e-4 {
            format!("\t\t\t<PlaybackSpeed>{speed_factor:.6}</PlaybackSpeed>\n")
        } else {
            String::new()
        };
        let reverse_xml = if is_reversed {
            "\t\t\t<PlayBackwards>true</PlayBackwards>\n".to_string()
        } else {
            String::new()
        };

        if let (Some(in_t), Some(out_t)) = (clip.source_in_ticks, clip.source_out_ticks) {
            if out_t > in_t {
                return (in_t, out_t, speed_xml, reverse_xml);
            }
        }

        let source_span = ((timeline_dur_ticks as f64) * speed_factor).round().max(1.0) as i64;
        let raw_in_ticks = seq_fps.frames_to_adobe_ticks(clip.source_in_frame.max(0));

        // Resolve stores <In> in record-domain frames for retimed clips and from-end for reversed clips.
        // Freeze frames already decoded their exact source hold frame in `source_in_frame`.
        let (in_ticks, out_ticks) = if self.source_format == NleFormat::DaVinciResolve && !is_freeze
        {
            let scaled_in = ((raw_in_ticks as f64) * speed_factor).round() as i64;
            if is_reversed && asset_dur_ticks > 0 {
                let out_t = (asset_dur_ticks - scaled_in).max(source_span);
                let in_t = (out_t - source_span).max(0);
                (in_t, out_t)
            } else {
                (scaled_in.max(0), scaled_in.max(0) + source_span)
            }
        } else {
            (raw_in_ticks, raw_in_ticks + source_span)
        };

        (in_ticks, out_ticks, speed_xml, reverse_xml)
    }

    fn clip_label_xml(&self, clip: &Clip, is_container: bool) -> String {
        if !self.options.include_clip_colors {
            return String::new();
        }
        let preset_idx = clip.label_preset_index.or_else(|| {
            clip.label_name
                .as_deref()
                .and_then(|s| s.rsplit('.').next()?.parse::<u8>().ok())
        });
        let Some(idx) = preset_idx else {
            if is_container {
                return "\t\t\t<Node Version=\"1\"><Properties Version=\"1\">\n\
                        \t\t\t\t<asl.clip.label.color>19005</asl.clip.label.color>\n\
                        \t\t\t\t<asl.clip.label.name>BE.Prefs.LabelColors.5</asl.clip.label.name>\n\
                        \t\t\t</Properties></Node>\n"
                    .to_string();
            }
            return String::new();
        };
        let color_int = premiere_label_color_int(idx);
        format!(
            "\t\t\t<Node Version=\"1\"><Properties Version=\"1\">\n\
             \t\t\t\t<asl.clip.label.color>{color_int}</asl.clip.label.color>\n\
             \t\t\t\t<asl.clip.label.name>BE.Prefs.LabelColors.{idx}</asl.clip.label.name>\n\
             \t\t\t</Properties></Node>\n"
        )
    }

    fn emit_video_component_chain(&mut self, clip: &Clip, report: &mut ConversionReport) -> u64 {
        let mut comp_ids: Vec<u64> = Vec::new();
        let has_custom_opacity =
            (clip.opacity - 100.0).abs() > 1e-3 || !clip.blend_mode.is_normal();
        let has_custom_motion = !clip.motion.is_identity();

        if has_custom_motion {
            comp_ids.push(self.emit_motion_component(&clip.motion));
        }
        if has_custom_opacity {
            comp_ids.push(self.emit_opacity_component(clip.opacity, clip.blend_mode));
        }
        let mut next_comp_id = 3u32;
        if self.options.include_lumetri_grades {
            for lum in &clip.lumetri_effects {
                comp_ids.push(self.emit_lumetri_component(lum, next_comp_id));
                next_comp_id += 1;
                report.grades_attached += 1;
            }
        }
        let has_stabilization = clip.effects.iter().any(|e| {
            e.enabled
                && (e.category == nle_core::EffectCategory::Stabilization
                    || e.match_name
                        .to_ascii_lowercase()
                        .contains("subspacestabilizer")
                    || e.match_name.to_ascii_lowercase().contains("stabiliz"))
        });
        if has_stabilization {
            comp_ids.push(self.emit_warp_stabilizer_component(next_comp_id));
            next_comp_id += 1;
        }
        // Attach matched third-party Adobe/MediaCore video filter plugins (e.g. `AE.AGDL Multi-Layer Anonymizer`)
        for eff in &clip.effects {
            if !eff.enabled || eff.is_intrinsic || eff.media_kind != TrackKind::Video {
                continue;
            }
            if !eff.match_name.starts_with("AE.") {
                continue;
            }
            let lower = eff.match_name.to_ascii_lowercase();
            if lower.contains("adbe motion")
                || lower.contains("adbe opacity")
                || lower.contains("lumetri")
                || lower.contains("subspacestabilizer")
            {
                continue;
            }
            comp_ids.push(self.emit_generic_video_filter_component(
                &eff.display_name,
                &eff.match_name,
                next_comp_id,
            ));
            next_comp_id += 1;
        }

        let chain_id = self.alloc_id();
        let mut comp_xml = String::new();
        if !comp_ids.is_empty() {
            comp_xml.push_str("\t\t\t<Components Version=\"1\">\n");
            for (i, cid) in comp_ids.iter().enumerate() {
                comp_xml.push_str(&format!("\t\t\t\t<Component Index=\"{i}\" ObjectRef=\"{cid}\" />\n"));
            }
            comp_xml.push_str("\t\t\t</Components>\n");
        }
        let default_opacity = if has_custom_opacity { "false" } else { "true" };
        self.push(format!(
            "\t<VideoComponentChain ObjectID=\"{chain_id}\" ClassID=\"0970e08a-f58f-4108-b29a-1a717b8e12e2\" Version=\"3\">\n\
             \t\t<DefaultOpacity>{default_opacity}</DefaultOpacity>\n\
             \t\t<DefaultOpacityComponentID>2</DefaultOpacityComponentID>\n\
             \t\t<ComponentChain Version=\"3\">\n\
             {comp_xml}\
             \t\t</ComponentChain>\n\
             \t</VideoComponentChain>"
        ));
        chain_id
    }

    fn emit_motion_component(&mut self, m: &MotionTransform) -> u64 {
        let px = 0.5 + m.position_x;
        let py = 0.5 - m.position_y;
        let ax = 0.5 + m.anchor_x;
        let ay = 0.5 + m.anchor_y;
        let uniform = m
            .scale_y
            .map(|sy| (sy - m.scale).abs() < 1e-4)
            .unwrap_or(true);
        let scale_h = m.scale_y.unwrap_or(m.scale);
        let scale_w = m.scale;

        let p_pos = self.emit_point_param("Position", 1, px, py);
        let p_scale = self.emit_float_param("Scale", 2, scale_h, 0.0, 10000.0, Some(200.0), None);
        let p_scale_w =
            self.emit_float_param("Scale Width", 3, scale_w, 0.0, 10000.0, Some(200.0), None);
        let p_unif = self.emit_bool_param(" ", 4, uniform);
        let p_rot = self.emit_float_param("Rotation", 5, m.rotation, -32768.0, 32767.0, None, Some(3));
        let p_anchor = self.emit_point_param("Anchor Point", 6, ax, ay);
        let p_af = self.emit_double_param("Anti-flicker Filter", 7, 0.0, 0.0, 1.0);
        let p_cl = self.emit_float_param("Crop Left", 8, m.crop_left, 0.0, 100.0, None, None);
        let p_ct = self.emit_float_param("Crop Top", 9, m.crop_top, 0.0, 100.0, None, None);
        let p_cr = self.emit_float_param("Crop Right", 10, m.crop_right, 0.0, 100.0, None, None);
        let p_cb = self.emit_float_param("Crop Bottom", 11, m.crop_bottom, 0.0, 100.0, None, None);

        let params = [
            p_pos, p_scale, p_scale_w, p_unif, p_rot, p_anchor, p_af, p_cl, p_ct, p_cr, p_cb,
        ];
        let mut params_xml = String::new();
        for (i, pid) in params.iter().enumerate() {
            params_xml.push_str(&format!("\t\t\t\t<Param Index=\"{i}\" ObjectRef=\"{pid}\" />\n"));
        }

        let cid = self.alloc_id();
        self.push(format!(
            "\t<VideoFilterComponent ObjectID=\"{cid}\" ClassID=\"d10da199-beea-4dd1-b941-ed3a78766d50\" Version=\"9\">\n\
             \t\t<Component Version=\"7\">\n\
             \t\t\t<Params Version=\"1\">\n\
             {params_xml}\
             \t\t\t</Params>\n\
             \t\t\t<ID>1</ID>\n\
             \t\t\t<Intrinsic>true</Intrinsic>\n\
             \t\t\t<DisplayName>Motion</DisplayName>\n\
             \t\t</Component>\n\
             \t\t<PremiereFilterPrivateData Encoding=\"base64\">ARI=</PremiereFilterPrivateData>\n\
             \t\t<VideoFilterType>2</VideoFilterType>\n\
             \t\t<MatchName>AE.ADBE Motion</MatchName>\n\
             \t</VideoFilterComponent>"
        ));
        cid
    }

    fn emit_opacity_component(&mut self, opacity: f64, blend_mode: BlendMode) -> u64 {
        let clamped = opacity.clamp(0.0, 100.0);
        let (bm_param2, bm_param3) = blend_mode.to_premiere_params();
        let p_op = self.emit_float_param("Opacity", 1, clamped, 0.0, 100.0, None, None);
        let p_bm1 = self.emit_enum_param("Blend Mode", 2, bm_param2 as i32, 0, 27, Some(10));
        let p_bm2 = self.emit_enum_param("Blend Mode", 3, bm_param3 as i32, 0, 31, None);
        let cid = self.alloc_id();
        self.push(format!(
            "\t<VideoFilterComponent ObjectID=\"{cid}\" ClassID=\"d10da199-beea-4dd1-b941-ed3a78766d50\" Version=\"9\">\n\
             \t\t<Component Version=\"7\">\n\
             \t\t\t<Params Version=\"1\">\n\
             \t\t\t\t<Param Index=\"0\" ObjectRef=\"{p_op}\" />\n\
             \t\t\t\t<Param Index=\"1\" ObjectRef=\"{p_bm1}\" />\n\
             \t\t\t\t<Param Index=\"2\" ObjectRef=\"{p_bm2}\" />\n\
             \t\t\t</Params>\n\
             \t\t\t<ID>2</ID>\n\
             \t\t\t<Intrinsic>true</Intrinsic>\n\
             \t\t\t<DisplayName>Opacity</DisplayName>\n\
             \t\t</Component>\n\
             \t\t<VideoFilterType>2</VideoFilterType>\n\
             \t\t<MatchName>AE.ADBE Opacity</MatchName>\n\
             \t</VideoFilterComponent>"
        ));
        cid
    }

    fn emit_lumetri_component(&mut self, lum: &LumetriData, comp_id_num: u32) -> u64 {
        let mut params = Vec::new();
        // Premiere Lumetri has no standalone 4th Offset wheel; fold Resolve Primary Offset RGB
        // gently into Temperature, Tint, and Exposure so global warm/cool/luma shifts survive.
        let mut temp = lum.temperature.unwrap_or(0.0);
        let mut tint = lum.tint.unwrap_or(0.0);
        let mut exp = lum.exposure.unwrap_or(0.0);
        if let Some([or, og, ob]) = lum.offset_rgb {
            if [or, og, ob].iter().all(|v| v.is_finite()) {
                let oy = 0.2126 * or + 0.7152 * og + 0.0722 * ob;
                exp = (exp + oy * 1.2).clamp(-5.0, 5.0);
                temp = (temp + (or - ob) * 120.0).clamp(-100.0, 100.0);
                tint = (tint + ((or + ob) * 0.5 - og) * 120.0).clamp(-100.0, 100.0);
            }
        }
        if lum.temperature.is_some() || temp.abs() > 1e-6 {
            params.push(self.emit_float_param("Temperature", 7, temp, -100.0, 100.0, None, None));
        }
        if lum.tint.is_some() || tint.abs() > 1e-6 {
            params.push(self.emit_float_param("Tint", 8, tint, -100.0, 100.0, None, None));
        }
        if lum.exposure.is_some() || exp.abs() > 1e-6 {
            params.push(self.emit_float_param("Exposure", 11, exp, -5.0, 5.0, None, None));
        }
        if let Some(v) = lum.contrast {
            params.push(self.emit_float_param("Contrast", 12, v, -100.0, 100.0, None, None));
        }
        if let Some(v) = lum.highlights {
            params.push(self.emit_float_param("Highlights", 13, v, -100.0, 100.0, None, None));
        }
        if let Some(v) = lum.shadows {
            params.push(self.emit_float_param("Shadows", 14, v, -100.0, 100.0, None, None));
        }
        if let Some(v) = lum.whites {
            params.push(self.emit_float_param("Whites", 15, v, -100.0, 100.0, None, None));
        }
        if let Some(v) = lum.blacks {
            params.push(self.emit_float_param("Blacks", 16, v, -100.0, 100.0, None, None));
        }
        if let Some(v) = lum.saturation {
            params.push(self.emit_float_param("Saturation", 20, v, 0.0, 200.0, None, None));
        }
        if let Some(v) = lum.sharpen {
            params.push(self.emit_float_param("Sharpen", 29, v, -100.0, 100.0, None, None));
        }
        if let Some(v) = lum.vibrance {
            params.push(self.emit_float_param("Vibrance", 30, v, -100.0, 100.0, None, None));
        }
        let has_wheels = lum.shadows_wheel.is_some_and(|w| !w.is_neutral())
            || lum.midtones_wheel.is_some_and(|w| !w.is_neutral())
            || lum.highlights_wheel.is_some_and(|w| !w.is_neutral());
        if has_wheels {
            params.push(self.emit_bool_param(" ", 46, true));
            let wheels_b64 = crate::lumetri::encode_color_wheels_param47(
                lum.shadows_wheel,
                lum.midtones_wheel,
                lum.highlights_wheel,
            );
            params.push(self.emit_arb_param(" ", 47, 9, &wheels_b64));
        }

        let mut params_xml = String::new();
        for (i, pid) in params.iter().enumerate() {
            params_xml.push_str(&format!("\t\t\t\t<Param Index=\"{i}\" ObjectRef=\"{pid}\" />\n"));
        }
        let cid = self.alloc_id();
        self.push(format!(
            "\t<VideoFilterComponent ObjectID=\"{cid}\" ClassID=\"d10da199-beea-4dd1-b941-ed3a78766d50\" Version=\"9\">\n\
             \t\t<Component Version=\"7\">\n\
             \t\t\t<Params Version=\"1\">\n\
             {params_xml}\
             \t\t\t</Params>\n\
             \t\t\t<ID>{comp_id_num}</ID>\n\
             \t\t\t<DisplayName>{}</DisplayName>\n\
             \t\t</Component>\n\
             \t\t<VideoFilterType>2</VideoFilterType>\n\
             \t\t<MatchName>AE.ADBE Lumetri</MatchName>\n\
             \t</VideoFilterComponent>",
            xml_escape(&lum.display_name)
        ));
        cid
    }

    fn emit_warp_stabilizer_component(&mut self, comp_id_num: u32) -> u64 {
        self.emit_generic_video_filter_component(
            "Warp Stabilizer",
            "AE.ADBE SubspaceStabilizer",
            comp_id_num,
        )
    }

    fn emit_generic_video_filter_component(
        &mut self,
        display_name: &str,
        match_name: &str,
        comp_id_num: u32,
    ) -> u64 {
        let cid = self.alloc_id();
        self.push(format!(
            "\t<VideoFilterComponent ObjectID=\"{cid}\" ClassID=\"d10da199-beea-4dd1-b941-ed3a78766d50\" Version=\"9\">\n\
             \t\t<Component Version=\"7\">\n\
             \t\t\t<Params Version=\"1\" />\n\
             \t\t\t<ID>{comp_id_num}</ID>\n\
             \t\t\t<DisplayName>{}</DisplayName>\n\
             \t\t</Component>\n\
             \t\t<VideoFilterType>2</VideoFilterType>\n\
             \t\t<MatchName>{}</MatchName>\n\
             \t</VideoFilterComponent>",
            xml_escape(display_name),
            xml_escape(match_name)
        ));
        cid
    }

    fn emit_arb_param(&mut self, name: &str, param_id: u32, control_type: u32, b64: &str) -> u64 {
        let oid = self.alloc_id();
        self.push(format!(
            "\t<ArbVideoComponentParam ObjectID=\"{oid}\" ClassID=\"313e54d4-6903-49ad-b0bf-8262cdd10f4e\" Version=\"3\">\n\
             \t\t<Name>{}</Name>\n\
             \t\t<ParameterControlType>{control_type}</ParameterControlType>\n\
             \t\t<ParameterID>{param_id}</ParameterID>\n\
             \t\t<StartKeyframePosition>{START_KF_SENTINEL}</StartKeyframePosition>\n\
             \t\t<StartKeyframeValue Encoding=\"base64\">{b64}</StartKeyframeValue>\n\
             \t</ArbVideoComponentParam>",
            xml_escape(name)
        ));
        oid
    }

    fn emit_point_param(&mut self, name: &str, param_id: u32, x: f64, y: f64) -> u64 {
        let oid = self.alloc_id();
        self.push(format!(
            "\t<PointComponentParam ObjectID=\"{oid}\" ClassID=\"ca81d347-309b-44d2-acc7-1c572efb973c\" Version=\"4\">\n\
             \t\t<Name>{}</Name>\n\
             \t\t<ParameterID>{param_id}</ParameterID>\n\
             \t\t<StartKeyframe>{START_KF_SENTINEL},{x:.6}:{y:.6},0,0,0,0,0,0,5,4,0,0,0,0</StartKeyframe>\n\
             \t</PointComponentParam>",
            xml_escape(name)
        ));
        oid
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_float_param(
        &mut self,
        name: &str,
        param_id: u32,
        val: f64,
        lower: f64,
        upper: f64,
        upper_ui: Option<f64>,
        control_type: Option<u32>,
    ) -> u64 {
        let oid = self.alloc_id();
        let ct = control_type
            .map(|c| format!("\t\t<ParameterControlType>{c}</ParameterControlType>\n"))
            .unwrap_or_default();
        let uui = upper_ui
            .map(|u| format!("\t\t<UpperUIBound>{u}</UpperUIBound>\n"))
            .unwrap_or_default();
        self.push(format!(
            "\t<VideoComponentParam ObjectID=\"{oid}\" ClassID=\"fe47129e-6c94-4fc0-95d5-c056a517aaf3\" Version=\"10\">\n\
             \t\t<Name>{}</Name>\n\
             {ct}\
             \t\t<ParameterID>{param_id}</ParameterID>\n\
             {uui}\
             \t\t<StartKeyframe>{START_KF_SENTINEL},{val},0,0,0,0,0,0</StartKeyframe>\n\
             \t\t<LowerBound>{lower}</LowerBound>\n\
             \t\t<UpperBound>{upper}</UpperBound>\n\
             \t</VideoComponentParam>",
            xml_escape(name)
        ));
        oid
    }

    fn emit_double_param(
        &mut self,
        name: &str,
        param_id: u32,
        val: f64,
        lower: f64,
        upper: f64,
    ) -> u64 {
        let oid = self.alloc_id();
        self.push(format!(
            "\t<VideoComponentParam ObjectID=\"{oid}\" ClassID=\"a4ff2d6e-7ac2-44f8-9d52-17d9ca50e542\" Version=\"10\">\n\
             \t\t<Name>{}</Name>\n\
             \t\t<ParameterID>{param_id}</ParameterID>\n\
             \t\t<StartKeyframe>{START_KF_SENTINEL},{val},0,0,0,0,0,0</StartKeyframe>\n\
             \t\t<LowerBound>{lower}</LowerBound>\n\
             \t\t<UpperBound>{upper}</UpperBound>\n\
             \t</VideoComponentParam>",
            xml_escape(name)
        ));
        oid
    }

    fn emit_bool_param(&mut self, name: &str, param_id: u32, val: bool) -> u64 {
        let oid = self.alloc_id();
        self.push(format!(
            "\t<VideoComponentParam ObjectID=\"{oid}\" ClassID=\"cc12343e-f113-4d3b-ae05-b287db77d461\" Version=\"10\">\n\
             \t\t<Name>{}</Name>\n\
             \t\t<ParameterID>{param_id}</ParameterID>\n\
             \t\t<StartKeyframe>{START_KF_SENTINEL},{val},0,0,0,0,0,0</StartKeyframe>\n\
             \t</VideoComponentParam>",
            xml_escape(name)
        ));
        oid
    }

    fn emit_enum_param(
        &mut self,
        name: &str,
        param_id: u32,
        val: i32,
        lower: i32,
        upper: i32,
        control_type: Option<u32>,
    ) -> u64 {
        let oid = self.alloc_id();
        let ct = control_type
            .map(|c| format!("\t\t<ParameterControlType>{c}</ParameterControlType>\n"))
            .unwrap_or_default();
        self.push(format!(
            "\t<VideoComponentParam ObjectID=\"{oid}\" ClassID=\"6e02e8bb-2569-46b2-8ab1-4ab11c43e9c8\" Version=\"10\">\n\
             \t\t<Name>{}</Name>\n\
             \t\t<DiscontinuousInterpolate>true</DiscontinuousInterpolate>\n\
             {ct}\
             \t\t<ParameterID>{param_id}</ParameterID>\n\
             \t\t<StartKeyframe>{START_KF_SENTINEL},{val},0,0,0,0,0,0</StartKeyframe>\n\
             \t\t<LowerBound>{lower}</LowerBound>\n\
             \t\t<UpperBound>{upper}</UpperBound>\n\
             \t</VideoComponentParam>",
            xml_escape(name)
        ));
        oid
    }

    fn emit_markers(&mut self, markers: &[Marker], fps: Rational) -> u64 {
        let mut entries: Vec<(String, u64)> = Vec::with_capacity(markers.len());
        for m in markers {
            let marker_uid = Uuid::new_v4().to_string();
            let start_ticks = fps.frames_to_adobe_ticks(m.frame.max(0));
            let dur_ticks = if m.duration_frames > 1 {
                fps.frames_to_adobe_ticks(m.duration_frames)
            } else {
                0
            };
            let color_argb = marker_color_to_premiere_argb(&m.color);
            let mut inner = serde_json::Map::new();
            inner.insert("mComment".into(), serde_json::Value::String(m.comment.clone()));
            if dur_ticks > 0 {
                inner.insert(
                    "mDuration".into(),
                    serde_json::json!({ "ticks": dur_ticks }),
                );
            }
            if let Some(argb) = color_argb {
                inner.insert(
                    "mCuePointList".into(),
                    serde_json::json!([{
                        "mKey": "keywordExtDVAv1_7fc50e95-a1ef-424b-a023-702927446d80",
                        "mValue": format!("{{\"color\":{argb}}}")
                    }]),
                );
            }
            inner.insert("mMarkerID".into(), serde_json::Value::String(marker_uid.clone()));
            inner.insert("mName".into(), serde_json::Value::String(m.name.clone()));
            inner.insert(
                "mStartTime".into(),
                serde_json::json!({ "ticks": start_ticks }),
            );
            inner.insert("mType".into(), serde_json::Value::String("Comment".into()));
            let payload = serde_json::json!({ "DVAMarker": inner }).to_string();

            let mid = self.alloc_id();
            self.push(format!(
                "\t<Marker ObjectID=\"{mid}\" ClassID=\"a45508e0-3ff7-4d04-90a7-2e0dfff4c910\" Version=\"3\">\n\
                 \t\t<DVAMarker>{}</DVAMarker>\n\
                 \t</Marker>",
                xml_escape(&payload)
            ));
            entries.push((marker_uid, mid));
        }

        let container_id = self.alloc_id();
        let mut inner_xml = String::new();
        if !entries.is_empty() {
            inner_xml.push_str("\t\t<Markers Version=\"1\">\n");
            for (idx, (uid, mid)) in entries.iter().enumerate() {
                inner_xml.push_str(&format!(
                    "\t\t\t<Marker Version=\"1\" Index=\"{idx}\">\n\
                     \t\t\t\t<First>{uid}</First>\n\
                     \t\t\t\t<Second ObjectRef=\"{mid}\" />\n\
                     \t\t\t</Marker>\n"
                ));
            }
            inner_xml.push_str("\t\t</Markers>\n");
        }
        self.push(format!(
            "\t<Markers ObjectID=\"{container_id}\" ClassID=\"bee50706-b524-416c-9f03-b596ce5f6866\" Version=\"4\">\n\
             {inner_xml}\
             \t\t<ByGUID>byGUID</ByGUID>\n\
             \t\t<LastMetadataState>00000000-0000-0000-0000-000000000000</LastMetadataState>\n\
             \t\t<LastContentState>00000000-0000-0000-0000-000000000000</LastContentState>\n\
             \t</Markers>"
        ));
        container_id
    }

    fn emit_bins_and_root(
        &mut self,
        project: &Project,
        sequences: &[&Sequence],
        root_bin_uid: &str,
        report: &mut ConversionReport,
    ) {
        let mut placed_cpis: HashSet<String> = HashSet::new();
        let mut root_items: Vec<String> = Vec::new();

        if self.options.include_bins && !project.bins.is_empty() {
            let mut bin_uid_by_id: HashMap<String, String> = HashMap::new();
            for bin in &project.bins {
                bin_uid_by_id.insert(bin.id.clone(), normalize_uuid(&bin.id));
            }

            // Collect child bins for each bin
            let mut child_bins: HashMap<String, Vec<String>> = HashMap::new();
            for bin in &project.bins {
                let buid = bin_uid_by_id[&bin.id].clone();
                if let Some(pid) = bin.parent_id.as_ref().filter(|p| bin_uid_by_id.contains_key(*p))
                {
                    child_bins.entry(pid.clone()).or_default().push(buid);
                } else {
                    root_items.push(buid);
                }
            }

            for bin in &project.bins {
                let buid = bin_uid_by_id[&bin.id].clone();
                let mut items: Vec<String> = Vec::new();
                if let Some(sub) = child_bins.get(&bin.id) {
                    items.extend(sub.iter().cloned());
                }
                for path in &bin.media_paths {
                    if let Some(mh) = self.media_by_path.get(path) {
                        if placed_cpis.insert(mh.clip_project_item_uid.clone()) {
                            items.push(mh.clip_project_item_uid.clone());
                        }
                    }
                }
                for sid in &bin.sequence_ids {
                    if let Some(sh) = self.seq_handles.get(sid) {
                        if placed_cpis.insert(sh.clip_project_item_uid.clone()) {
                            items.push(sh.clip_project_item_uid.clone());
                        }
                    }
                }
                self.emit_bin_item(&buid, &bin.name, &items, &bin.custom_metadata);
                report.bins_created += 1;
            }
        }

        // Place any remaining unbinned sequences and media items directly in Root Bin
        for seq in sequences {
            if let Some(sh) = self.seq_handles.get(&seq.id) {
                if placed_cpis.insert(sh.clip_project_item_uid.clone()) {
                    root_items.push(sh.clip_project_item_uid.clone());
                }
            }
        }
        let mut media_cpis: Vec<String> = self
            .media_by_id
            .values()
            .chain(self.media_by_path.values())
            .map(|h| h.clip_project_item_uid.clone())
            .collect();
        media_cpis.sort();
        media_cpis.dedup();
        for cpi in media_cpis {
            if placed_cpis.insert(cpi.clone()) {
                root_items.push(cpi);
            }
        }

        let mut items_xml = String::new();
        for (i, uid) in root_items.iter().enumerate() {
            items_xml.push_str(&format!("\t\t\t\t<Item Index=\"{i}\" ObjectURef=\"{uid}\" />\n"));
        }
        self.push(format!(
            "\t<RootProjectItem ObjectUID=\"{root_bin_uid}\" ClassID=\"1c307a89-9318-47d7-a583-bf2553736543\" Version=\"1\">\n\
             \t\t<ProjectItem Version=\"1\">\n\
             \t\t\t<Node Version=\"1\">\n\
             \t\t\t\t<Properties Version=\"1\" />\n\
             \t\t\t\t<ID>1000000</ID>\n\
             \t\t\t</Node>\n\
             \t\t\t<Name>Root Bin</Name>\n\
             \t\t</ProjectItem>\n\
             \t\t<ProjectItemContainer Version=\"1\">\n\
             \t\t\t<Items Version=\"1\">\n\
             {items_xml}\
             \t\t\t</Items>\n\
             \t\t</ProjectItemContainer>\n\
             \t</RootProjectItem>"
        ));
    }

    fn emit_bin_item(
        &mut self,
        uid: &str,
        name: &str,
        item_uids: &[String],
        custom_metadata: &BTreeMap<String, String>,
    ) {
        let mut items_xml = String::new();
        for (i, child_uid) in item_uids.iter().enumerate() {
            items_xml.push_str(&format!(
                "\t\t\t\t<Item Index=\"{i}\" ObjectURef=\"{child_uid}\" />\n"
            ));
        }
        let mut extra_props = String::new();
        for (k, v) in custom_metadata {
            if !v.is_empty() && is_valid_xml_tag_name(k) {
                extra_props.push_str(&format!("\t\t\t\t\t<{k}>{}</{k}>\n", xml_escape(v)));
            }
        }
        self.push(format!(
            "\t<BinProjectItem ObjectUID=\"{uid}\" ClassID=\"dbfd6653-24da-480e-a35e-ba45e9504e4b\" Version=\"1\">\n\
             \t\t<ProjectItem Version=\"1\">\n\
             \t\t\t<Node Version=\"1\">\n\
             \t\t\t\t<Properties Version=\"1\">\n\
             \t\t\t\t\t<Column.PropertyText.Label>BE.Prefs.LabelColors.7</Column.PropertyText.Label>\n\
             {extra_props}\
             \t\t\t\t</Properties>\n\
             \t\t\t</Node>\n\
             \t\t\t<Name>{}</Name>\n\
             \t\t</ProjectItem>\n\
             \t\t<ProjectItemContainer Version=\"1\">\n\
             \t\t\t<Items Version=\"1\">\n\
             {items_xml}\
             \t\t\t</Items>\n\
             \t\t</ProjectItemContainer>\n\
             \t</BinProjectItem>",
            xml_escape(name)
        ));
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

fn empty_track(index: usize, kind: TrackKind, name: &str) -> Track {
    Track {
        index,
        kind,
        name: name.to_string(),
        enabled: true,
        locked: false,
        muted: false,
        solo: false,
        audio_channels: if kind == TrackKind::Audio { 2 } else { 0 },
        output_bus_id: None,
        clips: Vec::new(),
        transitions: Vec::new(),
        markers: Vec::new(),
    }
}

fn seq_is_container(kind: ClipSourceKind) -> bool {
    kind != ClipSourceKind::Media
}

fn normalize_uuid(raw: &str) -> String {
    Uuid::parse_str(raw.trim())
        .map(|u| u.to_string())
        .unwrap_or_else(|_| Uuid::new_v4().to_string())
}

fn video_time_display_format(fps: Rational) -> u32 {
    let tb = fps.timebase();
    let ntsc = fps.is_ntsc();
    match (tb, ntsc) {
        (24, _) => 100,
        (25, _) => 101,
        (30, true) => 102,
        (30, false) => 104,
        (50, _) => 108,
        (60, true) => 109,
        (60, false) => 110,
        _ => 101,
    }
}

fn resolve_multicam_angle_index(active_angle: Option<&str>, track_names: &[String]) -> u32 {
    let Some(angle) = active_angle.map(str::trim).filter(|s| !s.is_empty()) else {
        return 0;
    };
    if let Some(pos) = track_names
        .iter()
        .position(|t| t.eq_ignore_ascii_case(angle))
    {
        return pos as u32;
    }
    // Parse trailing 1-based number from e.g. "Camera 3", "Angle 2", "V2", or bare "3"
    if let Some(num) = angle
        .rsplit(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok())
    {
        return num.saturating_sub(1);
    }
    0
}

fn map_video_transition<'a>(name: &'a str, effect_id: &'a str) -> (&'a str, &'a str) {
    let lower = format!("{name} {effect_id}").to_ascii_lowercase();
    if lower.contains("dip to black")
        || lower.contains("dip to color")
        || lower.contains("fade to color")
    {
        ("Dip to Black", "AE.ADBE Dip to Black")
    } else if lower.contains("dip to white") {
        ("Dip to White", "AE.ADBE Dip to White")
    } else if lower.contains("non-additive") || lower.contains("non additive") {
        ("Non-Additive Dissolve", "AE.ADBE Non-Additive Dissolve")
    } else if lower.contains("additive") {
        ("Additive Dissolve", "AE.ADBE Additive Dissolve")
    } else if lower.contains("morph cut") || lower.contains("smooth cut") || lower.contains("flow") {
        ("Morph Cut", "AE.ADBE Morph Cut")
    } else if lower.contains("film dissolve") {
        ("Film Dissolve", "AE.ADBE Film Dissolve")
    } else if lower.contains("wipe") {
        ("Wipe", "AE.ADBE Wipe")
    } else {
        ("Cross Dissolve", "AE.ADBE Cross Dissolve")
    }
}

fn map_audio_transition<'a>(name: &'a str, effect_id: &'a str) -> (&'a str, &'a str) {
    let lower = format!("{name} {effect_id}").to_ascii_lowercase();
    if lower.contains("0db") || lower.contains("0 db") || lower.contains("constant gain") {
        ("Constant Gain", "Constant Gain")
    } else if lower.contains("-3db") || lower.contains("-3 db") || lower.contains("exponential") {
        ("Exponential Fade", "Exponential Fade")
    } else {
        ("Constant Power", "Constant Power")
    }
}

fn premiere_label_color_int(preset_idx: u8) -> u32 {
    match preset_idx {
        0 => 11106954,
        1 => 6769408,
        2 => 480554,
        3 => 4564681,
        4 => 14987615,
        5 => 19005,
        6 => 15451232,
        7 => 11977321,
        8 => 9532833,
        9 => 4614879,
        10 => 15111011,
        11 => 11044699,
        12 => 7522849,
        13 => 3119697,
        14 => 15250085,
        15 => 14539932,
        _ => 6769408,
    }
}

fn marker_color_to_premiere_argb(color: &str) -> Option<u32> {
    let val = match color.to_ascii_lowercase().as_str() {
        "green" => 4285640049,
        "red" => 4291314231,
        "purple" | "lavender" | "fuchsia" => 4289825711,
        "orange" | "sand" => 4293496637,
        "yellow" | "lemon" | "cream" => 4293774903,
        "white" => 4294967295,
        "blue" | "sky" => 4282552534,
        "cyan" | "mint" => 4283018442,
        "pink" | "rose" => 4293418162,
        _ => return None,
    };
    Some(val)
}

fn canonical_meta_to_premiere_column_tag(key: &str) -> Option<&'static str> {
    match key.trim().to_ascii_lowercase().as_str() {
        "description" => Some("Column.PropertyText.Description"),
        "comments" | "comment" => Some("Column.PropertyText.Comment"),
        "log note" => Some("Column.PropertyText.LogNote"),
        "scene" => Some("Column.PropertyText.Scene"),
        "shot" => Some("Column.PropertyText.Shot"),
        "take" => Some("Column.PropertyText.Take"),
        "keywords" => Some("Column.PropertyText.Keywords"),
        "client" => Some("Column.PropertyText.Client"),
        "tape name" => Some("Column.PropertyText.TapeName"),
        "good take" => Some("Column.PropertyText.Good"),
        "angle" => Some("Column.PropertyText.CameraAngle"),
        "camera #" => Some("Column.PropertyText.CameraLabel"),
        _ => None,
    }
}

fn is_valid_xml_tag_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[allow(dead_code)]
fn _unused_msg_check(_: ConversionMessage, _: Severity) {}
