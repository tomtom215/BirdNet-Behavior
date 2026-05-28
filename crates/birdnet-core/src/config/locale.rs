//! Locale-tolerant numeric parsing for user-facing decimal inputs.
//!
//! Operators in countries that use a comma as the decimal separator
//! (most of continental Europe, Latin America, large parts of Africa and
//! the Middle East) routinely type `42,3601` for latitude and `0,75` for
//! confidence thresholds. The std library's [`f64::from_str`] only
//! accepts the period form, so without normalisation a perfectly valid
//! EU-formatted number silently fails parsing at runtime — or, worse,
//! survives the round-trip through `<input type="number">` where
//! browsers in EU locale strip the comma altogether, leaving `075`
//! (i.e. seventy-five) where the user typed three-quarters.
//!
//! This module is the canonical fix. Apply [`normalize_decimal`] at the
//! boundary where user input enters the system — typically the
//! settings POST handler in `birdnet-web` — so every downstream value
//! is the canonical period-form string. [`parse_decimal`] is the matching
//! parser that accepts both forms (also handy at config-validation time
//! when reading values that haven't yet been normalised).
//!
//! ## Accepted shapes
//!
//! - Trailing / leading whitespace.
//! - `42.3601` — canonical period form. Returned unchanged.
//! - `42,3601` — comma form. Replaced with a period.
//! - `42` — bare integer. Returned unchanged.
//! - `-71.0589`, `-71,0589` — leading minus sign preserved.
//! - `+42.36` — leading plus sign preserved.
//!
//! ## Deliberately rejected shapes
//!
//! - `1,234.56` and `1.234,56` — thousands-grouped numbers. The
//!   ambiguity (`,` as group vs. `,` as decimal) cannot be resolved
//!   without a locale hint, and bird-station numeric inputs (lat / lon
//!   / confidence / gain) never exceed 1000 in magnitude, so any value
//!   carrying two separator characters is treated as malformed.
//! - Strings with non-numeric characters beyond a single leading sign.
//!
//! The functions are infallible normalisers — [`normalize_decimal`]
//! returns the input unchanged if normalisation is unsafe, so a
//! later `parse::<f64>()` will surface the underlying error rather than
//! silently mangling the value.

/// Return `input` with a comma decimal separator replaced by a period,
/// when (and only when) doing so is unambiguous.
///
/// Rules:
/// 1. Trim leading / trailing whitespace.
/// 2. Drop an optional leading `+` or `-` sign before counting separators.
/// 3. If the body contains exactly one `,` and zero `.`, replace the
///    comma with a period.
/// 4. Otherwise return the trimmed input unchanged so the caller's
///    subsequent `parse::<f64>()` can surface the actual error.
#[must_use]
pub fn normalize_decimal(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let (sign, body) = match trimmed.as_bytes().first() {
        Some(b'+' | b'-') => (&trimmed[..1], &trimmed[1..]),
        _ => ("", trimmed),
    };

    let commas = body.bytes().filter(|&b| b == b',').count();
    let periods = body.bytes().filter(|&b| b == b'.').count();

    if commas == 1 && periods == 0 {
        let replaced = body.replace(',', ".");
        return format!("{sign}{replaced}");
    }
    trimmed.to_string()
}

/// Parse a decimal number from `input`, accepting either the period or
/// comma form as a decimal separator.
///
/// Equivalent to `normalize_decimal(input).parse::<f64>()` but spells the
/// intent explicitly at call sites that just want a number.
///
/// # Errors
///
/// Returns [`std::num::ParseFloatError`] when the (normalised) string is
/// not a valid `f64`.
pub fn parse_decimal(input: &str) -> Result<f64, std::num::ParseFloatError> {
    normalize_decimal(input).parse::<f64>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_period_form_passes_through_unchanged() {
        assert_eq!(normalize_decimal("42.3601"), "42.3601");
        assert_eq!(normalize_decimal("-71.0589"), "-71.0589");
        assert_eq!(normalize_decimal("+0.75"), "+0.75");
        assert_eq!(normalize_decimal("0"), "0");
    }

    #[test]
    fn comma_form_is_normalised_to_period() {
        assert_eq!(normalize_decimal("42,3601"), "42.3601");
        assert_eq!(normalize_decimal("-71,0589"), "-71.0589");
        assert_eq!(normalize_decimal("0,75"), "0.75");
        assert_eq!(normalize_decimal("+0,03"), "+0.03");
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(normalize_decimal("   42,3601\n"), "42.3601");
        assert_eq!(normalize_decimal(" 42.3601 "), "42.3601");
    }

    #[test]
    fn empty_input_normalises_to_empty() {
        assert_eq!(normalize_decimal(""), "");
        assert_eq!(normalize_decimal("   "), "");
    }

    #[test]
    fn ambiguous_thousands_grouping_passes_through_unchanged() {
        // Two separators — we can't tell which is the decimal.
        assert_eq!(normalize_decimal("1,234.56"), "1,234.56");
        assert_eq!(normalize_decimal("1.234,56"), "1.234,56");
        assert_eq!(normalize_decimal("1,000,000"), "1,000,000");
    }

    #[test]
    fn integer_form_unchanged() {
        assert_eq!(normalize_decimal("42"), "42");
        assert_eq!(normalize_decimal("-71"), "-71");
        assert_eq!(normalize_decimal("0"), "0");
    }

    #[test]
    fn parse_decimal_accepts_both_forms() {
        assert!((parse_decimal("42.3601").unwrap() - 42.3601).abs() < 1e-9);
        assert!((parse_decimal("42,3601").unwrap() - 42.3601).abs() < 1e-9);
        assert!((parse_decimal("-71,0589").unwrap() - -71.0589).abs() < 1e-9);
        assert!((parse_decimal("0,75").unwrap() - 0.75).abs() < 1e-9);
        assert!((parse_decimal("+0,03").unwrap() - 0.03).abs() < 1e-9);
    }

    #[test]
    fn parse_decimal_rejects_garbage() {
        assert!(parse_decimal("not a number").is_err());
        assert!(parse_decimal("1,234.56").is_err());
        // Empty after normalisation → parse fails.
        assert!(parse_decimal("").is_err());
        assert!(parse_decimal("   ").is_err());
    }

    #[test]
    fn parse_decimal_handles_scientific_notation() {
        // Scientific notation lives only in the period form (no comma
        // form exists for it); we don't normalise these so `f64::from_str`
        // handles them natively.
        assert!((parse_decimal("1e3").unwrap() - 1000.0).abs() < 1e-9);
        assert!((parse_decimal("-1.5e-2").unwrap() - -0.015).abs() < 1e-9);
    }

    #[test]
    fn special_floats_pass_through() {
        // f64::from_str accepts these — normalisation must not break them.
        assert!(parse_decimal("inf").unwrap().is_infinite());
        assert!(parse_decimal("nan").unwrap().is_nan());
    }

    #[test]
    fn negative_zero_preserved() {
        let v = parse_decimal("-0").unwrap();
        assert!(v.abs() < f64::EPSILON);
        assert!(v.is_sign_negative());
    }
}
