//! Data tables behind [`Category`] classification.
//!
//! This is the one place to extend. Add an extension or a directory name to the matching row; no other
//! code needs to change. Extensions are lowercase and without the dot (matching ignores case); directory
//! names are matched exactly (case-sensitive).
//!
//! # Files
//!
//! 1. [`COMPOUND_EXTENSIONS`]: multi-part suffixes such as `pkg.tar.zst`, checked first.
//! 2. [`SIZE_RULES`]: an extension whose category depends on the file size (`.raw`).
//! 3. [`EXTENSIONS`]: the plain extension to category map. An extension must appear only once across
//!    this table and [`SIZE_RULES`]; a test enforces it.
//!
//! # Directories (location rules)
//!
//! A directory whose name matches a rule imposes its category on its whole subtree. The outermost match
//! wins, so a `.js` file inside `node_modules` is `package`, not `code`, and a `.pack` file inside
//! `.git` is `code` (version-control data) wherever else `.pack` means a game archive (`app`).
//!
//! A pattern is one of
//!
//! - `name`: the directory is called exactly `name`;
//! - `prefix*` or `*suffix`: the name starts or ends with the given text;
//! - `parent/name`: the directory is called `name` and its parent is called `parent`.
//!   (`name` may itself be a `prefix*` or `*suffix` pattern.)
//!
//! Rules in [`DIRECTORY_NAMES`] always apply. Rules in [`PROJECT_DIRECTORY_NAMES`] apply only when the
//! directory is *project-like*: it sits in a directory whose listing contains one of the
//! [`PROJECT_MARKERS`]. That keeps a folder a user happened to call `build` or `out` in their home
//! directory from being treated as build output.

use super::Category;

/// File extension to category.
pub const EXTENSIONS: &[(Category, &[&str])] = &[
    (
        Category::Video,
        &[
            "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "m2ts", "vob", "3gp",
            "ogv",
        ],
    ),
    (
        Category::Image,
        &[
            "jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "tif", "tiff", "heic", "heif", "avif", "cr2",
            "nef", "arw", "dng", "psd", "xcf", "ico",
        ],
    ),
    (
        Category::Audio,
        &[
            "mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "aac", "wma", "aiff", "mid", "midi", "ape",
        ],
    ),
    (
        Category::Document,
        &[
            "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "xls", "xlsx", "ods", "ppt", "pptx", "odp",
            "epub", "mobi", "tex", "log",
        ],
    ),
    (
        Category::Archive,
        &[
            "zip", "tar", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar", "lz4", "lzma", "cab", "cpio", "br",
        ],
    ),
    (
        Category::Code,
        &[
            "rs", "py", "js", "mjs", "cjs", "jsx", "tsx", "svelte", "vue", "c", "h", "cc", "cpp", "hpp",
            "cs", "java", "kt", "go", "rb", "php", "swift", "lua", "sh", "bash", "zsh", "fish", "html",
            "css", "scss", "json", "yaml", "yml", "toml", "xml", "sql", "ts", "zig", "hs", "ex", "exs",
            "dart", "gd",
        ],
    ),
    (
        Category::Build,
        &[
            "o", "obj", "a", "lib", "rlib", "rmeta", "pdb", "ilk", "class", "pyc", "pyo", "d", "wasm", "map",
        ],
    ),
    (
        Category::Package,
        &[
            "deb", "rpm", "apk", "flatpak", "snap", "whl", "egg", "gem", "crate", "jar", "nupkg",
        ],
    ),
    (
        Category::App,
        &[
            // Programs and libraries.
            "exe",
            "msi",
            "pkg",
            "run",
            "dll",
            "so",
            "dylib",
            "node",
            "appimage",
            // Game and engine data (Unreal, Unity, Source, Godot, Bethesda, Steam shader cache, ...).
            "ucas",
            "utoc",
            "pak",
            "ress",
            "bundle",
            "unity3d",
            "assets",
            "assetbundle",
            "resource",
            "wem",
            "bnk",
            "bank",
            "bk2",
            "vpk",
            "gcf",
            "forge",
            "arc",
            "rpf",
            "big",
            "wad",
            "pck",
            "uasset",
            "umap",
            "upk",
            "cook",
            "foz",
            "pack",
        ],
    ),
    (
        Category::DiskImage,
        &[
            "iso", "img", "dmg", "vmdk", "vdi", "vhd", "vhdx", "qcow2", "qcow", "wim", "squashfs",
        ],
    ),
    (
        Category::Data,
        &[
            "sqlite",
            "sqlite3",
            "db",
            "mdb",
            "jsonl",
            "parquet",
            "csv",
            "npy",
            "npz",
            "h5",
            "hdf5",
            "safetensors",
            "gguf",
            "ckpt",
            "pt",
            "pth",
            "onnx",
            "tflite",
        ],
    ),
];

/// An extension that means two things, told apart by the file's apparent size.
pub struct SizeRule {
    pub extension: &'static str,
    /// Files strictly larger than this many bytes are `large`; all others are `small`.
    pub above: u64,
    pub small: Category,
    pub large: Category,
}

const MIB: u64 = 1024 * 1024;

pub const SIZE_RULES: &[SizeRule] = &[
    // Camera raw photos are tens of MB; a `.raw` of several hundred MB is a raw disk or data image.
    SizeRule {
        extension: "raw",
        above: 256 * MIB,
        small: Category::Image,
        large: Category::DiskImage,
    },
];

/// Multi-part suffixes (without the leading dot) that beat the last-extension lookup, so that
/// `foo.pkg.tar.zst` is a package and not just a `.zst` archive.
pub const COMPOUND_EXTENSIONS: &[(Category, &[&str])] = &[(
    Category::Package,
    &["pkg.tar.zst", "pkg.tar.xz", "pkg.tar.gz", "pkg.tar"],
)];

/// Location rules that always apply (see the module docs for the pattern syntax).
pub const DIRECTORY_NAMES: &[(Category, &[&str])] = &[
    // Version-control data: `.git/objects/pack/*.pack` is code, not a game pack.
    (Category::Code, &[".git"]),
    // Game launchers, stores, prefixes and the data they keep.
    (
        Category::App,
        &[
            "steamapps",
            "Steam",
            ".steam",
            "compatdata",
            "shadercache",
            "Proton*",
            ".wine",
            "Faugus*",
            "lutris",
            "heroic",
            "Games",
        ],
    ),
    // Downloaded or re-creatable dependencies and caches.
    (
        Category::Package,
        &[
            ".cache",
            "Cache",
            "CachedData",
            "GPUCache",
            "Code Cache",
            "__pycache__",
            ".npm",
            ".bun",
            ".cargo/registry",
            ".rustup",
            ".gradle",
            ".m2",
            ".nuget",
            "go/pkg",
            ".pnpm-store",
            ".yarn", // includes .yarn/cache
            "site-packages",
            "node_modules",
            ".venv",
            "venv",
            "bower_components",
        ],
    ),
    // Tool caches and bundles that are build output wherever they are.
    (
        Category::Build,
        &[
            ".nuxt",
            ".svelte-kit",
            ".parcel-cache",
            ".tox",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".angular",
            "*.dSYM",
        ],
    ),
];

/// Location rules for names that are common outside of software projects; they apply only when the
/// directory is project-like (see the module docs).
pub const PROJECT_DIRECTORY_NAMES: &[(Category, &[&str])] = &[(
    Category::Build,
    &[
        "target",
        "build",
        "dist",
        "out",
        ".next",
        ".turbo",
        "cmake-build-*",
    ],
)];

/// Names whose presence in a directory's listing makes the directory a software project.
pub const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "package.json",
    "Cargo.toml",
    "CMakeLists.txt",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "go.mod",
    "Makefile",
    "meson.build",
];
