//! Places cleanup never touches, whatever a rule says.

use platter_core::{EntryProbe, Environment, UserDir};

use crate::path::{is_within, owned};

/// System folders that may be neither a target nor have a target above them.
const SYSTEM_FOLDERS: &[&str] = &[
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib32",
    "/lib64",
    "/mnt",
    "/media",
    "/opt",
    "/proc",
    "/root",
    "/run",
    "/sbin",
    "/srv",
    "/sys",
    "/tmp",
    "/usr",
    "/var",
    "/var/cache",
    "/var/lib",
    "/var/log",
];

/// System folders nothing inside of may be a target.
const SYSTEM_INSIDE: &[&str] = &[
    "/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/opt", "/proc", "/root", "/run", "/sbin",
    "/srv", "/sys", "/usr",
];

/// Folders of the home directory nothing inside of may be a target: keys, passwords, settings and
/// browser and mail profiles.
const HOME_INSIDE: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".password-store",
    ".pki",
    ".config",
    ".local/share/keyrings",
    ".mozilla",
    ".thunderbird",
    ".local/share/kwalletd",
    ".var",
    "snap",
    ".aws",
    ".kube",
    ".netrc",
    ".git-credentials",
];

/// The protected places of one machine, as components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    /// Neither these nor anything above them may be a target.
    not_or_above: Vec<Vec<Vec<u8>>>,
    /// Nothing at or below these may be a target.
    not_inside: Vec<Vec<Vec<u8>>>,
    /// The same places by identity (device, inode), so another path to them (a bind mount of the home,
    /// a btrfs top level mounted elsewhere) is recognised too. `inside`: nothing at or below it; else only
    /// the place itself.
    identities: Vec<Identity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    place: String,
    inside: bool,
}

impl Protected {
    /// The protected places for `env`. `None` without a known home directory: then personal folders
    /// cannot be told apart, and cleanup refuses everything.
    pub fn for_environment(env: &Environment) -> Option<Self> {
        let home = env.home.as_ref()?;
        let home_bytes = home.as_os_str().as_encoded_bytes();
        let home_parts = owned(home_bytes)?;
        if home_parts.is_empty() {
            return None;
        }

        let mut not_or_above: Vec<Vec<Vec<u8>>> = vec![Vec::new(), home_parts.clone()];
        let mut not_inside: Vec<Vec<Vec<u8>>> = Vec::new();

        for folder in SYSTEM_FOLDERS {
            not_or_above.extend(owned(folder.as_bytes()));
        }
        for folder in SYSTEM_INSIDE {
            not_inside.extend(owned(folder.as_bytes()));
        }
        for folder in HOME_INSIDE {
            let mut parts = home_parts.clone();
            parts.extend(folder.split('/').map(|part| part.as_bytes().to_vec()));
            not_inside.push(parts);
        }
        for dir in UserDir::ALL {
            if let Some(path) = env.user_dir(dir)
                && let Some(parts) = owned(path.as_os_str().as_encoded_bytes())
            {
                not_or_above.push(parts.clone());
                not_inside.push(parts);
            }
        }

        // The folders above the home and the personal folders, so that they are known by identity too
        // (a second path to `/data` must not reach the `/data/users/u` above a home).
        let above: Vec<Vec<Vec<u8>>> = not_or_above
            .iter()
            .flat_map(|place| (1..place.len()).map(|length| place[..length].to_vec()))
            .collect();
        not_or_above.extend(above);
        not_or_above.sort();
        not_or_above.dedup();

        Some(Self {
            not_or_above,
            not_inside,
            identities: Vec::new(),
        })
    }

    /// Looks every protected place up on the disk, so it is also recognised by identity. A place that
    /// does not exist is skipped.
    ///
    /// A place reached through a link (`~/.config -> dotfiles/config`) is protected where it really is:
    /// its resolved path joins the lists, and the identity recorded is that of the folder, not the link.
    pub(crate) fn learn_identities(&mut self, probe: &impl EntryProbe) {
        let places: Vec<(Vec<Vec<u8>>, bool)> = self
            .not_inside
            .iter()
            .map(|place| (place.clone(), true))
            .chain(self.not_or_above.iter().map(|place| (place.clone(), false)))
            .collect();

        let mut identities = Vec::new();
        for (place, inside) in places {
            if place.is_empty() {
                continue;
            }
            let path = join_parts(&place);
            let resolved = probe
                .resolve(&path)
                .ok()
                .filter(|resolved| *resolved != path)
                .and_then(|resolved| owned(&resolved).filter(|parts| !parts.is_empty()));

            if let Some(real) = &resolved {
                if inside {
                    self.not_inside.push(real.clone());
                } else {
                    self.not_or_above.push(real.clone());
                }
            }

            let looked_at = resolved
                .as_ref()
                .map_or_else(|| path.clone(), |real| join_parts(real));
            if let Some(entry) = probe
                .probe(&looked_at)
                .ok()
                .and_then(|found| found.last().copied())
            {
                identities.push(Identity {
                    dev: entry.meta.dev,
                    ino: entry.meta.ino,
                    place: String::from_utf8_lossy(&path).into_owned(),
                    inside,
                });
            }
        }
        self.identities = identities;
    }

    /// The protected place met on the way to a target, by identity: `way` is the identity of every
    /// component from the top down, the target last.
    pub(crate) fn hit_by_identity(&self, way: &[(u64, u64)]) -> Option<String> {
        let target = way.last()?;
        self.identities
            .iter()
            .find(|known| {
                let id = (known.dev, known.ino);
                if known.inside {
                    way.contains(&id)
                } else {
                    *target == id
                }
            })
            .map(|known| known.place.clone())
    }

    /// The protected place `target` hits, if any, as a readable path.
    pub(crate) fn hit<A: AsRef<[u8]>>(&self, target: &[A]) -> Option<String> {
        let above = self.not_or_above.iter().find(|place| is_within(place, target));
        let inside = self.not_inside.iter().find(|place| is_within(target, place));

        above.or(inside).map(|place| {
            let names: Vec<String> = place
                .iter()
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect();
            format!("/{}", names.join("/"))
        })
    }
}

fn join_parts(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut path = Vec::new();
    for part in parts {
        path.push(b'/');
        path.extend_from_slice(part);
    }
    path
}
