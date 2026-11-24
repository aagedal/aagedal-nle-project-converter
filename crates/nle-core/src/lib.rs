pub mod error;
pub mod model;
pub mod report;
pub mod time;

pub use error::{NleError, NleResult};
pub use model::*;
pub use report::*;
pub use time::{frames_to_timecode, Rational, ADOBE_TICKS_PER_SECOND};

use std::path::Path;

/// Trait implemented by every NLE project reader (Premiere `.prproj`, Resolve `.drp`, FCPXML, Kdenlive, ...).
pub trait ProjectReader {
    fn format(&self) -> NleFormat;
    fn read_file(&self, path: &Path) -> NleResult<Project>;
}

/// Trait implemented by every NLE project writer.
pub trait ProjectWriter {
    fn format(&self) -> NleFormat;
    fn write_file(
        &self,
        project: &Project,
        output_path: &Path,
        options: &ConversionOptions,
    ) -> NleResult<ConversionReport>;
}
