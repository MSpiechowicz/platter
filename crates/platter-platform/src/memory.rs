//! A scan limit that fits the memory the machine has.
//!
//! The core caps a scan by a budget of memory ([`platter_core::ScanConfig::max_memory_bytes`]) so that a
//! huge or hostile tree ends in a clear error instead of an out-of-memory kill. A fixed number is wrong
//! for both a small laptop and a workstation with a terabyte of RAM, so an app asks for one that matches
//! the memory that is free right now. The unit is bytes, not entries: what a scan costs depends on the
//! length of the names as much as on their number (see [`platter_core::budget`]).

use platter_core::DEFAULT_MAX_MEMORY_BYTES;

/// The memory budget of a scan: half of the memory that is available, so the other half stays for the
/// rest of the system and for the short-lived peak while the finished tree is assembled. There is no
/// floor: with little free memory the budget is small and only a small folder fits, and the scan's
/// `TooLarge` message says that is why. On Linux the memory comes from `MemAvailable` in `/proc/meminfo`.
/// Where it cannot be read, and on other systems, this is the core default.
pub fn suggested_memory_budget() -> u64 {
    available_memory_bytes().map_or(DEFAULT_MAX_MEMORY_BYTES, memory_budget_for)
}

/// The budget for a machine with `available_bytes` of free memory (see [`suggested_memory_budget`]).
pub fn memory_budget_for(available_bytes: u64) -> u64 {
    available_bytes / 2
}

/// Memory the system can give to a program now without swapping (`MemAvailable`). `None` where it
/// cannot be read.
#[cfg(target_os = "linux")]
pub fn available_memory_bytes() -> Option<u64> {
    parse_mem_available(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// Memory the system can give to a program now without swapping. `None` on this system.
#[cfg(not(target_os = "linux"))]
pub fn available_memory_bytes() -> Option<u64> {
    None
}

/// All memory of the machine (`MemTotal`). `None` where it cannot be read.
#[cfg(target_os = "linux")]
pub fn total_memory_bytes() -> Option<u64> {
    parse_mem_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// All memory of the machine. `None` on this system.
#[cfg(not(target_os = "linux"))]
pub fn total_memory_bytes() -> Option<u64> {
    None
}

/// Memory this process holds in RAM right now (resident set, from `/proc/self/statm`). `None` where it
/// cannot be read.
#[cfg(target_os = "linux")]
pub fn resident_bytes() -> Option<u64> {
    let page = rustix::param::page_size() as u64;

    parse_resident(&std::fs::read_to_string("/proc/self/statm").ok()?, page)
}

/// Memory this process holds in RAM right now. `None` on this system.
#[cfg(not(target_os = "linux"))]
pub fn resident_bytes() -> Option<u64> {
    None
}

/// The `MemAvailable` line of `/proc/meminfo` (`MemAvailable:   12345678 kB`) in bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_mem_available(meminfo: &str) -> Option<u64> {
    parse_meminfo_line(meminfo, "MemAvailable:")
}

/// The `MemTotal` line of `/proc/meminfo` in bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_mem_total(meminfo: &str) -> Option<u64> {
    parse_meminfo_line(meminfo, "MemTotal:")
}

/// The second field of `/proc/self/statm` (resident pages) in bytes, for pages of `page_bytes`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_resident(statm: &str, page_bytes: u64) -> Option<u64> {
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;

    pages.checked_mul(page_bytes)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_meminfo_line(meminfo: &str, key: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with(key))?;

    let mut fields = line.split_whitespace().skip(1);
    let amount: u64 = fields.next()?.parse().ok()?;
    let unit = match fields.next() {
        Some("kB") | None => 1024,
        Some(_) => return None,
    };
    amount.checked_mul(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;

    #[test]
    fn the_budget_is_half_of_the_available_memory() {
        assert_eq!(memory_budget_for(32 * GIB), 16 * GIB);
        assert_eq!(memory_budget_for(4096 * GIB), 2048 * GIB, "no ceiling either");
    }

    #[test]
    fn there_is_no_floor_that_exceeds_what_is_available() {
        assert_eq!(memory_budget_for(0), 0);
        assert_eq!(memory_budget_for(200 * MIB), 100 * MIB);
        assert_eq!(memory_budget_for(1), 0);

        for available in [0, 1, 4096, 100 * MIB, 256 * MIB, 3 * GIB] {
            assert!(memory_budget_for(available) <= available / 2);
        }
        assert_eq!(memory_budget_for(u64::MAX), u64::MAX / 2);
    }

    #[test]
    fn mem_available_is_read_in_kibibytes() {
        let meminfo = "MemTotal:       32768000 kB\nMemFree:         1000000 kB\nMemAvailable:   16384000 kB\nBuffers: 1 kB\n";

        assert_eq!(parse_mem_available(meminfo), Some(16_384_000 * 1024));
    }

    #[test]
    fn a_meminfo_without_mem_available_or_with_nonsense_gives_nothing() {
        assert_eq!(parse_mem_available("MemTotal: 1 kB\n"), None);
        assert_eq!(parse_mem_available("MemAvailable: lots kB\n"), None);
        assert_eq!(parse_mem_available("MemAvailable: 5 MB\n"), None);
        assert_eq!(parse_mem_available(""), None);
    }

    #[test]
    fn the_resident_size_is_the_second_field_of_statm_in_pages() {
        assert_eq!(
            parse_resident("5000 1200 300 10 0 900 0\n", 4096),
            Some(1200 * 4096)
        );
        assert_eq!(parse_resident("5000", 4096), None);
        assert_eq!(parse_resident("5000 many", 4096), None);
        assert_eq!(
            parse_resident("1 18446744073709551615", 4096),
            None,
            "no overflow"
        );
    }

    #[test]
    fn the_total_is_read_apart_from_what_is_available() {
        let meminfo = "MemTotal:       32768000 kB\nMemAvailable:   16384000 kB\n";

        assert_eq!(parse_mem_total(meminfo), Some(32_768_000 * 1024));
        assert_eq!(parse_mem_total("MemAvailable: 1 kB\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_and_machine_report_memory() {
        let resident = resident_bytes().unwrap();
        let total = total_memory_bytes().unwrap();

        assert!(resident > 0 && resident < total);
        assert!(available_memory_bytes().unwrap() <= total);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_machine_gets_a_budget_below_what_it_has() {
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap();
        let available = parse_mem_available(&meminfo).unwrap();

        // Memory moves between the two reads; the budget is half of something close to `available`.
        assert!(suggested_memory_budget() <= available);
        assert!(suggested_memory_budget() > 0);
    }
}
