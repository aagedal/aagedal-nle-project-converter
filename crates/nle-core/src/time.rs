use serde::{Deserialize, Serialize};
use std::fmt;

/// Adobe Premiere Pro internal ticks per second (254,016,000,000).
/// Divisible by all standard integer and NTSC film/broadcast frame rates and audio rates.
pub const ADOBE_TICKS_PER_SECOND: i64 = 254_016_000_000;

/// Exact rational number (`num / den`) for drift-free NLE time math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const ZERO: Self = Self { num: 0, den: 1 };
    pub const FPS_23_976: Self = Self { num: 24000, den: 1001 };
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    pub const FPS_25: Self = Self { num: 25, den: 1 };
    pub const FPS_29_97: Self = Self { num: 30000, den: 1001 };
    pub const FPS_30: Self = Self { num: 30, den: 1 };
    pub const FPS_50: Self = Self { num: 50, den: 1 };
    pub const FPS_59_94: Self = Self { num: 60000, den: 1001 };
    pub const FPS_60: Self = Self { num: 60, den: 1 };

    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "Rational denominator cannot be zero");
        let (n, d) = if den < 0 { (-num, -den) } else { (num, den) };
        if n == 0 {
            return Self::ZERO;
        }
        let g = gcd(n.unsigned_abs(), d.unsigned_abs()) as i64;
        Self { num: n / g, den: d / g }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Snap a floating-point FPS (or Adobe tick duration) to the nearest canonical rational FPS.
    pub fn from_fps_approx(fps: f64) -> Self {
        if !fps.is_finite() || fps <= 0.0 {
            return Self::FPS_30;
        }
        const KNOWN: &[Rational] = &[
            Rational::FPS_23_976,
            Rational::FPS_24,
            Rational::FPS_25,
            Rational::FPS_29_97,
            Rational::FPS_30,
            Rational { num: 48000, den: 1001 },
            Rational { num: 48, den: 1 },
            Rational::FPS_50,
            Rational::FPS_59_94,
            Rational::FPS_60,
            Rational { num: 120, den: 1 },
        ];
        for &rate in KNOWN {
            if (rate.as_f64() - fps).abs() < 0.015 {
                return rate;
            }
        }
        Self::new((fps * 1000.0).round() as i64, 1000)
    }

    /// Convert an Adobe Premiere `FrameRate` tick duration (ticks per frame) into an exact `Rational` FPS.
    pub fn from_adobe_frame_ticks(frame_ticks: i64) -> Self {
        if frame_ticks <= 0 || frame_ticks >= 9_000_000_000_000_000_000 {
            return Self::FPS_30;
        }
        let approx = ADOBE_TICKS_PER_SECOND as f64 / frame_ticks as f64;
        Self::from_fps_approx(approx)
    }

    /// Convert Adobe pproTicks to frame count at this FPS (using 128-bit integer arithmetic).
    pub fn adobe_ticks_to_frames(self, ticks: i64) -> i64 {
        if ticks <= 0 {
            return 0;
        }
        // frames = round(ticks / ADOBE_TICKS_PER_SECOND * (num / den))
        let numerator = (ticks as i128) * (self.num as i128);
        let denominator = (ADOBE_TICKS_PER_SECOND as i128) * (self.den as i128);
        ((numerator + denominator / 2) / denominator) as i64
    }

    /// Convert frame count at this FPS back to Adobe pproTicks.
    pub fn frames_to_adobe_ticks(self, frames: i64) -> i64 {
        let numerator = (frames as i128) * (ADOBE_TICKS_PER_SECOND as i128) * (self.den as i128);
        let denominator = self.num as i128;
        ((numerator + denominator / 2) / denominator) as i64
    }

    /// Whether this frame rate is an NTSC 1000/1001 fractional rate (23.976, 29.97, 59.94, etc.).
    pub fn is_ntsc(self) -> bool {
        self.den == 1001
    }

    /// Integer timebase (e.g. 24 for 23.976, 30 for 29.97, 25 for 25).
    pub fn timebase(self) -> i64 {
        self.as_f64().round() as i64
    }

    /// Duration of a single frame as a rational number of seconds (inverse of FPS).
    pub fn frame_duration(self) -> Self {
        Self::new(self.den, self.num)
    }

    /// Convert a frame count at this FPS into FCPXML rational time string (e.g. `"1001/24000s"` or `"0s"`).
    pub fn frames_to_fcpxml(self, frames: i64) -> String {
        if frames == 0 {
            return "0s".to_string();
        }
        let sec = Self::new(frames * self.den, self.num);
        if sec.den == 1 {
            format!("{}s", sec.num)
        } else {
            format!("{}/{}s", sec.num, sec.den)
        }
    }

    /// Human-readable frame rate label (e.g. "23.976", "25", "29.97", "59.94").
    pub fn label(self) -> String {
        if self.den == 1 {
            format!("{}", self.num)
        } else {
            let v = format!("{:.3}", self.as_f64());
            v.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    }
}

impl Default for Rational {
    fn default() -> Self {
        Self::FPS_25
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// Format an absolute frame count as SMPTE timecode (`HH:MM:SS:FF` or `HH;MM;SS;FF`).
pub fn frames_to_timecode(total_frames: i64, fps: Rational) -> String {
    let total_frames = total_frames.max(0);
    let display_fps = fps.timebase().max(1);
    let is_ntsc = fps.is_ntsc();
    let sep = if is_ntsc { ';' } else { ':' };
    let is_dropframe = is_ntsc && matches!(display_fps, 30 | 60);

    if is_dropframe {
        let drop_frames = if display_fps == 60 { 4 } else { 2 };
        let frames_per_minute = display_fps * 60;
        let frames_per_10min = 10 * frames_per_minute - 9 * drop_frames;

        let blocks_10 = total_frames / frames_per_10min;
        let mut remaining = total_frames % frames_per_10min;
        let mut total_minutes = blocks_10 * 10;

        for _ in 0..9 {
            let needed = frames_per_minute - drop_frames;
            if remaining >= needed {
                remaining -= needed;
                total_minutes += 1;
            } else {
                break;
            }
        }
        if remaining >= frames_per_minute && total_minutes % 10 == 9 {
            remaining -= frames_per_minute;
            total_minutes += 1;
        }
        let hours = total_minutes / 60;
        let minutes = total_minutes % 60;
        let seconds = remaining / display_fps;
        let frames = remaining % display_fps;
        return format!("{hours:02}{sep}{minutes:02}{sep}{seconds:02}{sep}{frames:02}");
    }

    let hours = total_frames / (3600 * display_fps);
    let rem = total_frames % (3600 * display_fps);
    let minutes = rem / (60 * display_fps);
    let rem = rem % (60 * display_fps);
    let seconds = rem / display_fps;
    let frames = rem % display_fps;
    format!("{hours:02}{sep}{minutes:02}{sep}{seconds:02}{sep}{frames:02}")
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adobe_ticks_roundtrip_ntsc_and_pal() {
        let fps25 = Rational::from_adobe_frame_ticks(10_160_640_000);
        assert_eq!(fps25, Rational::FPS_25);
        assert_eq!(fps25.adobe_ticks_to_frames(ADOBE_TICKS_PER_SECOND * 10), 250);
        assert_eq!(fps25.frames_to_adobe_ticks(250), ADOBE_TICKS_PER_SECOND * 10);

        let fps23976 = Rational::from_adobe_frame_ticks(10_594_584_000);
        assert_eq!(fps23976, Rational::FPS_23_976);
        assert!(fps23976.is_ntsc());
        assert_eq!(fps23976.adobe_ticks_to_frames(10_594_584_000 * 24), 24);
    }

    #[test]
    fn timecode_formatting() {
        assert_eq!(frames_to_timecode(90_000, Rational::FPS_25), "01:00:00:00");
        assert_eq!(frames_to_timecode(25 * 62 + 13, Rational::FPS_25), "00:01:02:13");
        assert_eq!(frames_to_timecode(0, Rational::FPS_29_97), "00;00;00;00");
    }

    #[test]
    fn fcpxml_rational_strings() {
        assert_eq!(Rational::FPS_24.frames_to_fcpxml(0), "0s");
        assert_eq!(Rational::FPS_24.frames_to_fcpxml(24), "1s");
        assert_eq!(Rational::FPS_23_976.frames_to_fcpxml(10), "1001/2400s");
    }
}
