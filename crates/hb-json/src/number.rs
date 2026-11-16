//! JSON numbers as Gson (and therefore Minecraft) sees them.
//!
//! Two representations matter to a porting tool:
//!
//! * **Text.** Whatever was read is what is written, so `4.0` stays `4.0`.
//!   `serde_json`'s habit of turning `4.0` into `4.0` and `1e2` into `100.0`
//!   is harmless; turning `4.0` into `4` is not, because Gson then hands the
//!   game an `Integer` where a `Float` is required and the block fails to
//!   load with a class-cast error.
//! * **Java type.** A converter sometimes has to *emit* a float (`4.0`)
//!   where a legacy file had an int, or the reverse. [`Number::float`] and
//!   [`Number::int`] make that explicit.

use std::fmt;

/// A JSON number: the original text plus a parsed view.
#[derive(Debug, Clone, PartialEq)]
pub struct Number {
    text: String,
    /// `true` when the text has a fraction or exponent, i.e. Gson yields a
    /// floating-point value for it.
    floating: bool,
}

impl Number {
    /// Parses number text that the JSON grammar already validated.
    pub fn parse_unchecked(text: &str) -> Self {
        let floating = text.contains(['.', 'e', 'E']);
        Self {
            text: text.to_owned(),
            floating,
        }
    }

    /// Parses a standalone number, checking the JSON grammar.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() {
            return Err("empty number".into());
        }
        let bytes = text.as_bytes();
        let mut at = 0;
        if bytes[at] == b'-' {
            at += 1;
        }
        let int_start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at == int_start {
            return Err(format!("{text:?} has no digits"));
        }
        if bytes[int_start] == b'0' && at - int_start > 1 {
            return Err(format!("{text:?} has a leading zero"));
        }
        if at < bytes.len() && bytes[at] == b'.' {
            at += 1;
            let frac_start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            if at == frac_start {
                return Err(format!("{text:?} has no digits after the point"));
            }
        }
        if at < bytes.len() && matches!(bytes[at], b'e' | b'E') {
            at += 1;
            if at < bytes.len() && matches!(bytes[at], b'+' | b'-') {
                at += 1;
            }
            let exp_start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            if at == exp_start {
                return Err(format!("{text:?} has an empty exponent"));
            }
        }
        if at != bytes.len() {
            return Err(format!("{text:?} has trailing characters"));
        }
        Ok(Self::parse_unchecked(text))
    }

    /// An integer literal. `i64::MIN` and friends print exactly.
    pub fn from_i64(n: i64) -> Self {
        Self {
            text: n.to_string(),
            floating: false,
        }
    }

    /// A floating-point literal that always looks like one (`4` becomes
    /// `4.0`), the form Gson needs for a float field.
    pub fn from_f64(f: f64) -> Self {
        if !f.is_finite() {
            // JSON has no NaN or Infinity. Clamping would be a lie; the
            // converter must not have got here. Keep it visible instead.
            return Self {
                text: "0.0".into(),
                floating: true,
            };
        }
        let mut text = format_java_double(f);
        if !text.contains(['.', 'e', 'E']) {
            text.push_str(".0");
        }
        Self {
            text,
            floating: true,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        if self.floating {
            return None;
        }
        self.text.parse().ok()
    }

    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().and_then(|n| i32::try_from(n).ok())
    }

    pub fn as_f64(&self) -> Option<f64> {
        self.text.parse().ok()
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|f| f as f32)
    }

    /// Whether Gson would produce a floating-point value from this text.
    pub fn is_floating(&self) -> bool {
        self.floating
    }

    /// The literal text, as read or as constructed.
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Formats a double the way Java's `Double.toString` does for the magnitudes
/// Minecraft data uses: plain decimal notation from 1e-3 up to 1e7, and
/// `d.dddEn` outside it.
///
/// This matters because a converted `"hardness": 4.0` should read `4.0`, not
/// `4.0000000000000001`, and `0.98` must not become `0.9800000190734863`.
pub fn format_java_double(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.to_owned();
    }
    let abs = f.abs();
    if (1e-3..1e7).contains(&abs) {
        // Rust's `{}` for f64 is the shortest round-tripping representation,
        // which matches Java's algorithm for these ranges.
        let mut s = format!("{f}");
        if !s.contains('.') {
            s.push_str(".0");
        }
        s
    } else {
        format!("{f:e}")
    }
}

/// Shortest text for a float that keeps it a float (`0.98f` -> `0.98`).
pub fn format_java_float(f: f32) -> String {
    format_java_double(f as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_number_text() {
        for ok in ["0", "-1", "4.0", "1e5", "1E+5", "-0.0", "1.5e-3"] {
            assert!(Number::parse(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-", "01", "1.", ".5", "1e", "1e+", "1x", "--1"] {
            assert!(Number::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn floats_always_look_floating() {
        assert_eq!(Number::from_f64(4.0).to_string(), "4.0");
        assert_eq!(Number::from_f64(0.98).to_string(), "0.98");
        assert_eq!(Number::from_f64(-1.0).to_string(), "-1.0");
        assert_eq!(Number::from_i64(4).to_string(), "4");
        assert!(Number::from_f64(4.0).is_floating());
        assert!(!Number::from_i64(4).is_floating());
    }

    #[test]
    fn java_double_matches_expected_strings() {
        assert_eq!(format_java_double(4.0), "4.0");
        assert_eq!(format_java_double(0.98), "0.98");
        assert_eq!(format_java_double(-0.0), "-0.0");
        assert_eq!(format_java_double(0.0), "0.0");
        assert_eq!(format_java_double(15.0), "15.0");
        assert_eq!(format_java_float(0.5), "0.5");
    }

    #[test]
    fn non_finite_never_emits_invalid_json() {
        assert_eq!(Number::from_f64(f64::NAN).to_string(), "0.0");
        assert!(Number::parse(&Number::from_f64(f64::INFINITY).to_string()).is_ok());
    }
}
