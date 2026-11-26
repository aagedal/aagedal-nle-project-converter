use nle_core::Rational;
use std::ops::{Add, Sub};

/// Exact rational time in seconds (`num / den` seconds) used throughout FCPXML
/// (e.g. `"0s"`, `"10s"`, `"1001/24000s"`, `"3600s"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FcpTime {
    pub num: i128,
    pub den: i128,
}

impl FcpTime {
    pub const ZERO: Self = Self { num: 0, den: 1 };

    pub fn new(num: i128, den: i128) -> Self {
        if den == 0 || num == 0 {
            return Self::ZERO;
        }
        let (n, d) = if den < 0 { (-num, -den) } else { (num, den) };
        let g = gcd_u128(n.unsigned_abs(), d.unsigned_abs()) as i128;
        Self {
            num: n / g,
            den: d / g,
        }
    }

    /// Parse an FCPXML rational time string such as `"0s"`, `"3600s"`, `"1001/24000s"`, `"-100/2500s"`, or `"1.5s"`.
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().trim_end_matches('s').trim();
        if s.is_empty() {
            return None;
        }
        if let Some((n_str, d_str)) = s.split_once('/') {
            let n = n_str.trim().parse::<i128>().ok()?;
            let d = d_str.trim().parse::<i128>().ok()?;
            if d == 0 {
                return None;
            }
            return Some(Self::new(n, d));
        }
        if let Some((whole, frac)) = s.split_once('.') {
            let neg = whole.starts_with('-');
            let whole_abs = whole.trim_start_matches(['-', '+']);
            let w: i128 = if whole_abs.is_empty() {
                0
            } else {
                whole_abs.parse().ok()?
            };
            let frac_trimmed = frac.trim();
            if frac_trimmed.is_empty() {
                return Some(Self::new(if neg { -w } else { w }, 1));
            }
            let scale = 10i128.checked_pow(frac_trimmed.len().min(12) as u32)?;
            let frac_slice = &frac_trimmed[..frac_trimmed.len().min(12)];
            let f: i128 = frac_slice.parse().ok()?;
            let total = w.checked_mul(scale)?.checked_add(f)?;
            return Some(Self::new(if neg { -total } else { total }, scale));
        }
        let n = s.parse::<i128>().ok()?;
        Some(Self::new(n, 1))
    }

    /// Convert a frame count at `fps` into exact `FcpTime`.
    pub fn from_frames(frames: i64, fps: Rational) -> Self {
        Self::new((frames as i128) * (fps.den as i128), fps.num as i128)
    }

    /// Convert this rational time into the nearest integer frame count at `fps`.
    pub fn to_frames(self, fps: Rational) -> i64 {
        if self.num == 0 {
            return 0;
        }
        let num = self.num * (fps.num as i128);
        let den = self.den * (fps.den as i128);
        if den == 0 {
            return 0;
        }
        if num >= 0 {
            ((num + den / 2) / den) as i64
        } else {
            -(((-num) + den / 2) / den) as i64
        }
    }

    pub fn as_f64(self) -> f64 {
        if self.den == 0 {
            0.0
        } else {
            self.num as f64 / self.den as f64
        }
    }

    /// Format into canonical reduced FCPXML time syntax (`"0s"`, `"5s"`, `"1001/24000s"`).
    pub fn to_fcpxml_string(self) -> String {
        if self.num == 0 {
            "0s".to_string()
        } else if self.den == 1 {
            format!("{}s", self.num)
        } else {
            format!("{}/{}s", self.num, self.den)
        }
    }
}

impl Default for FcpTime {
    fn default() -> Self {
        Self::ZERO
    }
}

impl Add for FcpTime {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        if self.num == 0 {
            return rhs;
        }
        if rhs.num == 0 {
            return self;
        }
        let g = gcd_u128(self.den.unsigned_abs(), rhs.den.unsigned_abs()) as i128;
        let l_den = self.den / g;
        let r_den = rhs.den / g;
        let num = self.num * r_den + rhs.num * l_den;
        let den = l_den * rhs.den;
        Self::new(num, den)
    }
}

impl Sub for FcpTime {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        self + Self {
            num: -rhs.num,
            den: rhs.den,
        }
    }
}

/// Convert an FCPXML `frameDuration` string (e.g. `"1001/24000s"`, `"100/2500s"`, `"1/30s"`)
/// into an exact `Rational` FPS.
pub fn parse_frame_duration_to_fps(raw: &str) -> Option<Rational> {
    let t = FcpTime::parse(raw)?;
    if t.num <= 0 || t.den <= 0 {
        return None;
    }
    let exact = Rational::new(t.den as i64, t.num as i64);
    // Keep exact rational for standard rates (24000/1001, 25/1, 30000/1001, 24/1, 50/1, 60000/1001, 60/1, etc.)
    if exact.num > 0 && exact.den > 0 {
        Some(exact)
    } else {
        Some(Rational::from_fps_approx(t.den as f64 / t.num as f64))
    }
}

/// Decode a `file://` URL from FCPXML `<media-rep src="...">` into a local filesystem path.
pub fn file_url_to_path(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let rest = if let Some(s) = trimmed.strip_prefix("file://localhost") {
        s
    } else if let Some(s) = trimmed.strip_prefix("file://") {
        s
    } else {
        return trimmed.to_string();
    };

    let decoded = percent_decode(rest);
    // Windows drive letter path: `/C:/Users/...` -> `C:/Users/...`
    let bytes = decoded.as_bytes();
    if bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

/// Encode a local filesystem path into an RFC 3986 `file://` URL for FCPXML `<media-rep src="...">`.
pub fn path_to_file_url(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("file://") {
        return trimmed.to_string();
    }
    let norm = trimmed.replace('\\', "/");
    let encoded = percent_encode_path(&norm);
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 16);
    for (idx, &b) in path.as_bytes().iter().enumerate() {
        let is_drive_colon = b == b':' && idx == 1 && path.as_bytes()[0].is_ascii_alphabetic();
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'=' | b'@')
            || is_drive_colon
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
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
    fn parses_and_converts_fcpxml_times() {
        assert_eq!(FcpTime::parse("0s"), Some(FcpTime::ZERO));
        assert_eq!(FcpTime::parse("3600s"), Some(FcpTime::new(3600, 1)));
        assert_eq!(
            FcpTime::parse("1001/24000s"),
            Some(FcpTime::new(1001, 24000))
        );
        assert_eq!(FcpTime::parse("1.5s"), Some(FcpTime::new(3, 2)));

        let fps23976 = parse_frame_duration_to_fps("1001/24000s").unwrap();
        assert_eq!(fps23976, Rational::FPS_23_976);
        assert_eq!(FcpTime::parse("1001/2400s").unwrap().to_frames(fps23976), 10);

        let fps25 = parse_frame_duration_to_fps("100/2500s").unwrap();
        assert_eq!(fps25, Rational::FPS_25);
        assert_eq!(FcpTime::parse("3600s").unwrap().to_frames(fps25), 90_000);
        assert_eq!(FcpTime::from_frames(25, fps25).to_fcpxml_string(), "1s");
    }

    #[test]
    fn file_url_unicode_roundtrip() {
        let orig = "/Volumes/Media/Søndag Økt #1/Klipp Å.mov";
        let url = path_to_file_url(orig);
        assert!(url.starts_with("file:///Volumes/Media/"));
        assert!(!url.contains(' '));
        assert!(!url.contains('#'));
        assert_eq!(file_url_to_path(&url), orig);
        assert_eq!(
            file_url_to_path("file://localhost/Users/editor/My%20Clip.mov"),
            "/Users/editor/My Clip.mov"
        );
    }
}
