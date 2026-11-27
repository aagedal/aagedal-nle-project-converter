use nle_core::{ConversionOptions, NleFormat, ProjectInspection};
use nle_effects::{analyze_project_effects, apply_effect_decisions, MappingAction, UserEffectDecisions};

#[test]
fn full_premiere_to_resolve_and_interchange_pipeline() {
    let dir = std::env::temp_dir().join("aagedal_nle_e2e");
    let _ = std::fs::create_dir_all(&dir);

    let prproj_path = dir.join("demo_studio.prproj");
    let drp_path = dir.join("demo_studio.drp");
    let fcpxml_path = dir.join("demo_studio.fcpxml");
    let otio_path = dir.join("demo_studio.otio");
    let fcp7_path = dir.join("demo_studio.xml");

    // 1. Generate a real gzipped .prproj file and read it directly
    nle_premiere::write_sample_prproj(&prproj_path).expect("write demo .prproj");
    let mut project = nle_premiere::read_prproj_file(&prproj_path).expect("read demo .prproj");

    // Attach standard file metadata + custom third-party project metadata to verify
    // Premiere <-> Resolve transfer AND verify custom metadata does NOT leak into other NLE formats.
    project
        .custom_metadata_schema
        .insert("External.Item.Id".into(), "External Item ID".into());
    project
        .custom_metadata_schema
        .insert("External.Artifact".into(), "External Artifact".into());
    if let Some(asset) = project.media_assets.first_mut() {
        asset
            .metadata
            .insert("Description".into(), "Studio interview A-cam".into());
        asset
            .custom_metadata
            .insert("External.Item.Id".into(), "ext-item-12345".into());
        asset
            .custom_metadata
            .insert("External.Artifact".into(), r#"{"id":"art-99"}"#.into());
    }

    let inspection = ProjectInspection::from_project(&project);
    assert_eq!(inspection.sequences.len(), 1);
    assert_eq!(inspection.bin_count, 2);

    // 2. Analyze effects for DaVinci Resolve target
    let plan = analyze_project_effects(&project, &[], NleFormat::DaVinciResolve);
    // Motion + Lumetri are NativeAutomatic; Warp Stabilizer, Gaussian Blur, Ultra Key, DeNoise are AskUserSuggested; FilmConvert is Unmapped
    assert_eq!(plan.native_auto_count, 2);
    assert_eq!(plan.needs_user_review_count, 4);
    assert_eq!(plan.unmapped_count, 1);

    // 3. Apply user decisions (override Warp Stabilizer to Fusion Planar Tracker, skip FilmConvert)
    let mut decisions = UserEffectDecisions::default();
    decisions.actions.insert(
        "ae.adbe subspacestabilizer".into(),
        MappingAction::MapToEquivalent {
            target_effect_id: "resolve.fusion.planar_tracker".into(),
            target_display_name: "Fusion → Planar Tracker / Camera Lock".into(),
            add_review_marker: true,
        },
    );
    decisions.actions.insert(
        "ae.rubbermonkey.filmconvertnitrate".into(),
        MappingAction::Skip,
    );

    let fx_summary = apply_effect_decisions(&mut project, &plan, &decisions, true);
    // 4 effects mapped with markers (Ultra Key, Warp Stabilizer, Gaussian Blur, DeNoise); FilmConvert skipped
    assert_eq!(fx_summary.markers_injected, 4);

    // 4. Write native DaVinci Resolve .drp
    let opts = ConversionOptions::default();
    let drp_report = nle_resolve::write_drp_file(&project, &drp_path, &opts).expect("write .drp");
    assert_eq!(drp_report.sequences_converted, 1);
    assert_eq!(drp_report.video_clips_converted, 2);
    assert_eq!(drp_report.audio_clips_converted, 1);
    assert_eq!(drp_report.grades_attached, 1);
    assert_eq!(drp_report.bins_created, 2);
    // 1 original Premiere marker + 4 injected effect review markers = 5 markers
    assert_eq!(drp_report.markers_written, 5);

    // 5. Read the generated .drp back (bidirectional proof!) and export to FCPXML, OTIO, and FCP7 XML
    let resolve_project = nle_resolve::read_drp_file(&drp_path).expect("re-read generated .drp");
    assert_eq!(resolve_project.source_format, NleFormat::DaVinciResolve);
    assert_eq!(resolve_project.sequences.len(), 1);
    assert_eq!(resolve_project.sequences[0].video_tracks[0].clips.len(), 2);
    // Verify Lumetri Basic Correction survived Premiere -> Resolve DRX -> Readback
    let clip0_grades = &resolve_project.sequences[0].video_tracks[0].clips[0].lumetri_effects;
    assert_eq!(clip0_grades.len(), 1, "expected 1 decoded Resolve primary grade node on clip 1");
    assert!((clip0_grades[0].exposure.unwrap() - 0.45).abs() < 0.01);
    assert!((clip0_grades[0].contrast.unwrap() - 18.0).abs() < 0.05);
    assert!((clip0_grades[0].saturation.unwrap() - 112.0).abs() < 0.05);

    // Verify Premiere Warp Stabilizer activated Resolve Inspector Stabilization on clip 2
    assert!(
        resolve_project.sequences[0].video_tracks[0].clips[1]
            .effects
            .iter()
            .any(|e| e.match_name == "resolve.inspector.stabilization"),
        "expected activated Resolve Inspector Stabilization on clip 2"
    );
    // Verify standard + custom/third-party metadata survived Premiere -> Resolve
    assert!(
        resolve_project.media_assets.iter().any(|a| {
            a.metadata.get("Description").map(String::as_str) == Some("Studio interview A-cam")
                && a.custom_metadata.get("External.Item.Id").map(String::as_str)
                    == Some("ext-item-12345")
                && a.custom_metadata.get("External.Artifact").map(String::as_str)
                    == Some(r#"{"id":"art-99"}"#)
        }),
        "expected standard & custom metadata on Resolve media asset"
    );

    let fcp_rep = nle_interchange::write_fcpxml_file(&resolve_project, &fcpxml_path, &opts)
        .expect("write .fcpxml from .drp");
    assert_eq!(fcp_rep.video_clips_converted, 2);

    let otio_rep = nle_interchange::write_otio_file(&resolve_project, &otio_path, &opts)
        .expect("write .otio from .drp");
    assert_eq!(otio_rep.video_clips_converted, 2);

    let xmeml_rep = nle_interchange::write_fcp7_xml_file(&resolve_project, &fcp7_path, &opts)
        .expect("write FCP7 .xml from .drp");
    assert_eq!(xmeml_rep.video_clips_converted, 2);

    // Verify custom/third-party MAM metadata does NOT leak into non-Resolve/Premiere formats
    for other_path in [&fcpxml_path, &otio_path, &fcp7_path] {
        let content = std::fs::read_to_string(other_path).expect("read interchange output");
        assert!(
            !content.contains("External.Item.Id") && !content.contains("ext-item-12345"),
            "custom metadata must not leak into {:?}",
            other_path
        );
    }

    // 6. Native Resolve .drp -> Adobe Premiere Pro .prproj conversion + re-read!
    let back_prproj_path = dir.join("from_resolve.prproj");
    let prproj_opts = ConversionOptions {
        target_format: NleFormat::PremierePro,
        ..Default::default()
    };
    let prproj_rep = nle_premiere::write_prproj_file(&resolve_project, &back_prproj_path, &prproj_opts)
        .expect("write native .prproj from .drp");
    assert_eq!(prproj_rep.sequences_converted, 1);
    assert_eq!(prproj_rep.video_clips_converted, 2);
    assert_eq!(prproj_rep.audio_clips_converted, 1);
    assert!(prproj_rep.markers_written >= 1);

    let reread_prproj = nle_premiere::read_prproj_file(&back_prproj_path)
        .expect("read back native .prproj generated from .drp");
    assert_eq!(reread_prproj.sequences.len(), 1);
    assert_eq!(reread_prproj.sequences[0].video_tracks[0].clips.len(), 2);
    assert!(!reread_prproj.sequences[0].markers.is_empty());
    // Verify Resolve Inspector Stabilization round-tripped back to Premiere Warp Stabilizer on clip 2
    assert!(
        reread_prproj.sequences[0].video_tracks[0].clips[1]
            .effects
            .iter()
            .any(|e| e.match_name == "AE.ADBE SubspaceStabilizer"),
        "expected Warp Stabilizer attached on round-tripped Premiere clip 2"
    );
    assert!(
        reread_prproj.media_assets.iter().any(|a| {
            a.metadata.get("Description").map(String::as_str) == Some("Studio interview A-cam")
                && a.custom_metadata.get("External.Item.Id").map(String::as_str)
                    == Some("ext-item-12345")
                && a.custom_metadata.get("External.Artifact").map(String::as_str)
                    == Some(r#"{"id":"art-99"}"#)
        }),
        "expected standard & custom metadata on round-tripped Premiere media asset"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fcpxml_bundle_to_resolve_and_premiere_pipeline() {
    let dir = std::env::temp_dir().join("aagedal_nle_fcp_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let fcpxmld = dir.join("SampleEdit.fcpxmld");
    std::fs::create_dir_all(&fcpxmld).expect("create fcpxmld dir");

    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.15">
  <resources>
    <format id="r1" name="FFVideoFormat1080p25" frameDuration="100/2500s" width="1920" height="1080" colorSpace="1-1-1 (Rec. 709)"/>
    <asset id="r2" name="Interview_A" uid="UID-A" start="3600s" duration="500/25s" hasVideo="1" format="r1" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/Interview_A.mov"/>
    </asset>
    <asset id="r3" name="Broll_Overlay" uid="UID-B" start="0s" duration="250/25s" hasVideo="1" format="r1" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/Broll_Overlay.mov"/>
    </asset>
    <effect id="r4" name="Gaussian" uid="FxPlug:Gaussian"/>
  </resources>
  <library>
    <event name="Shoot Day 1">
      <project name="FinalCut_Edit">
        <sequence format="r1" duration="200/25s" tcStart="3600s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">
          <spine>
            <asset-clip ref="r2" offset="3600s" name="Interview_A" start="3601s" duration="200/25s">
              <adjust-transform scale="1.1 1.1" rotation="-5"/>
              <adjust-stabilization type="automatic"/>
              <asset-clip ref="r3" lane="1" offset="3603s" name="Broll_Overlay" start="0s" duration="75/25s">
                <adjust-blend amount="0.8"/>
                <filter-video ref="r4" name="Gaussian"/>
              </asset-clip>
              <marker start="3602s" duration="1/25s" value="Key Quote" note="Great delivery"/>
            </asset-clip>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>"#;
    std::fs::write(fcpxmld.join("Info.fcpxml"), xml).expect("write Info.fcpxml");

    // 1. Read .fcpxmld bundle
    let fcp_proj = nle_interchange::read_fcpxml_file(&fcpxmld).expect("read .fcpxmld");
    assert_eq!(fcp_proj.source_format, NleFormat::FinalCutPro);
    assert_eq!(fcp_proj.sequences.len(), 1);
    assert_eq!(fcp_proj.sequences[0].video_tracks.len(), 2);

    // 2. Analyze effects for Resolve target
    let mut for_resolve = fcp_proj.clone();
    let plan_drp = analyze_project_effects(&for_resolve, &[], NleFormat::DaVinciResolve);
    assert!(plan_drp.needs_user_review_count >= 2); // Stabilization + Gaussian Blur
    assert!(plan_drp.native_auto_count >= 1); // Motion (Transform)
    let fx_res = apply_effect_decisions(&mut for_resolve, &plan_drp, &UserEffectDecisions::default(), true);
    assert!(fx_res.markers_injected >= 2);

    // 3. Convert FCP -> Resolve .drp & re-read
    let drp_path = dir.join("from_fcp.drp");
    let drp_rep = nle_resolve::write_drp_file(&for_resolve, &drp_path, &ConversionOptions::default())
        .expect("write .drp from FCP");
    assert_eq!(drp_rep.sequences_converted, 1);
    assert_eq!(drp_rep.video_clips_converted, 2);
    assert_eq!(drp_rep.audio_clips_converted, 2);

    let reread_drp = nle_resolve::read_drp_file(&drp_path).expect("re-read .drp");
    assert_eq!(reread_drp.sequences[0].video_tracks.len(), 2);
    assert!(reread_drp.sequences[0].video_tracks[0].clips[0]
        .effects
        .iter()
        .any(|e| e.match_name == "resolve.inspector.stabilization"));

    // 4. Convert FCP -> Premiere .prproj & re-read
    let mut for_pr = fcp_proj.clone();
    let plan_pr = analyze_project_effects(&for_pr, &[], NleFormat::PremierePro);
    apply_effect_decisions(&mut for_pr, &plan_pr, &UserEffectDecisions::default(), true);
    let pr_path = dir.join("from_fcp.prproj");
    let pr_opts = ConversionOptions {
        target_format: NleFormat::PremierePro,
        ..Default::default()
    };
    let pr_rep = nle_premiere::write_prproj_file(&for_pr, &pr_path, &pr_opts)
        .expect("write .prproj from FCP");
    assert_eq!(pr_rep.sequences_converted, 1);
    assert_eq!(pr_rep.video_clips_converted, 2);

    let reread_pr = nle_premiere::read_prproj_file(&pr_path).expect("re-read .prproj");
    assert_eq!(reread_pr.sequences[0].video_tracks.len(), 2);
    assert!(reread_pr.sequences[0].video_tracks[0].clips[0]
        .effects
        .iter()
        .any(|e| e.match_name == "AE.ADBE SubspaceStabilizer"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn flatten_multicam_and_speed_ramp_across_formats() {
    let dir = std::env::temp_dir().join("aagedal_nle_flatten_ramp_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let fcpxml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.11">
  <resources>
    <format id="r1" name="FFVideoFormat1080p25" frameDuration="1/25s" width="1920" height="1080"/>
    <asset id="r2" name="CamA_ProRes422HQ.mov" uid="UID-A" start="0s" duration="250/25s" hasVideo="1" format="r1" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/CamA_ProRes422HQ.mov"/>
    </asset>
    <asset id="r3" name="CamB_MXF.mxf" uid="UID-B" start="0s" duration="250/25s" hasVideo="1" format="r1" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Media/CamB_MXF.mxf"/>
    </asset>
    <media id="r4" name="Studio_MC">
      <multicam format="r1" tcStart="0s" tcFormat="NDF">
        <mc-angle name="Camera 1" angleID="ang-1">
          <asset-clip ref="r2" offset="0s" name="CamA_ProRes422HQ.mov" start="0s" duration="200/25s"/>
        </mc-angle>
        <mc-angle name="Camera 2" angleID="ang-2">
          <asset-clip ref="r3" offset="0s" name="CamB_MXF.mxf" start="0s" duration="200/25s"/>
        </mc-angle>
      </multicam>
    </media>
  </resources>
  <library>
    <event name="Session">
      <project name="MC_And_Ramp">
        <sequence format="r1" duration="200/25s" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">
          <spine>
            <mc-clip ref="r4" offset="0s" name="Studio_MC" start="0s" duration="50/25s">
              <mc-source angleID="ang-1" srcEnable="all"/>
            </mc-clip>
            <mc-clip ref="r4" offset="50/25s" name="Studio_MC" start="50/25s" duration="50/25s">
              <mc-source angleID="ang-2" srcEnable="all"/>
            </mc-clip>
            <asset-clip ref="r2" offset="100/25s" name="SpeedRampShot" start="0s" duration="100/25s">
              <timeMap>
                <timept time="0s" value="0s" interp="smooth2"/>
                <timept time="50/25s" value="25/25s" interp="smooth2"/>
                <timept time="100/25s" value="125/25s" interp="smooth2"/>
              </timeMap>
            </asset-clip>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>"#;
    let fcpxml_in = dir.join("input.fcpxml");
    std::fs::write(&fcpxml_in, fcpxml).expect("write fcpxml");

    let proj = nle_interchange::read_fcpxml_file(&fcpxml_in).expect("read fcpxml");
    // Verify multi-point speed ramp was parsed from FCPXML <timeMap>
    let main_seq = proj
        .sequences
        .iter()
        .find(|s| s.kind == nle_core::SequenceKind::Timeline)
        .expect("main timeline");
    assert_eq!(main_seq.video_tracks[0].clips[2].speed_keyframes.len(), 3);

    // 1. Export to Resolve .drp WITH flatten_nested_and_multicam = true
    let drp_flat = dir.join("flattened.drp");
    let flat_opts = ConversionOptions {
        target_format: nle_core::NleFormat::DaVinciResolve,
        flatten_nested_and_multicam: true,
        ..Default::default()
    };
    let drp_rep = nle_resolve::write_drp_file(&proj, &drp_flat, &flat_opts).expect("write flat drp");
    assert_eq!(drp_rep.sequences_converted, 1);

    let reread_drp = nle_resolve::read_drp_file(&drp_flat).expect("read flat drp");
    assert_eq!(reread_drp.sequences.len(), 1);
    let clips = &reread_drp.sequences[0].video_tracks[0].clips;
    assert_eq!(clips.len(), 3);
    assert_eq!(clips[0].source_kind, nle_core::ClipSourceKind::Media);
    assert_eq!(clips[0].media_path, "/Media/CamA_ProRes422HQ.mov");
    assert_eq!(clips[1].source_kind, nle_core::ClipSourceKind::Media);
    assert_eq!(clips[1].media_path, "/Media/CamB_MXF.mxf");
    // Speed ramp survived FCPXML -> Resolve .drp -> readback!
    assert_eq!(clips[2].speed_keyframes.len(), 3);

    // Codec metadata in Resolve Media Pool reflects ProRes & MXF (no stale avc1)
    let mxf_asset = reread_drp
        .media_assets
        .iter()
        .find(|a| a.path.ends_with(".mxf"))
        .expect("mxf asset");
    assert_eq!(mxf_asset.video_codec, None);
    assert_eq!(mxf_asset.audio_codec.as_deref(), Some("Linear PCM"));

    // 2. Export original unflattened project directly to OTIO and FCP7 XML -> auto-flattens to real paths!
    let otio_out = dir.join("auto_flat.otio");
    let fcp7_out = dir.join("auto_flat.xml");
    nle_interchange::write_otio_file(&proj, &otio_out, &ConversionOptions::default())
        .expect("write otio");
    nle_interchange::write_fcp7_xml_file(&proj, &fcp7_out, &ConversionOptions::default())
        .expect("write fcp7");
    let otio_txt = std::fs::read_to_string(&otio_out).unwrap();
    let fcp7_txt = std::fs::read_to_string(&fcp7_out).unwrap();
    assert!(otio_txt.contains("CamA_ProRes422HQ.mov") && otio_txt.contains("CamB_MXF.mxf"));
    assert!(fcp7_txt.contains("CamA_ProRes422HQ.mov") && fcp7_txt.contains("CamB_MXF.mxf"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cross_nle_third_party_plugin_matches_and_attaches_natively() {
    use nle_effects::{
        analyze_project_effects_with_plugins, InstalledPlugin, InstalledPluginRegistry,
        MappingFidelity, PluginFormat,
    };

    let dir = std::env::temp_dir().join("aagedal_nle_cross_plugin_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // Deterministic registry mirroring Redact Anonymizer installed in Premiere, Resolve, and FCP
    let registry = InstalledPluginRegistry::from_plugins(vec![
        InstalledPlugin {
            name: "Redact Anonymizer".into(),
            normalized_name: "redact anonymizer".into(),
            target_nle: NleFormat::PremierePro,
            format: PluginFormat::PremiereMediaCore,
            media_kind: nle_core::TrackKind::Video,
            native_id: "AE.AGDL Multi-Layer Anonymizer".into(),
            bundle_id: Some("me.aagedal.premiere.anonymizer".into()),
            category: Some("Aagedal".into()),
            version: Some("2.0.0".into()),
            path: "/Library/Application Support/Adobe/Common/Plug-ins/7.0/MediaCore/Anonymizer.plugin".into(),
            match_aliases: vec![
                "agdl multi layer anonymizer".into(),
                "anonymizer".into(),
                "multilayeranonymizer".into(),
                "redact anonymizer".into(),
            ],
        },
        InstalledPlugin {
            name: "Redact Anonymizer".into(),
            normalized_name: "redact anonymizer".into(),
            target_nle: NleFormat::DaVinciResolve,
            format: PluginFormat::OpenFx,
            media_kind: nle_core::TrackKind::Video,
            native_id: "me.aagedal.ofx.MultiLayerAnonymizer".into(),
            bundle_id: Some("me.aagedal.ofx.MultiLayerAnonymizer".into()),
            category: None,
            version: Some("2.0.0".into()),
            path: "/Library/OFX/Plugins/Anonymizer.ofx.bundle".into(),
            match_aliases: vec![
                "anonymizer".into(),
                "me.aagedal.ofx.multilayeranonymizer".into(),
                "multilayeranonymizer".into(),
                "redact anonymizer".into(),
            ],
        },
        InstalledPlugin {
            name: "Redact Anonymizer".into(),
            normalized_name: "redact anonymizer".into(),
            target_nle: NleFormat::FinalCutPro,
            format: PluginFormat::FxPlug,
            media_kind: nle_core::TrackKind::Video,
            native_id: "FxPlug:211035B9-C3CF-42E6-B8CB-F9C28DC3B98A".into(),
            bundle_id: Some("me.aagedal.fxplug.anonymizer.service".into()),
            category: Some("Aagedal".into()),
            version: Some("2.0.0".into()),
            path: "/Applications/Redact Anonymizer.app/Contents/PlugIns/Redact Anonymizer XPC Service.pluginkit".into(),
            match_aliases: vec![
                "anonymizerfxplugin".into(),
                "redact anonymizer".into(),
            ],
        },
    ]);

    // 1. Start from a Final Cut Pro project containing Redact Anonymizer Motion Effect template
    let fcpxml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.15">
  <resources>
    <format id="r1" name="FFVideoFormat1080p25" frameDuration="1/25s" width="1920" height="1080"/>
    <asset id="r2" name="Interview.mov" uid="UID-1" start="0s" duration="100/25s" hasVideo="1" format="r1">
      <media-rep kind="original-media" src="file:///Media/Interview.mov"/>
    </asset>
    <effect id="r3" name="Redact Anonymizer" uid="~/Effects.localized/Aagedal/Redact Anonymizer/Redact Anonymizer.moef"/>
  </resources>
  <library>
    <event name="News">
      <project name="Anonymized_Interview">
        <sequence format="r1" duration="100/25s" tcStart="0s" tcFormat="NDF">
          <spine>
            <asset-clip ref="r2" offset="0s" name="Interview.mov" start="0s" duration="100/25s">
              <filter-video ref="r3" name="Redact Anonymizer"/>
            </asset-clip>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>"#;
    let fcp_proj = nle_interchange::parse_fcpxml_str(fcpxml, "Anonymized_Interview").unwrap();

    // 2. FCP -> Premiere Pro: Redact Anonymizer matches installed Premiere plugin & attaches natively!
    let mut to_pr = fcp_proj.clone();
    let plan_pr = analyze_project_effects_with_plugins(&to_pr, &[], NleFormat::PremierePro, &registry);
    assert_eq!(plan_pr.unmapped_count, 0);
    assert_eq!(plan_pr.needs_user_review_count, 1);
    assert_eq!(plan_pr.effects[0].fidelity, MappingFidelity::AskUserSuggested);
    assert_eq!(
        plan_pr.effects[0].available_targets[0].id,
        "plugin.premiere:AE.AGDL Multi-Layer Anonymizer"
    );

    let fx_pr = apply_effect_decisions(&mut to_pr, &plan_pr, &UserEffectDecisions::default(), true);
    assert_eq!(fx_pr.markers_injected, 1);
    let pr_out = dir.join("with_redact.prproj");
    nle_premiere::write_prproj_file(
        &to_pr,
        &pr_out,
        &ConversionOptions {
            target_format: NleFormat::PremierePro,
            ..Default::default()
        },
    )
    .expect("write prproj with Redact Anonymizer");

    // Re-read the written .prproj -> verify Redact Anonymizer is natively attached!
    let pr_reread = nle_premiere::read_prproj_file(&pr_out).expect("re-read prproj");
    let pr_clip = &pr_reread.sequences[0].video_tracks[0].clips[0];
    assert!(
        pr_clip
            .effects
            .iter()
            .any(|e| e.match_name == "AE.AGDL Multi-Layer Anonymizer"),
        "expected native Premiere plugin AE.AGDL Multi-Layer Anonymizer on clip, got: {:?}",
        pr_clip.effects
    );

    // 3. Premiere Pro -> Final Cut Pro: Redact Anonymizer matches installed FxPlug & attaches natively!
    let mut to_fcp = pr_reread.clone();
    let plan_fcp = analyze_project_effects_with_plugins(&to_fcp, &[], NleFormat::FinalCutPro, &registry);
    assert_eq!(
        plan_fcp.effects[0].available_targets[0].id,
        "plugin.fcp:FxPlug:211035B9-C3CF-42E6-B8CB-F9C28DC3B98A"
    );
    apply_effect_decisions(&mut to_fcp, &plan_fcp, &UserEffectDecisions::default(), true);
    let fcp_out = dir.join("back_to_fcp.fcpxml");
    nle_interchange::write_fcpxml_file(
        &to_fcp,
        &fcp_out,
        &ConversionOptions {
            target_format: NleFormat::FinalCutPro,
            ..Default::default()
        },
    )
    .expect("write fcpxml with FxPlug");
    let fcp_text = std::fs::read_to_string(&fcp_out).unwrap();
    assert!(fcp_text.contains("FxPlug:211035B9-C3CF-42E6-B8CB-F9C28DC3B98A"));
    assert!(fcp_text.contains("<filter-video"));
    let fcp_reread = nle_interchange::read_fcpxml_file(&fcp_out).expect("re-read fcpxml");
    assert!(fcp_reread.sequences[0].video_tracks[0].clips[0]
        .effects
        .iter()
        .any(|e| e.match_name.contains("FxPlug:211035B9-C3CF-42E6-B8CB-F9C28DC3B98A")));

    // 4. Premiere Pro -> DaVinci Resolve: matches installed Resolve OFX ID
    let plan_drp = analyze_project_effects_with_plugins(&pr_reread, &[], NleFormat::DaVinciResolve, &registry);
    assert_eq!(
        plan_drp.effects[0].available_targets[0].id,
        "plugin.resolve.ofx:me.aagedal.ofx.MultiLayerAnonymizer"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn roundtrips_fades_transitions_opacity_and_blend_modes_across_all_three_nles() {
    use nle_core::*;
    use std::collections::BTreeMap;

    let dir = std::env::temp_dir().join("aagedal_nle_fades_blend_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let mk_clip = |id: &str, name: &str, start: i64, end: i64, opacity: f64, blend: BlendMode, media_kind: MediaKind| Clip {
        id: id.into(),
        name: name.into(),
        media_id: "m1".into(),
        media_path: "/Media/SceneA.mov".into(),
        media_kind,
        source_kind: ClipSourceKind::Media,
        active_angle: None,
        start_frame: start,
        end_frame: end,
        source_in_frame: start,
        source_out_frame: end,
        source_in_ticks: None,
        source_out_ticks: None,
        source_fps: Rational::FPS_25,
        source_width: 1920,
        source_height: 1080,
        speed_percent: 100.0,
        reversed: false,
        speed_keyframes: vec![],
        retime_interpolation: RetimeInterpolation::Default,
        enabled: true,
        audio_channel: if media_kind == MediaKind::Audio { 1 } else { 0 },
        link_group_id: None,
        label_name: None,
        label_preset_index: None,
        motion: MotionTransform::default(),
        opacity,
        blend_mode: blend,
        lumetri_effects: vec![],
        effects: vec![],
        markers: vec![],
    };

    let project = Project {
        name: "FadesAndBlends".into(),
        source_format: NleFormat::PremierePro,
        media_assets: vec![MediaAsset {
            id: "m1".into(),
            name: "SceneA.mov".into(),
            path: "/Media/SceneA.mov".into(),
            media_kind: MediaKind::Video,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 400,
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
        }],
        sequences: vec![Sequence {
            id: "seq-1".into(),
            name: "Fades_Blend_Edit".into(),
            kind: SequenceKind::Timeline,
            width: 1920,
            height: 1080,
            frame_rate: Rational::FPS_25,
            duration_frames: 200,
            start_tc_frame: 0,
            mark_in: None,
            mark_out: None,
            video_tracks: vec![
                Track {
                    index: 1,
                    kind: TrackKind::Video,
                    name: "V1".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 0,
                    output_bus_id: None,
                    clips: vec![
                        mk_clip("v1", "Shot1", 0, 100, 100.0, BlendMode::Normal, MediaKind::Video),
                        mk_clip("v2", "Shot2", 100, 200, 100.0, BlendMode::Normal, MediaKind::Video),
                    ],
                    transitions: vec![
                        // Head fade-in
                        Transition {
                            name: "Cross Dissolve".into(),
                            effect_id: "AE.ADBE Cross Dissolve".into(),
                            start_frame: 0,
                            end_frame: 20,
                            alignment: "start".into(),
                        },
                        // Mid-cut Dip to Black
                        Transition {
                            name: "Dip to Black".into(),
                            effect_id: "AE.ADBE Dip to Black".into(),
                            start_frame: 90,
                            end_frame: 110,
                            alignment: "center".into(),
                        },
                        // Tail fade-out
                        Transition {
                            name: "Cross Dissolve".into(),
                            effect_id: "AE.ADBE Cross Dissolve".into(),
                            start_frame: 180,
                            end_frame: 200,
                            alignment: "end".into(),
                        },
                    ],
                    markers: vec![],
                },
                Track {
                    index: 2,
                    kind: TrackKind::Video,
                    name: "V2".into(),
                    enabled: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    audio_channels: 0,
                    output_bus_id: None,
                    clips: vec![
                        mk_clip("v3", "OverlayFlare", 25, 125, 72.5, BlendMode::Screen, MediaKind::Video),
                        mk_clip("v4", "MultiplyTex", 125, 175, 60.0, BlendMode::Multiply, MediaKind::Video),
                    ],
                    transitions: vec![],
                    markers: vec![],
                },
            ],
            audio_tracks: vec![Track {
                index: 1,
                kind: TrackKind::Audio,
                name: "A1".into(),
                enabled: true,
                locked: false,
                muted: false,
                solo: false,
                audio_channels: 2,
                output_bus_id: None,
                clips: vec![
                    mk_clip("a1", "Audio1", 0, 100, 100.0, BlendMode::Normal, MediaKind::Audio),
                    mk_clip("a2", "Audio2", 100, 200, 100.0, BlendMode::Normal, MediaKind::Audio),
                ],
                transitions: vec![
                    Transition {
                        name: "Constant Power".into(),
                        effect_id: "Constant Power".into(),
                        start_frame: 0,
                        end_frame: 15,
                        alignment: "start".into(),
                    },
                    Transition {
                        name: "Exponential Fade".into(),
                        effect_id: "Exponential Fade".into(),
                        start_frame: 185,
                        end_frame: 200,
                        alignment: "end".into(),
                    },
                ],
                markers: vec![],
            }],
            audio_buses: vec![],
            markers: vec![],
            metadata: BTreeMap::new(),
        }],
        ..Default::default()
    };

    // 1. Premiere IR -> Resolve .drp -> read back
    let drp_path = dir.join("fades_blends.drp");
    nle_resolve::write_drp_file(&project, &drp_path, &ConversionOptions::default()).unwrap();
    let from_drp = nle_resolve::read_drp_file(&drp_path).unwrap();
    let drp_seq = &from_drp.sequences[0];
    assert_eq!(drp_seq.video_tracks[0].transitions.len(), 3);
    assert_eq!(drp_seq.video_tracks[0].transitions[0].alignment, "start");
    assert_eq!(drp_seq.video_tracks[0].transitions[1].name, "Dip To Color Dissolve");
    assert_eq!(drp_seq.video_tracks[0].transitions[2].alignment, "end");
    assert_eq!(drp_seq.audio_tracks[0].transitions.len(), 2);
    assert_eq!(drp_seq.audio_tracks[0].transitions[0].name, "Cross Fade +3DB");
    assert_eq!(drp_seq.audio_tracks[0].transitions[1].name, "Cross Fade -3DB");
    assert!((drp_seq.video_tracks[1].clips[0].opacity - 72.5).abs() < 1e-3);
    assert_eq!(drp_seq.video_tracks[1].clips[0].blend_mode, BlendMode::Screen);
    assert!((drp_seq.video_tracks[1].clips[1].opacity - 60.0).abs() < 1e-3);
    assert_eq!(drp_seq.video_tracks[1].clips[1].blend_mode, BlendMode::Multiply);

    // 2. Resolve -> Premiere .prproj -> read back
    let pr_path = dir.join("fades_blends.prproj");
    nle_premiere::write_prproj_file(
        &from_drp,
        &pr_path,
        &ConversionOptions {
            target_format: NleFormat::PremierePro,
            ..Default::default()
        },
    )
    .unwrap();
    let from_pr = nle_premiere::read_prproj_file(&pr_path).unwrap();
    let pr_seq = &from_pr.sequences[0];
    assert_eq!(pr_seq.video_tracks[0].transitions.len(), 3);
    assert_eq!(pr_seq.video_tracks[0].transitions[0].alignment, "start");
    assert_eq!(pr_seq.video_tracks[0].transitions[1].effect_id, "AE.ADBE Dip to Black");
    assert_eq!(pr_seq.video_tracks[0].transitions[2].alignment, "end");
    assert_eq!(pr_seq.audio_tracks[0].transitions.len(), 2);
    assert_eq!(pr_seq.audio_tracks[0].transitions[0].alignment, "start");
    assert_eq!(pr_seq.audio_tracks[0].transitions[1].effect_id, "Exponential Fade");
    assert_eq!(pr_seq.audio_tracks[0].transitions[1].alignment, "end");
    assert!((pr_seq.video_tracks[1].clips[0].opacity - 72.5).abs() < 1e-3);
    assert_eq!(pr_seq.video_tracks[1].clips[0].blend_mode, BlendMode::Screen);
    assert!((pr_seq.video_tracks[1].clips[1].opacity - 60.0).abs() < 1e-3);
    assert_eq!(pr_seq.video_tracks[1].clips[1].blend_mode, BlendMode::Multiply);

    // 3. Premiere -> Final Cut Pro .fcpxml -> read back
    let fcp_path = dir.join("fades_blends.fcpxml");
    nle_interchange::write_fcpxml_file(
        &from_pr,
        &fcp_path,
        &ConversionOptions {
            target_format: NleFormat::FinalCutPro,
            ..Default::default()
        },
    )
    .unwrap();
    let from_fcp = nle_interchange::read_fcpxml_file(&fcp_path).unwrap();
    let fcp_seq = &from_fcp.sequences[0];
    assert_eq!(fcp_seq.video_tracks[0].transitions.len(), 3);
    assert_eq!(fcp_seq.video_tracks[0].transitions[0].alignment, "start");
    assert_eq!(fcp_seq.video_tracks[0].transitions[1].alignment, "center");
    assert_eq!(fcp_seq.video_tracks[0].transitions[2].alignment, "end");
    assert_eq!(fcp_seq.audio_tracks[0].transitions.len(), 2);
    assert_eq!(fcp_seq.audio_tracks[0].transitions[0].alignment, "start");
    assert_eq!(fcp_seq.audio_tracks[0].transitions[1].alignment, "end");
    assert!((fcp_seq.video_tracks[1].clips[0].opacity - 72.5).abs() < 1e-1);
    assert_eq!(fcp_seq.video_tracks[1].clips[0].blend_mode, BlendMode::Screen);
    assert!((fcp_seq.video_tracks[1].clips[1].opacity - 60.0).abs() < 1e-1);
    assert_eq!(fcp_seq.video_tracks[1].clips[1].blend_mode, BlendMode::Multiply);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn otio_and_fcp7_xml_read_and_convert_to_resolve_and_premiere() {
    use nle_core::{BlendMode, ClipSourceKind};

    let dir = std::env::temp_dir().join("aagedal_nle_otio_fcp7_read_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    // 1. Read OpenTimelineIO (.otio) with media_references, Gap, Transition, LinearTimeWarp, Marker, Effect
    let otio_in = dir.join("editorial.otio");
    let otio_json = r#"{
      "OTIO_SCHEMA": "Timeline.1",
      "name": "OTIO_Edit",
      "global_start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 90000.0 },
      "metadata": { "aagedal_nle_converter": { "width": 1920, "height": 1080 } },
      "tracks": {
        "OTIO_SCHEMA": "Stack.1",
        "name": "tracks",
        "markers": [
          {
            "OTIO_SCHEMA": "Marker.2",
            "name": "VFX Note",
            "color": "CYAN",
            "comment": "Check wire removal",
            "marked_range": {
              "OTIO_SCHEMA": "TimeRange.1",
              "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 50.0 },
              "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 5.0 }
            }
          }
        ],
        "children": [
          {
            "OTIO_SCHEMA": "Track.1",
            "name": "V1",
            "kind": "Video",
            "children": [
              {
                "OTIO_SCHEMA": "Clip.2",
                "name": "Hero_A.mov",
                "active_media_reference_key": "DEFAULT_MEDIA",
                "media_references": {
                  "DEFAULT_MEDIA": {
                    "OTIO_SCHEMA": "ExternalReference.1",
                    "target_url": "file:///Media/Hero_A.mov",
                    "available_range": {
                      "OTIO_SCHEMA": "TimeRange.1",
                      "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 90000.0 },
                      "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 500.0 }
                    }
                  }
                },
                "source_range": {
                  "OTIO_SCHEMA": "TimeRange.1",
                  "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 90025.0 },
                  "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 100.0 }
                },
                "effects": [
                  {
                    "OTIO_SCHEMA": "LinearTimeWarp.1",
                    "name": "LinearTimeWarp",
                    "effect_name": "LinearTimeWarp",
                    "time_scalar": 0.5
                  },
                  {
                    "OTIO_SCHEMA": "Effect.1",
                    "name": "Gaussian Blur",
                    "effect_name": "Gaussian Blur",
                    "metadata": { "radius": 12 }
                  }
                ]
              },
              {
                "OTIO_SCHEMA": "Transition.1",
                "name": "Cross Dissolve",
                "transition_type": "SMPTE_Dissolve",
                "in_offset": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 10.0 },
                "out_offset": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 10.0 }
              },
              {
                "OTIO_SCHEMA": "Clip.2",
                "name": "Hero_B.mov",
                "media_reference": {
                  "OTIO_SCHEMA": "ExternalReference.1",
                  "target_url": "file:///Media/Hero_B.mov"
                },
                "source_range": {
                  "OTIO_SCHEMA": "TimeRange.1",
                  "start_time": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 0.0 },
                  "duration": { "OTIO_SCHEMA": "RationalTime.1", "rate": 25.0, "value": 75.0 }
                },
                "effects": [
                  {
                    "OTIO_SCHEMA": "LinearTimeWarp.1",
                    "name": "LinearTimeWarp",
                    "effect_name": "LinearTimeWarp",
                    "time_scalar": -1.0
                  }
                ]
              }
            ]
          }
        ]
      }
    }"#;
    std::fs::write(&otio_in, otio_json).expect("write otio input");

    let mut otio_proj = nle_interchange::read_otio_file(&otio_in).expect("read .otio");
    assert_eq!(otio_proj.source_format, NleFormat::OpenTimelineIo);
    let otio_plan = analyze_project_effects(&otio_proj, &[], NleFormat::DaVinciResolve);
    // Gaussian Blur should be detected with a Resolve equivalent, while SMPTE_Dissolve is standard native!
    assert_eq!(otio_plan.needs_user_review_count, 1);
    assert_eq!(otio_plan.unmapped_count, 0);
    apply_effect_decisions(&mut otio_proj, &otio_plan, &UserEffectDecisions::default(), true);

    // Convert OTIO -> Resolve .drp and re-read
    let drp_from_otio = dir.join("from_otio.drp");
    let drp_rep = nle_resolve::write_drp_file(
        &otio_proj,
        &drp_from_otio,
        &ConversionOptions::default(),
    )
    .expect("OTIO -> .drp");
    assert_eq!(drp_rep.sequences_converted, 1);
    assert_eq!(drp_rep.video_clips_converted, 2);

    let drp_reread = nle_resolve::read_drp_file(&drp_from_otio).expect("re-read .drp");
    let dseq = &drp_reread.sequences[0];
    assert_eq!(dseq.start_tc_frame, 90_000);
    assert_eq!(dseq.video_tracks[0].clips.len(), 2);
    assert!((dseq.video_tracks[0].clips[0].speed_percent - 50.0).abs() < 0.5);
    assert!(dseq.video_tracks[0].clips[1].reversed);
    assert_eq!(dseq.video_tracks[0].transitions.len(), 1);
    assert!(dseq.markers.iter().any(|m| m.name == "VFX Note"));

    // Convert OTIO -> Premiere .prproj and re-read
    let pr_from_otio = dir.join("from_otio.prproj");
    nle_premiere::write_prproj_file(
        &otio_proj,
        &pr_from_otio,
        &ConversionOptions {
            target_format: NleFormat::PremierePro,
            ..Default::default()
        },
    )
    .expect("OTIO -> .prproj");
    let pr_reread = nle_premiere::read_prproj_file(&pr_from_otio).expect("re-read .prproj");
    assert_eq!(pr_reread.sequences[0].video_tracks[0].clips.len(), 2);
    assert_eq!(pr_reread.sequences[0].video_tracks[0].transitions.len(), 1);

    // 2. Read FCP7 XML (.xml) with bins, -1 transition boundaries, Motion/Opacity/BlendMode, links, and nested seq
    let fcp7_in = dir.join("premiere_export.xml");
    let fcp7_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xmeml>
<xmeml version="5">
  <project>
    <name>Fcp7_E2E</name>
    <children>
      <bin>
        <name>Footage</name>
        <children>
          <clip id="m-clip-1">
            <name>ShotA.mov</name>
            <file id="file-a">
              <name>ShotA.mov</name>
              <pathurl>file://localhost/Media/ShotA.mov</pathurl>
              <rate><timebase>25</timebase><ntsc>FALSE</ntsc></rate>
              <duration>400</duration>
              <timecode>
                <rate><timebase>25</timebase><ntsc>FALSE</ntsc></rate>
                <string>01:00:00:00</string>
                <frame>90000</frame>
              </timecode>
              <media>
                <video><samplecharacteristics><width>1920</width><height>1080</height></samplecharacteristics></video>
                <audio><samplecharacteristics><samplerate>48000</samplerate></samplecharacteristics><channelcount>2</channelcount></audio>
              </media>
            </file>
          </clip>
        </children>
      </bin>
      <sequence id="seq-main">
        <name>FCP7_Main</name>
        <duration>150</duration>
        <rate><timebase>25</timebase><ntsc>FALSE</ntsc></rate>
        <media>
          <video>
            <format><samplecharacteristics><width>1920</width><height>1080</height></samplecharacteristics></format>
            <track>
              <clipitem id="v-1">
                <name>ShotA.mov</name>
                <start>0</start><end>-1</end><in>10</in><out>85</out>
                <compositemode>screen</compositemode>
                <file id="file-a"/>
                <link><linkclipref>v-1</linkclipref><mediatype>video</mediatype></link>
                <link><linkclipref>a-1</linkclipref><mediatype>audio</mediatype></link>
                <filter><effect><name>Basic Motion</name><effectid>basic</effectid><parameter><parameterid>scale</parameterid><value>115</value></parameter></effect></filter>
                <filter><effect><name>Opacity</name><effectid>opacity</effectid><parameter><parameterid>opacity</parameterid><value>80</value></parameter></effect></filter>
              </clipitem>
              <transitionitem>
                <start>65</start><end>85</end><alignment>center</alignment>
                <effect><name>Cross Dissolve</name><effectid>Cross Dissolve</effectid></effect>
              </transitionitem>
              <clipitem id="v-2">
                <name>ShotA_Part2</name>
                <start>-1</start><end>150</end><in>85</in><out>160</out>
                <file id="file-a"/>
              </clipitem>
            </track>
          </video>
          <audio>
            <track>
              <clipitem id="a-1">
                <name>ShotA.mov</name>
                <start>0</start><end>75</end><in>10</in><out>85</out>
                <file id="file-a"/>
                <sourcetrack><mediatype>audio</mediatype><trackindex>1</trackindex></sourcetrack>
                <link><linkclipref>v-1</linkclipref><mediatype>video</mediatype></link>
                <link><linkclipref>a-1</linkclipref><mediatype>audio</mediatype></link>
              </clipitem>
            </track>
          </audio>
        </media>
      </sequence>
    </children>
  </project>
</xmeml>"#;
    std::fs::write(&fcp7_in, fcp7_xml).expect("write fcp7 xml");

    let fcp7_proj = nle_interchange::read_xml_interchange_file(&fcp7_in).expect("read FCP7 .xml");
    assert_eq!(fcp7_proj.source_format, NleFormat::Fcp7Xml);
    assert_eq!(fcp7_proj.bins.len(), 1);
    assert_eq!(fcp7_proj.sequences[0].video_tracks[0].clips[0].end_frame, 75);
    assert_eq!(fcp7_proj.sequences[0].video_tracks[0].clips[1].start_frame, 75);
    assert_eq!(fcp7_proj.sequences[0].video_tracks[0].clips[0].source_kind, ClipSourceKind::Media);

    // Convert FCP7 XML -> Resolve .drp & re-read
    let drp_from_fcp7 = dir.join("from_fcp7.drp");
    let rep_drp = nle_resolve::write_drp_file(
        &fcp7_proj,
        &drp_from_fcp7,
        &ConversionOptions::default(),
    )
    .expect("FCP7 XML -> .drp");
    assert_eq!(rep_drp.bins_created, 1);
    assert_eq!(rep_drp.video_clips_converted, 2);
    assert_eq!(rep_drp.audio_clips_converted, 1);

    let reread_drp_fcp7 = nle_resolve::read_drp_file(&drp_from_fcp7).expect("re-read .drp from fcp7");
    let v0 = &reread_drp_fcp7.sequences[0].video_tracks[0].clips[0];
    assert!((v0.motion.scale - 115.0).abs() < 0.2);
    assert!((v0.opacity - 80.0).abs() < 0.2);
    assert_eq!(v0.blend_mode, BlendMode::Screen);
    assert_eq!(reread_drp_fcp7.sequences[0].video_tracks[0].transitions.len(), 1);

    // Convert FCP7 XML -> Premiere .prproj & re-read
    let pr_from_fcp7 = dir.join("from_fcp7.prproj");
    nle_premiere::write_prproj_file(
        &fcp7_proj,
        &pr_from_fcp7,
        &ConversionOptions {
            target_format: NleFormat::PremierePro,
            ..Default::default()
        },
    )
    .expect("FCP7 XML -> .prproj");
    let reread_pr_fcp7 = nle_premiere::read_prproj_file(&pr_from_fcp7).expect("re-read .prproj from fcp7");
    let pv0 = &reread_pr_fcp7.sequences[0].video_tracks[0].clips[0];
    assert!((pv0.motion.scale - 115.0).abs() < 0.2);
    assert!((pv0.opacity - 80.0).abs() < 0.2);
    assert_eq!(pv0.blend_mode, BlendMode::Screen);
    assert!(pv0.link_group_id.is_some());
    assert_eq!(
        pv0.link_group_id,
        reread_pr_fcp7.sequences[0].audio_tracks[0].clips[0].link_group_id
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn final_cut_pro_to_and_from_edl_and_aaf_pipeline() {
    let dir = std::env::temp_dir().join("aagedal_nle_fcp_edl_aaf_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let fcpxml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.15">
  <resources>
    <format id="r1" name="FFVideoFormat1080p25" frameDuration="1/25s" width="1920" height="1080"/>
    <asset id="r2" name="A001_Scene1.mov" uid="UID-1" start="3600s" duration="500/25s" hasVideo="1" format="r1" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Volumes/RAID/A001_Scene1.mov"/>
    </asset>
    <asset id="r3" name="B002_Scene2.mov" uid="UID-2" start="0s" duration="400/25s" hasVideo="1" format="r1" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Volumes/RAID/B002_Scene2.mov"/>
    </asset>
    <asset id="r4" name="Score.wav" uid="UID-3" start="0s" duration="400/25s" hasAudio="1" audioSources="1" audioChannels="2" audioRate="48000">
      <media-rep kind="original-media" src="file:///Volumes/RAID/Score.wav"/>
    </asset>
  </resources>
  <library>
    <event name="Reel 1">
      <project name="FCP_Handoff">
        <sequence format="r1" duration="200/25s" tcStart="3600s" tcFormat="NDF" audioLayout="stereo" audioRate="48k">
          <spine>
            <asset-clip ref="r2" offset="3600s" name="A001_Scene1.mov" start="3601s" duration="100/25s">
              <marker start="3602s" duration="1/25s" value="Audio Check" note="Boom shadow"/>
            </asset-clip>
            <transition name="Cross Dissolve" offset="3603.6s" duration="20/25s"/>
            <asset-clip ref="r3" offset="3604s" name="B002_Scene2.mov" start="0s" duration="100/25s">
              <asset-clip ref="r4" lane="-1" offset="0s" name="Score.wav" start="0s" duration="100/25s"/>
            </asset-clip>
          </spine>
        </sequence>
      </project>
    </event>
  </library>
</fcpxml>"#;

    let fcp_in = dir.join("fcp_handoff.fcpxml");
    std::fs::write(&fcp_in, fcpxml).expect("write fcpxml");
    let fcp_proj = nle_interchange::read_fcpxml_file(&fcp_in).expect("read fcpxml");

    // 1. Final Cut Pro -> CMX 3600 EDL -> Final Cut Pro (.fcpxml)
    let edl_out = dir.join("handoff.edl");
    let edl_rep = nle_interchange::write_edl_file(
        &fcp_proj,
        &edl_out,
        &ConversionOptions {
            target_format: NleFormat::Edl,
            ..Default::default()
        },
    )
    .expect("FCP -> .edl");
    assert_eq!(edl_rep.sequences_converted, 1);
    assert_eq!(edl_rep.video_clips_converted, 2);

    let from_edl = nle_interchange::read_edl_file(&edl_out).expect("read .edl");
    assert_eq!(from_edl.source_format, NleFormat::Edl);
    assert_eq!(from_edl.sequences[0].video_tracks[0].clips.len(), 2);
    assert_eq!(from_edl.sequences[0].video_tracks[0].transitions.len(), 1);
    assert_eq!(from_edl.sequences[0].markers.len(), 1);

    let fcp_from_edl = dir.join("from_edl.fcpxml");
    nle_interchange::write_fcpxml_file(
        &from_edl,
        &fcp_from_edl,
        &ConversionOptions {
            target_format: NleFormat::FinalCutPro,
            ..Default::default()
        },
    )
    .expect("EDL -> .fcpxml");
    let reread_fcp_edl = nle_interchange::read_fcpxml_file(&fcp_from_edl).expect("re-read .fcpxml");
    assert_eq!(reread_fcp_edl.sequences[0].video_tracks[0].clips.len(), 2);
    assert_eq!(
        reread_fcp_edl.sequences[0].video_tracks[0].clips[0].media_path,
        "/Volumes/RAID/A001_Scene1.mov"
    );

    // 2. Final Cut Pro -> AAF (.aaf) -> Final Cut Pro (.fcpxml) + Resolve (.drp)
    let aaf_out = dir.join("handoff.aaf");
    let aaf_rep = nle_interchange::write_aaf_file(
        &fcp_proj,
        &aaf_out,
        &ConversionOptions {
            target_format: NleFormat::Aaf,
            ..Default::default()
        },
    )
    .expect("FCP -> .aaf");
    assert_eq!(aaf_rep.sequences_converted, 1);
    assert_eq!(aaf_rep.video_clips_converted, 2);
    assert!(aaf_rep.audio_clips_converted >= 1);

    let from_aaf = nle_interchange::read_aaf_file(&aaf_out).expect("read .aaf");
    assert_eq!(from_aaf.source_format, NleFormat::Aaf);
    assert_eq!(from_aaf.sequences[0].start_tc_frame, 90_000);
    assert_eq!(from_aaf.sequences[0].video_tracks[0].clips.len(), 2);
    assert_eq!(from_aaf.sequences[0].video_tracks[0].transitions.len(), 1);
    assert_eq!(from_aaf.sequences[0].markers.len(), 1);

    let fcp_from_aaf = dir.join("from_aaf.fcpxml");
    nle_interchange::write_fcpxml_file(
        &from_aaf,
        &fcp_from_aaf,
        &ConversionOptions {
            target_format: NleFormat::FinalCutPro,
            ..Default::default()
        },
    )
    .expect("AAF -> .fcpxml");
    let reread_fcp_aaf = nle_interchange::read_fcpxml_file(&fcp_from_aaf).expect("re-read .fcpxml");
    assert_eq!(reread_fcp_aaf.sequences[0].video_tracks[0].clips.len(), 2);

    let drp_from_aaf = dir.join("from_aaf.drp");
    nle_resolve::write_drp_file(&from_aaf, &drp_from_aaf, &ConversionOptions::default())
        .expect("AAF -> .drp");
    let reread_drp_aaf = nle_resolve::read_drp_file(&drp_from_aaf).expect("re-read .drp");
    assert_eq!(reread_drp_aaf.sequences[0].video_tracks[0].clips.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
