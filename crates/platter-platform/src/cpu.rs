//! How many physical cores the machine has. Threads on a model run best one per physical core: two
//! hardware threads on one core are slower than one, because the work waits on memory, not on the core.

/// Physical cores (not hardware threads), `None` where this cannot be told (no `/proc/cpuinfo`, or a
/// CPU that lists no core ids, as on some ARM machines).
#[cfg(target_os = "linux")]
pub fn physical_cores() -> Option<usize> {
    parse_physical_cores(&std::fs::read_to_string("/proc/cpuinfo").ok()?)
}

/// Physical cores (not hardware threads), from the system through `sysinfo`.
#[cfg(windows)]
pub fn physical_cores() -> Option<usize> {
    sysinfo::System::physical_core_count()
        .ok()
        .filter(|&cores| cores > 0)
}

/// Physical cores. Not known on this system.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn physical_cores() -> Option<usize> {
    None
}

/// Counts distinct (`physical id`, `core id`) pairs in the text of `/proc/cpuinfo`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_physical_cores(cpuinfo: &str) -> Option<usize> {
    use std::collections::HashSet;

    let mut cores = HashSet::new();
    for processor in cpuinfo.split("\n\n") {
        let field = |name: &str| {
            processor
                .lines()
                .find_map(|line| line.strip_prefix(name)?.trim_start().strip_prefix(':'))
                .map(str::trim)
        };
        let Some(core) = field("core id") else {
            continue;
        };
        cores.insert((field("physical id").unwrap_or("0").to_owned(), core.to_owned()));
    }

    (!cores.is_empty()).then_some(cores.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_hardware_threads_of_one_core_count_once() {
        let cpuinfo = "processor\t: 0\nphysical id\t: 0\ncore id\t\t: 0\n\n\
            processor\t: 1\nphysical id\t: 0\ncore id\t\t: 1\n\n\
            processor\t: 2\nphysical id\t: 0\ncore id\t\t: 0\n\n\
            processor\t: 3\nphysical id\t: 0\ncore id\t\t: 1\n";

        assert_eq!(parse_physical_cores(cpuinfo), Some(2));
    }

    #[test]
    fn the_same_core_id_on_two_sockets_is_two_cores() {
        let cpuinfo =
            "processor : 0\nphysical id : 0\ncore id : 0\n\nprocessor : 1\nphysical id : 1\ncore id : 0\n";

        assert_eq!(parse_physical_cores(cpuinfo), Some(2));
    }

    #[test]
    fn a_cpu_without_core_ids_is_unknown() {
        assert_eq!(
            parse_physical_cores("processor : 0\nBogoMIPS : 48\n\nprocessor : 1\n"),
            None
        );
        assert_eq!(parse_physical_cores(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn the_cores_of_this_machine_are_known() {
        let cores = physical_cores().expect("the system reports its cores");

        assert!(cores >= 1, "{cores}");
    }
}
