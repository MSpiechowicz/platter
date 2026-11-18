//! The macOS adapters (P2). Every module here exists from package M0, so the packages that fill them
//! (M1 to M3) never edit `lib.rs`, this file, `mac_pure/mod.rs` or the routing in `runtime.rs`: each fills
//! the files it owns. Until a package has landed, its module compiles and refuses (`Unsupported`), the way
//! [`crate::unsupported`] does, so cleanup and the advisor fail closed.
//!
//! | Module | Filled by | What it is |
//! |---|---|---|
//! | [`reader`] | M1 | [`MacDirSource`]: the scan's directory reader (for now `GenericDirSource`) |
//! | [`exclusions`] | M1 | default exclusions (`/System/Volumes`, `devfs`, network filesystems, `.Spotlight-V100`, ...) |
//! | [`limits`] | M1 | raising the soft limit on open descriptors |
//! | [`volumes`] | M1 | the volume list's macOS rules (hides `/System/Volumes/*`), labels |
//! | [`probe`] | M2a | [`MacEntryProbe`]: looks at a path component by component |
//! | [`remove`] | M2a | [`MacEntryRemover`]: moves a target into the trash, identity checked, and puts it back |
//! | [`trash`] | M2a | Platter's own trash: `~/.Trash`, or `<mount>/.Trashes/<uid>` on other volumes |
//! | [`tools`] | M2a | [`MacToolRunner`]: runs no elevated tools, and unelevated ones only from root-owned system folders |
//! | [`runtime`] | M3 | the advisor's helper process: socket, model descriptor, environment |
//! | [`pinned`] | M3 | opening the model file so that what was checked is what is loaded (`/dev/fd/N`) |
//! | [`parent_watch`] | M3 | the helper ends when its parent does (socket end of file, and polling `getppid`) |
//!
//! The pure rules live in [`crate::mac_pure`], which is compiled on every OS so that their tests run on
//! Linux. Safe code only: `unsafe` is forbidden in this workspace, and `rustix` is the one system crate.
//! The contract (paths, same-disk rule, trash layout, actions) is under "macOS (P2)" in
//! `docs/architecture.md`.

pub mod exclusions;
pub mod limits;
pub mod parent_watch;
pub mod pinned;
pub mod probe;
pub mod reader;
pub mod remove;
pub mod runtime;
pub mod tools;
mod trash;
pub mod volumes;

pub use probe::MacEntryProbe;
pub use reader::MacDirSource;
pub use remove::MacEntryRemover;
pub use tools::MacToolRunner;
