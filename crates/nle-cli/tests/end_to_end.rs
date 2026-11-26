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
