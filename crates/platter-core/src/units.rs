//! How a size is written: the unit family and `format_size`, shared by the rule reasons and the advisor's answers.

/// The unit family sizes are written in: `GB` counts in thousands, `GiB` in 1024s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Units {
    Decimal,
    Binary,
}

const DECIMAL_UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
const BINARY_UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

/// `1.41 TB`, `492 GB`, `3.4 GB`, `512 B`: a port of `formatSize` of the web view
/// (`apps/desktop/src/lib/format.ts`), so a size reads the same in the rows and in the answer. Two
/// decimals from TB up, one below 10, none from 10 on.
pub fn format_size(bytes: u64, units: Units) -> String {
    let (base, names) = match units {
        Units::Decimal => (1000.0, DECIMAL_UNITS),
        Units::Binary => (1024.0, BINARY_UNITS),
    };

    let mut index = 0;
    let mut value = bytes as f64;
    while value >= base && index < names.len() - 1 {
        value /= base;
        index += 1;
    }

    if index == 0 {
        return format!("{} B", bytes);
    }

    let mut decimals = if value < 10.0 { 1 } else { 0 };
    if index >= 4 {
        decimals = 2;
    }

    // 999.7 GB must read "1.00 TB", not "1000 GB".
    let rounded: f64 = to_fixed(value, decimals).parse().unwrap_or(value);
    if rounded >= base && index < names.len() - 1 {
        value /= base;
        index += 1;
        decimals = if index >= 4 { 2 } else { 1 };
    }

    format!("{} {}", to_fixed(value, decimals), names[index])
}

/// JavaScript's `Number.prototype.toFixed`: an exact tie rounds up (Rust's formatting rounds it to even).
fn to_fixed(value: f64, decimals: usize) -> String {
    let scale = 10f64.powi(decimals as i32);
    let scaled = value * scale;

    if scaled.fract() == 0.5 && scaled / scale == value {
        return format!("{:.*}", decimals, (scaled.floor() + 1.0) / scale);
    }

    format!("{:.*}", decimals, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_written_in_the_family_asked_for() {
        assert_eq!(format_size(1_000_000_000, Units::Decimal), "1.0 GB");
        assert_eq!(format_size(1_073_741_824, Units::Binary), "1.0 GiB");
        assert_eq!(format_size(512, Units::Binary), "512 B");
        assert_eq!(format_size(1_610_612_736, Units::Binary), "1.5 GiB");
    }

    #[test]
    fn a_size_that_rounds_up_to_the_next_unit_reads_in_it() {
        assert_eq!(format_size(999_700_000, Units::Decimal), "1.0 GB");
        assert_eq!(format_size(999_700_000_000, Units::Decimal), "1.00 TB");
    }

    #[test]
    fn an_exact_tie_rounds_up_as_in_the_web_view() {
        assert_eq!(format_size(2_250, Units::Decimal), "2.3 KB");
    }
}
