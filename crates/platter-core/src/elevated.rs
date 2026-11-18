//! The commands that run as the administrator on Linux, as a closed list.
//!
//! Both sides read this one list. The cleanup rules build their tools from it, so what Platter asks for
//! is always on it. The root helper (`platter-elevate`, started through `pkexec`) takes only an id from it
//! and runs the command it names, so what the helper can be told to run is never more than these seven
//! fixed commands.

/// One command of the closed list. Its program and arguments are fixed: nothing is ever appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevatedTool {
    /// The pacman cache, keeping 1 version of each package.
    Paccache1,
    /// The pacman cache, keeping 2 versions of each package.
    Paccache2,
    /// The pacman cache, keeping 3 versions of each package.
    Paccache3,
    AptClean,
    DnfClean,
    /// The journal, down to the last two weeks.
    JournalVacuum,
    /// Everything inside the systemd core dump folder. Stays on its filesystem and follows no link.
    Coredumps,
}

impl ElevatedTool {
    /// Every command on the list.
    pub const ALL: [Self; 7] = [
        Self::Paccache1,
        Self::Paccache2,
        Self::Paccache3,
        Self::AptClean,
        Self::DnfClean,
        Self::JournalVacuum,
        Self::Coredumps,
    ];

    /// The name the helper is started with. It never changes once released: an older helper
    /// does not know a new one.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Paccache1 => "paccache-keep-1",
            Self::Paccache2 => "paccache-keep-2",
            Self::Paccache3 => "paccache-keep-3",
            Self::AptClean => "apt-clean",
            Self::DnfClean => "dnf-clean",
            Self::JournalVacuum => "journal-vacuum",
            Self::Coredumps => "coredumps",
        }
    }

    /// The program's file name. It is found in the system's own program folders only.
    pub const fn program(self) -> &'static str {
        match self {
            Self::Paccache1 | Self::Paccache2 | Self::Paccache3 => "paccache",
            Self::AptClean => "apt-get",
            Self::DnfClean => "dnf",
            Self::JournalVacuum => "journalctl",
            Self::Coredumps => "find",
        }
    }

    pub const fn args(self) -> &'static [&'static str] {
        match self {
            Self::Paccache1 => &["-rk1"],
            Self::Paccache2 => &["-rk2"],
            Self::Paccache3 => &["-rk3"],
            Self::AptClean => &["clean"],
            Self::DnfClean => &["clean", "packages"],
            Self::JournalVacuum => &["--vacuum-time=2weeks"],
            Self::Coredumps => &["/var/lib/systemd/coredump", "-xdev", "-mindepth", "1", "-delete"],
        }
    }

    /// The command with this exact id. No trimming, no case folding, no prefix.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.id() == id)
    }

    /// The command with exactly this program and these arguments.
    pub fn find(program: &str, args: &[&str]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tool| tool.program() == program && tool.args() == args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_found_by_its_id_and_by_its_program_and_arguments() {
        for tool in ElevatedTool::ALL {
            assert_eq!(ElevatedTool::from_id(tool.id()), Some(tool));
            assert_eq!(ElevatedTool::find(tool.program(), tool.args()), Some(tool));
        }
    }

    #[test]
    fn ids_are_unique() {
        for tool in ElevatedTool::ALL {
            let same = ElevatedTool::ALL
                .iter()
                .filter(|other| other.id() == tool.id())
                .count();

            assert_eq!(same, 1, "{}", tool.id());
        }
    }

    #[test]
    fn the_ids_are_the_released_ones() {
        let ids: Vec<&str> = ElevatedTool::ALL.iter().map(|tool| tool.id()).collect();

        assert_eq!(
            ids,
            [
                "paccache-keep-1",
                "paccache-keep-2",
                "paccache-keep-3",
                "apt-clean",
                "dnf-clean",
                "journal-vacuum",
                "coredumps",
            ]
        );
    }

    #[test]
    fn an_id_must_match_exactly() {
        for id in [
            "",
            " apt-clean",
            "apt-clean ",
            "apt-clean\n",
            "apt-",
            "apt-clean-all",
            "Apt-Clean",
            "APT-CLEAN",
            "../apt-clean",
            "paccache-keep-4",
            "journal-vacuum --rotate",
        ] {
            assert_eq!(ElevatedTool::from_id(id), None, "{id:?}");
        }
    }

    #[test]
    fn a_command_must_match_exactly() {
        assert_eq!(
            ElevatedTool::find("apt-get", &["clean"]),
            Some(ElevatedTool::AptClean)
        );
        assert_eq!(ElevatedTool::find("apt-get", &[]), None);
        assert_eq!(ElevatedTool::find("apt-get", &["clean", "-y"]), None);
        assert_eq!(ElevatedTool::find("apt-get", &["autoremove"]), None);
        assert_eq!(ElevatedTool::find("paccache", &["-rk4"]), None);
        assert_eq!(ElevatedTool::find("docker", &["system", "prune"]), None);
        assert_eq!(ElevatedTool::find("find", &["/", "-delete"]), None);
    }
}
