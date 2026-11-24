pub mod fixture;
pub mod lumetri;
pub mod reader;

pub use fixture::{sample_prproj_xml, write_sample_prproj};
pub use reader::{parse_prproj_xml, read_prproj_file, PremiereReader};

#[cfg(test)]
mod tests {
    use super::*;
    use nle_core::Rational;

    #[test]
    fn parses_synthetic_prproj_end_to_end() {
        let xml = sample_prproj_xml();
        let project = parse_prproj_xml(&xml, "Studio Demo").expect("parse sample prproj");
        assert_eq!(project.name, "Studio Demo");
        assert_eq!(project.color_config.working_space.as_deref(), Some("Rec. 709"));
        assert_eq!(project.bins.len(), 2);
        assert_eq!(project.sequences.len(), 1);

        let seq = &project.sequences[0];
        assert_eq!(seq.name, "Main_Edit_v03");
        assert_eq!(seq.width, 1920);
        assert_eq!(seq.height, 1080);
        assert_eq!(seq.frame_rate, Rational::FPS_25);
        assert_eq!(seq.video_tracks.len(), 1);
        assert_eq!(seq.audio_tracks.len(), 1);
        assert_eq!(seq.video_tracks[0].clips.len(), 2);
        assert_eq!(seq.markers.len(), 1);
        assert_eq!(seq.markers[0].name, "Intro Cut");

        let c1 = &seq.video_tracks[0].clips[0];
        assert_eq!(c1.start_frame, 0);
        assert_eq!(c1.end_frame, 125);
        assert!((c1.motion.scale - 55.0).abs() < 1e-6);
        assert_eq!(c1.lumetri_effects.len(), 1);
        assert_eq!(c1.lumetri_effects[0].exposure, Some(0.45));
        assert_eq!(c1.effects.len(), 1);
        assert_eq!(c1.effects[0].match_name, "AE.ADBE Ultra Key");

        let c2 = &seq.video_tracks[0].clips[1];
        assert_eq!(c2.speed_percent, 50.0);
        assert_eq!(c2.effects.len(), 3);
    }
}
