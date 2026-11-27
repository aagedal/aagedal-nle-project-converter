use nle_core::{
    frames_to_timecode, timecode_to_frames, AlphaMode, BlendMode, Clip, ClipSourceKind,
    ColorConfig, DataLevels, Marker, MarkerScope, MediaAsset, MediaKind, MotionTransform, NleError,
    NleFormat, NleResult, Project, ProjectReader, Rational, RetimeInterpolation, Sequence,
    SequenceKind, Track, TrackKind, Transition,
};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub struct EdlReader;

impl ProjectReader for EdlReader {
    fn format(&self) -> NleFormat {
        NleFormat::Edl
    }

    fn read_file(&self, path: &Path) -> NleResult<Project> {
        read_edl_file(path)
    }
}

/// Read a CMX 3600 `.edl` file into the canonical `Project` IR.
pub fn read_edl_file(path: &Path) -> NleResult<Project> {
    let raw = fs::read_to_string(path)?;
    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("EDL Sequence");
    let mut project = parse_edl_str(&raw, default_name)?;
    project.source_path = Some(path.to_string_lossy().into_owned());
    Ok(project)
}

/// Parse a CMX 3600 EDL document string into the canonical `Project` IR.
pub fn parse_edl_str(raw: &str, default_name: &str) -> NleResult<Project> {
    let lines: Vec<&str> = raw.lines().collect();

    let mut title = default_name.to_string();
    let mut is_drop_frame = false;
    let mut explicit_fps: Option<Rational> = None;

    // Pass 1: Read headers & detect frame rate hints
    let mut max_frame_digit = 0i64;
    let mut m2_rates: Vec<f64> = Vec::new();
    let mut saw_semicolon_tc = false;

    for line in &lines {
        let trimmed = line.trim();
        if let Some(rest) = strip_prefix_ci(trimmed, "TITLE:") {
            let t = rest.trim();
            if !t.is_empty() {
                title = t.to_string();
            }
        } else if let Some(rest) = strip_prefix_ci(trimmed, "FCM:") {
            let mode = rest.trim().to_ascii_uppercase();
            if mode.contains("DROP") && !mode.contains("NON") {
                is_drop_frame = true;
            }
        } else if let Some(rest) = strip_prefix_ci(trimmed, "* FPS:")
            .or_else(|| strip_prefix_ci(trimmed, "* FRAME RATE:"))
            .or_else(|| strip_prefix_ci(trimmed, "* TIMELINE FPS:"))
        {
            if let Ok(fps_f) = rest.trim().parse::<f64>()
                && fps_f > 0.0
            {
                explicit_fps = Some(Rational::from_fps_approx(fps_f));
            }
        } else if let Some(m2_fps) = parse_m2_line(trimmed).map(|(_, r)| r.abs())
            && m2_fps > 0.0
        {
            m2_rates.push(m2_fps);
        }

        // Inspect all timecode tokens for max frame number & semicolon drop-frame
        for tok in trimmed.split_whitespace() {
            if is_timecode_token(tok) {
                if tok.contains(';') {
                    saw_semicolon_tc = true;
                }
                if let Some(ff) = tok
                    .rsplit([':', ';'])
                    .next()
                    .and_then(|s| s.parse::<i64>().ok())
                {
                    max_frame_digit = max_frame_digit.max(ff);
                }
            }
        }
    }

    let seq_fps = infer_edl_fps(
        explicit_fps,
        is_drop_frame || saw_semicolon_tc,
        max_frame_digit,
        &m2_rates,
    );

    // Pass 2: Parse CMX 3600 events and attached comment/M2 lines
    let mut raw_events: Vec<RawEdlEvent> = Vec::new();
    let mut raw_markers: Vec<RawEdlMarker> = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(ev) = parse_event_line(trimmed, seq_fps) {
            raw_events.push(ev);
            continue;
        }

        // M2 motion-memory speed line
        if let Some((m2_reel, m2_fps)) = parse_m2_line(trimmed) {
            let target_idx = raw_events
                .iter()
                .rposition(|e| e.reel.eq_ignore_ascii_case(&m2_reel))
                .or_else(|| raw_events.len().checked_sub(1));
            if let Some(idx) = target_idx {
                raw_events[idx].m2_fps = Some(m2_fps);
            }
            continue;
        }

        // Extended audio tracks line: e.g. `AUD  3  4`
        if let Some(rest) = strip_prefix_ci(trimmed, "AUD ") {
            if let Some(last) = raw_events.last_mut() {
                for tok in rest.split_whitespace() {
                    if let Ok(ch) = tok.parse::<usize>()
                        && ch >= 1
                        && !last.audio_track_indices.contains(&ch)
                    {
                        last.audio_track_indices.push(ch);
                    }
                }
            }
            continue;
        }

        // Comment / metadata / marker lines (`* ...` or Resolve pipe-markers)
        if trimmed.starts_with('*') || trimmed.contains("|C:") {
            // Avid `* LOC: 01:00:02:00 RED Comment text`
            if let Some(loc_rest) = strip_prefix_ci(trimmed, "* LOC:")
                && let Some(mk) = parse_avid_loc_marker(loc_rest, seq_fps)
            {
                raw_markers.push(mk);
                continue;
            }

            // DaVinci Resolve EDL marker line: `|C:ResolveColorBlue |M:Name |D:1`
            if trimmed.contains("|C:") || trimmed.contains("|M:") {
                let rec_frame = raw_events.last().map(|e| e.rec_in).unwrap_or(0);
                if let Some(mk) = parse_resolve_pipe_marker(trimmed, rec_frame) {
                    raw_markers.push(mk);
                }
                continue;
            }

            if let Some(last) = raw_events.last_mut() {
                if let Some(val) = strip_prefix_ci(trimmed, "* FROM CLIP NAME:")
                    .or_else(|| strip_prefix_ci(trimmed, "* CLIP NAME:"))
                {
                    let v = val.trim();
                    if !v.is_empty() {
                        last.from_clip_name = Some(v.to_string());
                    }
                } else if let Some(val) = strip_prefix_ci(trimmed, "* TO CLIP NAME:") {
                    let v = val.trim();
                    if !v.is_empty() {
                        last.to_clip_name = Some(v.to_string());
                    }
                } else if let Some(val) = strip_prefix_ci(trimmed, "* SOURCE FILE:")
                    .or_else(|| strip_prefix_ci(trimmed, "* SOURCE FILE PATH:"))
                    .or_else(|| strip_prefix_ci(trimmed, "* FILE:"))
                {
                    let v = val.trim();
                    if !v.is_empty() {
                        last.source_file = Some(v.to_string());
                    }
                } else if let Some(val) = strip_prefix_ci(trimmed, "* RESOLVE COLOR:")
                    .or_else(|| strip_prefix_ci(trimmed, "* CLIP COLOR:"))
                {
                    let v = val.trim();
                    if !v.is_empty() {
                        last.label_color = Some(v.to_string());
                    }
                }
            }
        }
    }

    if raw_events.is_empty() {
        return Err(NleError::InvalidFormat(
            "No valid CMX 3600 edit events found in EDL".into(),
        ));
    }

    // Determine timeline start TC from the earliest event's record-in
    let min_rec_in = raw_events.iter().map(|e| e.rec_in).min().unwrap_or(0);
    let one_hour = 3600 * seq_fps.timebase();
    let start_tc_frame = if min_rec_in >= one_hour && min_rec_in < one_hour + 3600 * seq_fps.timebase()
    {
        // Standard 01:00:00:00 timeline start when first event is at or shortly after 1 hour
        if min_rec_in == one_hour {
            one_hour
        } else {
            min_rec_in
        }
    } else {
        min_rec_in.max(0)
    };

    // Pass 3: Convert raw CMX events into video/audio tracks, resolving dissolves & wipes
    let mut video_tracks_map: BTreeMap<usize, Track> = BTreeMap::new();
    let mut audio_tracks_map: BTreeMap<usize, Track> = BTreeMap::new();
    let mut assets: Vec<MediaAsset> = Vec::new();
    let mut asset_idx_by_key: HashMap<String, usize> = HashMap::new();
    let mut next_clip_id = 1usize;

    fn ensure_vtrack(map: &mut BTreeMap<usize, Track>, idx: usize) -> &mut Track {
        map.entry(idx).or_insert_with(|| Track {
            index: idx,
            kind: TrackKind::Video,
            name: format!("V{idx}"),
            enabled: true,
            locked: false,
            muted: false,
            solo: false,
            audio_channels: 0,
            output_bus_id: None,
            clips: Vec::new(),
            transitions: Vec::new(),
            markers: Vec::new(),
        })
    }
    fn ensure_atrack(map: &mut BTreeMap<usize, Track>, idx: usize, channels: u32) -> &mut Track {
        map.entry(idx).or_insert_with(|| Track {
            index: idx,
            kind: TrackKind::Audio,
            name: format!("A{idx}"),
            enabled: true,
            locked: false,
            muted: false,
            solo: false,
            audio_channels: channels,
            output_bus_id: None,
            clips: Vec::new(),
            transitions: Vec::new(),
            markers: Vec::new(),
        })
    }

    for ev in &raw_events {
        let is_blank = ev.reel.eq_ignore_ascii_case("BL") || ev.reel.eq_ignore_ascii_case("BLK");

        // Handle dissolve / wipe transition attached to this event
        if let Some((tr_kind, tr_dur)) = ev.transition {
            let tr_start_rel = (ev.rec_in - start_tc_frame).max(0);
            let tr_end_rel = tr_start_rel + tr_dur.max(1);
            let cut_rel = tr_start_rel + (tr_dur / 2);

            let (tr_name, tr_id) = match tr_kind {
                EdlTransitionKind::Dissolve => ("Cross Dissolve", "AE.ADBE Cross Dissolve"),
                EdlTransitionKind::Wipe(code) => {
                    let _ = code;
                    ("Wipe", "Wipe")
                }
            };

            if ev.has_video {
                let vtrack = ensure_vtrack(&mut video_tracks_map, ev.video_track_index);
                // If previous clip ended right at tr_start_rel (standard CMX dissolve pair),
                // extend outgoing clip to cut_rel and start incoming clip at cut_rel!
                if let Some(prev_clip) = vtrack.clips.last_mut()
                    && (prev_clip.end_frame - tr_start_rel).abs() <= 1
                {
                    let delta = cut_rel - prev_clip.end_frame;
                    prev_clip.end_frame = cut_rel;
                    prev_clip.source_out_frame += delta.max(0);
                }
                vtrack.transitions.push(Transition {
                    name: tr_name.to_string(),
                    effect_id: tr_id.to_string(),
                    start_frame: tr_start_rel,
                    end_frame: tr_end_rel,
                    alignment: "center".to_string(),
                });
            }
            for &a_idx in &ev.audio_track_indices {
                let atrack = ensure_atrack(&mut audio_tracks_map, a_idx, ev.audio_channels);
                if let Some(prev_clip) = atrack.clips.last_mut()
                    && (prev_clip.end_frame - tr_start_rel).abs() <= 1
                {
                    let delta = cut_rel - prev_clip.end_frame;
                    prev_clip.end_frame = cut_rel;
                    prev_clip.source_out_frame += delta.max(0);
                }
                atrack.transitions.push(Transition {
                    name: "Constant Power".to_string(),
                    effect_id: "Constant Power".to_string(),
                    start_frame: tr_start_rel,
                    end_frame: tr_end_rel,
                    alignment: "center".to_string(),
                });
            }
        }

        // Skip blank/black filler events AND zero-length outgoing dissolve anchor lines
        if is_blank || ev.rec_out <= ev.rec_in {
            continue;
        }

        let mut start_frame = (ev.rec_in - start_tc_frame).max(0);
        let end_frame = (ev.rec_out - start_tc_frame).max(start_frame + 1);
        let mut src_in = ev.src_in.max(0);

        // If this event starts with a dissolve/wipe after an outgoing clip, shift its start to the midpoint cut
        if let Some((_, tr_dur)) = ev.transition {
            let half = tr_dur / 2;
            if start_frame + half < end_frame {
                start_frame += half;
                src_in += half;
            }
        }

        let clip_name = ev
            .to_clip_name
            .as_deref()
            .or(ev.from_clip_name.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or(&ev.reel)
            .to_string();
        let media_path = ev
            .source_file
            .clone()
            .unwrap_or_else(|| clip_name.clone());

        let (speed_percent, reversed) = if let Some(m2) = ev.m2_fps {
            if m2.abs() < 1e-4 {
                (0.0, false)
            } else {
                let pct = (m2.abs() / seq_fps.as_f64()) * 100.0;
                (pct, m2 < 0.0)
            }
        } else {
            (100.0, false)
        };

        let src_out = if speed_percent < 0.01 {
            src_in
        } else if ev.src_out > src_in {
            ev.src_out
        } else {
            let tl_dur = (end_frame - start_frame).max(1) as f64;
            src_in + (tl_dur * (speed_percent / 100.0)).round().max(1.0) as i64
        };

        // Register MediaAsset (storing source_tc_frame = 0 so clip.source_in_frame holds exact tape/file TC frames,
        // or if source_in is >= 1 hour we store asset TC offset when consistent)
        let asset_key = media_path.clone();
        let media_kind = if ev.has_video {
            MediaKind::Video
        } else {
            MediaKind::Audio
        };
        let asset_id = if let Some(&idx) = asset_idx_by_key.get(&asset_key) {
            let a = &mut assets[idx];
            if src_out > a.duration_frames {
                a.duration_frames = src_out;
            }
            a.id.clone()
        } else {
            let id = format!("edl-asset-{}", assets.len() + 1);
            let mut meta = BTreeMap::new();
            if !ev.reel.is_empty() && !ev.reel.eq_ignore_ascii_case("AX") {
                meta.insert("Tape Name".to_string(), ev.reel.clone());
            }
            let asset = MediaAsset {
                id: id.clone(),
                name: clip_name.clone(),
                path: media_path.clone(),
                media_kind,
                width: if ev.has_video { 1920 } else { 0 },
                height: if ev.has_video { 1080 } else { 0 },
                frame_rate: seq_fps,
                duration_frames: src_out.max(end_frame - start_frame),
                source_tc_frame: 0,
                source_tc_string: frames_to_timecode(0, seq_fps),
                audio_channels: if !ev.audio_track_indices.is_empty() {
                    ev.audio_channels
                } else {
                    0
                },
                audio_sample_rate: if !ev.audio_track_indices.is_empty() {
                    48_000
                } else {
                    0
                },
                color_space: None,
                video_codec: None,
                audio_codec: None,
                alpha_mode: AlphaMode::Default,
                data_levels: DataLevels::Auto,
                pixel_aspect_ratio: None,
                metadata: meta,
                custom_metadata: BTreeMap::new(),
            };
            let idx = assets.len();
            assets.push(asset);
            asset_idx_by_key.insert(asset_key, idx);
            id
        };

        let link_group_id = if ev.has_video && !ev.audio_track_indices.is_empty() {
            Some(format!("edl-ev-{}", ev.event_num))
        } else {
            None
        };

        if ev.has_video {
            let vtrack = ensure_vtrack(&mut video_tracks_map, ev.video_track_index);
            let cid = format!("edl-clip-{next_clip_id}");
            next_clip_id += 1;
            vtrack.clips.push(Clip {
                id: cid,
                name: clip_name.clone(),
                media_id: media_id_clone(&asset_id),
                media_path: media_path.clone(),
                media_kind: MediaKind::Video,
                source_kind: ClipSourceKind::Media,
                active_angle: None,
                start_frame,
                end_frame,
                source_in_frame: src_in,
                source_out_frame: src_out,
                source_in_ticks: None,
                source_out_ticks: None,
                source_fps: seq_fps,
                source_width: 1920,
                source_height: 1080,
                speed_percent,
                reversed,
                speed_keyframes: Vec::new(),
                retime_interpolation: RetimeInterpolation::Default,
                enabled: true,
                audio_channel: 0,
                link_group_id: link_group_id.clone(),
                label_name: ev.label_color.clone(),
                label_preset_index: None,
                motion: MotionTransform::default(),
                opacity: 100.0,
                blend_mode: BlendMode::Normal,
                lumetri_effects: Vec::new(),
                effects: Vec::new(),
                markers: Vec::new(),
            });
        }

        for &a_idx in &ev.audio_track_indices {
            let atrack = ensure_atrack(&mut audio_tracks_map, a_idx, ev.audio_channels);
            let cid = format!("edl-clip-{next_clip_id}");
            next_clip_id += 1;
            atrack.clips.push(Clip {
                id: cid,
                name: clip_name.clone(),
                media_id: asset_id.clone(),
                media_path: media_path.clone(),
                media_kind: MediaKind::Audio,
                source_kind: ClipSourceKind::Media,
                active_angle: None,
                start_frame,
                end_frame,
                source_in_frame: src_in,
                source_out_frame: src_out,
                source_in_ticks: None,
                source_out_ticks: None,
                source_fps: seq_fps,
                source_width: 0,
                source_height: 0,
                speed_percent,
                reversed,
                speed_keyframes: Vec::new(),
                retime_interpolation: RetimeInterpolation::Default,
                enabled: true,
                audio_channel: a_idx as u32,
                link_group_id: link_group_id.clone(),
                label_name: ev.label_color.clone(),
                label_preset_index: None,
                motion: MotionTransform::default(),
                opacity: 100.0,
                blend_mode: BlendMode::Normal,
                lumetri_effects: Vec::new(),
                effects: Vec::new(),
                markers: Vec::new(),
            });
        }
    }

    let markers: Vec<Marker> = raw_markers
        .into_iter()
        .map(|m| Marker {
            frame: (m.rec_frame - start_tc_frame).max(0),
            duration_frames: m.duration_frames.max(1),
            name: m.name,
            comment: m.comment,
            color: m.color,
            scope: MarkerScope::Sequence,
        })
        .collect();

    let video_tracks: Vec<Track> = video_tracks_map.into_values().collect();
    let audio_tracks: Vec<Track> = audio_tracks_map.into_values().collect();

    let mut seq = Sequence {
        id: Uuid::new_v4().to_string(),
        name: title.clone(),
        kind: SequenceKind::Timeline,
        width: 1920,
        height: 1080,
        frame_rate: seq_fps,
        duration_frames: 0,
        start_tc_frame,
        mark_in: None,
        mark_out: None,
        video_tracks,
        audio_tracks,
        audio_buses: Vec::new(),
        markers,
        metadata: BTreeMap::new(),
    };
    seq.duration_frames = seq.computed_duration_frames();

    let mut metadata = BTreeMap::new();
    metadata.insert("edl_format".into(), "CMX3600".into());

    Ok(Project {
        name: title,
        source_format: NleFormat::Edl,
        source_path: None,
        color_config: ColorConfig::default(),
        scale_to_frame_size: true,
        bins: Vec::new(),
        media_assets: assets,
        sequences: vec![seq],
        metadata,
        custom_metadata_schema: BTreeMap::new(),
    })
}

fn media_id_clone(s: &str) -> String {
    s.to_string()
}

// ── Internal EDL Line Parsers ───────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum EdlTransitionKind {
    Dissolve,
    Wipe(u32),
}

struct RawEdlEvent {
    event_num: u32,
    reel: String,
    has_video: bool,
    video_track_index: usize,
    audio_track_indices: Vec<usize>,
    audio_channels: u32,
    transition: Option<(EdlTransitionKind, i64)>,
    src_in: i64,
    src_out: i64,
    rec_in: i64,
    rec_out: i64,
    m2_fps: Option<f64>,
    from_clip_name: Option<String>,
    to_clip_name: Option<String>,
    source_file: Option<String>,
    label_color: Option<String>,
}

struct RawEdlMarker {
    rec_frame: i64,
    duration_frames: i64,
    name: String,
    comment: String,
    color: String,
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn is_timecode_token(tok: &str) -> bool {
    let parts: Vec<&str> = tok.split([':', ';']).collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()))
}

fn infer_edl_fps(
    explicit: Option<Rational>,
    is_drop_frame: bool,
    max_frame_digit: i64,
    m2_rates: &[f64],
) -> Rational {
    if let Some(fps) = explicit {
        return fps;
    }
    if is_drop_frame {
        return if max_frame_digit >= 30 {
            Rational::FPS_59_94
        } else {
            Rational::FPS_29_97
        };
    }
    if max_frame_digit >= 50 {
        return Rational::FPS_60;
    }
    if max_frame_digit >= 30 {
        return Rational::FPS_50;
    }
    if max_frame_digit >= 25 {
        return Rational::FPS_30;
    }
    if max_frame_digit == 24 {
        return Rational::FPS_25;
    }
    // Check M2 rates for common 100%-speed or standard base rates (e.g. 24.0, 23.976, 25.0, 29.97, 30.0)
    for &r in m2_rates {
        if (r - 24.0).abs() < 0.01 {
            return Rational::FPS_24;
        }
        if (r - 23.976).abs() < 0.015 {
            return Rational::FPS_23_976;
        }
        if (r - 25.0).abs() < 0.01 {
            return Rational::FPS_25;
        }
        if (r - 29.97).abs() < 0.015 {
            return Rational::FPS_29_97;
        }
        if (r - 30.0).abs() < 0.01 {
            return Rational::FPS_30;
        }
    }
    Rational::FPS_25
}

fn parse_event_line(line: &str, fps: Rational) -> Option<RawEdlEvent> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 8 {
        return None;
    }
    let event_num = tokens[0].parse::<u32>().ok()?;
    // Last 4 tokens must be the 4 SMPTE timecodes: src_in src_out rec_in rec_out
    let n = tokens.len();
    if !is_timecode_token(tokens[n - 4])
        || !is_timecode_token(tokens[n - 3])
        || !is_timecode_token(tokens[n - 2])
        || !is_timecode_token(tokens[n - 1])
    {
        return None;
    }

    let src_in = timecode_to_frames(tokens[n - 4], fps)?;
    let src_out = timecode_to_frames(tokens[n - 3], fps)?;
    let rec_in = timecode_to_frames(tokens[n - 2], fps)?;
    let rec_out = timecode_to_frames(tokens[n - 1], fps)?;

    let reel = tokens[1].to_string();
    let track_tok = tokens[2];
    let edit_tok = tokens[3].to_ascii_uppercase();

    let (has_video, video_track_index, audio_track_indices, audio_channels) =
        parse_track_selector(track_tok);

    let transition = if edit_tok == "D" {
        let dur = tokens
            .get(4)
            .filter(|t| !is_timecode_token(t))
            .and_then(|t| t.parse::<i64>().ok())
            .unwrap_or(0);
        if dur > 0 {
            Some((EdlTransitionKind::Dissolve, dur))
        } else {
            None
        }
    } else if let Some(wrest) = edit_tok.strip_prefix('W') {
        let code = wrest.parse::<u32>().unwrap_or(1);
        let dur = tokens
            .get(4)
            .filter(|t| !is_timecode_token(t))
            .and_then(|t| t.parse::<i64>().ok())
            .unwrap_or(0);
        if dur > 0 {
            Some((EdlTransitionKind::Wipe(code), dur))
        } else {
            None
        }
    } else {
        None
    };

    Some(RawEdlEvent {
        event_num,
        reel,
        has_video,
        video_track_index,
        audio_track_indices,
        audio_channels,
        transition,
        src_in,
        src_out,
        rec_in,
        rec_out,
        m2_fps: None,
        from_clip_name: None,
        to_clip_name: None,
        source_file: None,
        label_color: None,
    })
}

fn parse_track_selector(tok: &str) -> (bool, usize, Vec<usize>, u32) {
    let up = tok.to_ascii_uppercase();
    match up.as_str() {
        "V" | "V1" => (true, 1, Vec::new(), 0),
        "A" | "A1" => (false, 1, vec![1], 1),
        "A2" => (false, 1, vec![2], 1),
        "AA" | "A1/2" | "A12" => (false, 1, vec![1], 2),
        "B" | "AA/V" | "V/AA" | "VA" | "AV" => (true, 1, vec![1], 2),
        "NONE" => (false, 1, Vec::new(), 0),
        _ => {
            if let Some(rest) = up.strip_prefix('V')
                && let Ok(idx) = rest.parse::<usize>()
            {
                return (true, idx.max(1), Vec::new(), 0);
            }
            if let Some(rest) = up.strip_prefix('A')
                && let Ok(idx) = rest.parse::<usize>()
            {
                return (false, 1, vec![idx.max(1)], 1);
            }
            (true, 1, Vec::new(), 0)
        }
    }
}

fn parse_m2_line(line: &str) -> Option<(String, f64)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 3 || !tokens[0].eq_ignore_ascii_case("M2") {
        return None;
    }
    let reel = tokens[1].to_string();
    let fps = tokens[2].parse::<f64>().ok()?;
    Some((reel, fps))
}

fn parse_avid_loc_marker(rest: &str, fps: Rational) -> Option<RawEdlMarker> {
    let mut parts = rest.split_whitespace();
    let tc = parts.next()?;
    let rec_frame = timecode_to_frames(tc, fps)?;
    let color_raw = parts.next().unwrap_or("BLUE");
    let tail: String = parts.collect::<Vec<_>>().join(" ");
    if (tail.contains("|C:") || tail.contains("|M:"))
        && let Some(mut mk) = parse_resolve_pipe_marker(&tail, rec_frame)
    {
        if mk.color.is_empty() {
            mk.color = normalize_edl_color(color_raw);
        }
        return Some(mk);
    }
    let name = if tail.is_empty() {
        "Locator".to_string()
    } else {
        tail.chars().take(48).collect()
    };
    Some(RawEdlMarker {
        rec_frame,
        duration_frames: 1,
        name,
        comment: tail,
        color: normalize_edl_color(color_raw),
    })
}

fn parse_resolve_pipe_marker(line: &str, rec_frame: i64) -> Option<RawEdlMarker> {
    // Format: `* Comment text |C:ResolveColorBlue |M:Marker Name |D:1`
    let cleaned = line.trim().trim_start_matches('*').trim();
    let mut comment = String::new();
    let mut name = String::new();
    let mut color = "Blue".to_string();
    let mut duration_frames = 1i64;

    for (idx, seg) in cleaned.split('|').enumerate() {
        let s = seg.trim();
        if idx == 0 && !s.starts_with("C:") && !s.starts_with("M:") && !s.starts_with("D:") {
            comment = s.to_string();
        } else if let Some(c) = strip_prefix_ci(s, "C:") {
            color = normalize_edl_color(c.trim());
        } else if let Some(m) = strip_prefix_ci(s, "M:") {
            name = m.trim().to_string();
        } else if let Some(d) = strip_prefix_ci(s, "D:")
            && let Ok(df) = d.trim().parse::<i64>()
        {
            duration_frames = df.max(1);
        }
    }

    if name.is_empty() && comment.is_empty() {
        return None;
    }
    if name.is_empty() {
        name = comment.clone();
    }
    Some(RawEdlMarker {
        rec_frame,
        duration_frames,
        name,
        comment,
        color,
    })
}

fn normalize_edl_color(raw: &str) -> String {
    let trimmed = raw
        .trim()
        .strip_prefix("ResolveColor")
        .or_else(|| raw.trim().strip_prefix("resolvecolor"))
        .unwrap_or(raw.trim());
    match trimmed.to_ascii_lowercase().as_str() {
        "red" | "rose" => "Red".into(),
        "green" | "mint" => "Green".into(),
        "blue" => "Blue".into(),
        "cyan" | "sky" => "Cyan".into(),
        "yellow" | "lemon" => "Yellow".into(),
        "orange" | "sand" => "Orange".into(),
        "purple" | "fuchsia" | "lavender" => "Purple".into(),
        "pink" | "magenta" => "Pink".into(),
        "white" | "cream" => "White".into(),
        other if !other.is_empty() => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => "Blue".into(),
            }
        }
        _ => "Blue".into(),
    }
}
