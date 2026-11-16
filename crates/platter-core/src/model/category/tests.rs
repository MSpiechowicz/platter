use std::collections::HashSet;

use super::*;
use crate::model::category_table::{EXTENSIONS, SIZE_RULES};

const MIB: u64 = 1024 * 1024;

fn file(name: &str) -> Category {
    Category::of_file(name.as_bytes(), 1000)
}

fn dir(parent: Option<&str>, name: &str, project_like: bool) -> Option<Category> {
    Category::of_directory(parent.map(str::as_bytes), name.as_bytes(), &mut || project_like)
}

fn assert_files(expected: Category, names: &[&str]) {
    for name in names {
        assert_eq!(file(name), expected, "{name}");
    }
}

#[test]
fn classifies_by_extension_case_insensitively() {
    assert_eq!(file("movie.MKV"), Category::Video);
    assert_eq!(file("photo.jpeg"), Category::Image);
    assert_eq!(file("song.flac"), Category::Audio);
    assert_eq!(file("report.pdf"), Category::Document);
    assert_eq!(file("backup.tar.zst"), Category::Archive);
    assert_eq!(file("main.rs"), Category::Code);
    assert_eq!(file("tool.AppImage"), Category::App);
    assert_eq!(file("disk.qcow2"), Category::DiskImage);
    assert_eq!(file("Level.UASSET"), Category::App);
    assert_eq!(file("sharedassets0.resS"), Category::App);
}

#[test]
fn names_without_a_usable_extension_are_other() {
    assert_files(
        Category::Other,
        &["Makefile", ".bashrc", "trailing.", "weird.unknownext", "data.bin"],
    );
    assert_eq!(Category::of_file(&[0xff, b'.', 0xfe], 10), Category::Other);
}

#[test]
fn game_and_engine_data_and_binaries_are_apps() {
    assert_files(
        Category::App,
        &[
            "a.ucas",
            "a.utoc",
            "a.pak",
            "a.ress",
            "a.resS",
            "a.bundle",
            "a.unity3d",
            "a.assets",
            "a.assetbundle",
            "a.resource",
            "a.wem",
            "a.bnk",
            "a.bank",
            "a.bk2",
            "a.vpk",
            "a.gcf",
            "a.forge",
            "a.arc",
            "a.rpf",
            "a.big",
            "a.wad",
            "a.pck",
            "a.uasset",
            "a.umap",
            "a.upk",
            "a.cook",
            "a.foz",
            "a.pack",
            "a.dll",
            "a.so",
            "a.exe",
            "a.dylib",
            "a.node",
            "a.appimage",
        ],
    );
}

#[test]
fn raw_is_an_image_up_to_256_mib_and_a_disk_image_above() {
    let raw = |size| Category::of_file(b"IMG_0001.RAW", size);

    assert_eq!(raw(0), Category::Image);
    assert_eq!(raw(30 * MIB), Category::Image);
    assert_eq!(raw(256 * MIB), Category::Image);
    assert_eq!(raw(256 * MIB + 1), Category::DiskImage);
    assert_eq!(raw(3_500 * MIB), Category::DiskImage);
}

#[test]
fn other_extensions_ignore_the_size() {
    for size in [0, 1, 256 * MIB, u64::MAX] {
        assert_eq!(Category::of_file(b"x.png", size), Category::Image);
        assert_eq!(Category::of_file(b"x.iso", size), Category::DiskImage);
    }
}

#[test]
fn disk_image_extensions_are_disk_images_at_any_size() {
    for size in [0, 10, 10 * MIB * 1000] {
        for name in ["a.img", "a.iso", "a.qcow2", "a.vmdk", "a.vdi", "a.vhd", "a.vhdx"] {
            assert_eq!(
                Category::of_file(name.as_bytes(), size),
                Category::DiskImage,
                "{name}"
            );
        }
    }
}

#[test]
fn build_artifacts_are_build() {
    assert_files(
        Category::Build,
        &[
            "a.o", "a.obj", "a.a", "a.lib", "a.rlib", "a.rmeta", "a.pdb", "a.ilk", "a.class", "a.pyc",
            "a.pyo", "a.d",
        ],
    );
}

#[test]
fn compressed_files_are_archives() {
    assert_files(
        Category::Archive,
        &[
            "a.zst", "a.xz", "a.bz2", "a.gz", "a.tgz", "a.7z", "a.rar", "a.zip", "a.tar", "a.lz4", "a.br",
        ],
    );
}

#[test]
fn package_formats_are_packages() {
    assert_files(
        Category::Package,
        &[
            "a.deb",
            "a.rpm",
            "a.apk",
            "a.flatpak",
            "a.snap",
            "a.whl",
            "a.crate",
            "a.jar",
            "a.nupkg",
            "linux-6.1-1-x86_64.pkg.tar.zst",
            "tool.PKG.TAR.XZ",
        ],
    );
}

#[test]
fn compound_extensions_need_a_stem_and_leave_plain_archives_alone() {
    assert_eq!(file("pkg.tar.zst"), Category::Archive);
    assert_eq!(file(".pkg.tar.zst"), Category::Archive);
    assert_eq!(file("backup.tar.zst"), Category::Archive);
    assert_eq!(file("notpkg.tar.zst"), Category::Archive);
}

#[test]
fn data_and_models_are_data() {
    assert_files(
        Category::Data,
        &[
            "a.sqlite",
            "a.sqlite3",
            "a.db",
            "a.mdb",
            "a.jsonl",
            "a.parquet",
            "a.csv",
            "a.npy",
            "a.npz",
            "a.h5",
            "a.hdf5",
            "a.safetensors",
            "a.gguf",
            "a.ckpt",
            "a.pt",
            "a.pth",
            "a.onnx",
            "a.tflite",
        ],
    );
}

#[test]
fn an_extension_has_one_rule() {
    let mut seen = HashSet::new();

    let all = EXTENSIONS
        .iter()
        .flat_map(|(_, extensions)| extensions.iter().copied())
        .chain(SIZE_RULES.iter().map(|rule| rule.extension));
    for extension in all {
        assert!(seen.insert(extension), "{extension} is listed twice");
        assert_eq!(
            extension,
            extension.to_ascii_lowercase(),
            "{extension} must be lowercase"
        );
        assert!(
            extension.len() <= MAX_EXTENSION,
            "{extension} is too long to ever match"
        );
    }
}

#[test]
fn unconditional_directories_impose_a_category() {
    let cases: &[(Category, &[&str])] = &[
        (Category::Code, &[".git"]),
        (
            Category::App,
            &[
                "steamapps",
                "Steam",
                ".steam",
                "compatdata",
                "shadercache",
                "Proton",
                "Proton-GE-9",
                "Proton Experimental",
                ".wine",
                "Faugus",
                "FaugusLauncher",
                "lutris",
                "heroic",
                "Games",
            ],
        ),
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
                ".rustup",
                ".gradle",
                ".m2",
                ".nuget",
                ".pnpm-store",
                ".yarn",
                "site-packages",
                "node_modules",
                ".venv",
                "venv",
                "bower_components",
            ],
        ),
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
                "App.dSYM",
                "x.dSYM",
            ],
        ),
    ];

    for (category, names) in cases {
        for name in *names {
            assert_eq!(dir(None, name, false), Some(*category), "{name}");
            assert_eq!(dir(Some("anything"), name, false), Some(*category), "{name}");
        }
    }
}

#[test]
fn directory_names_are_case_sensitive_and_match_whole_names() {
    for name in [
        "src",
        "Documents",
        "cache",
        "node_module",
        "My .git",
        "Gamesx",
        "Proto",
        "dSYM",
    ] {
        assert_eq!(dir(None, name, true), None, "{name}");
    }
}

#[test]
fn parent_qualified_rules_need_that_parent() {
    let cases = [(".cargo", "registry"), ("go", "pkg")];

    for (parent, name) in cases {
        assert_eq!(
            dir(Some(parent), name, false),
            Some(Category::Package),
            "{parent}/{name}"
        );
        assert_eq!(dir(Some("other"), name, false), None, "other/{name}");
        assert_eq!(dir(None, name, false), None, "{name} without parent");
    }
    assert_eq!(dir(Some(".cargo"), "bin", false), None);
}

#[test]
fn project_rules_need_a_project_like_directory() {
    let names = [
        "target",
        "build",
        "dist",
        "out",
        ".next",
        ".turbo",
        "cmake-build-debug",
        "cmake-build-",
    ];

    for name in names {
        assert_eq!(dir(Some("proj"), name, true), Some(Category::Build), "{name}");
        assert_eq!(dir(Some("proj"), name, false), None, "{name} outside a project");
    }
}

#[test]
fn the_project_probe_runs_only_when_a_project_rule_matches_and_at_most_once() {
    let mut calls = 0;
    let mut probe = || {
        calls += 1;
        true
    };

    assert_eq!(Category::of_directory(None, b"src", &mut probe), None);
    assert_eq!(
        Category::of_directory(None, b"node_modules", &mut probe),
        Some(Category::Package)
    );
    assert_eq!(
        Category::of_directory(None, b"build", &mut probe),
        Some(Category::Build)
    );
    assert_eq!(calls, 1);
}

#[test]
fn project_markers_are_recognized_by_exact_name() {
    for name in [
        ".git",
        "package.json",
        "Cargo.toml",
        "CMakeLists.txt",
        "pyproject.toml",
    ] {
        assert!(Category::is_project_marker(name.as_bytes()), "{name}");
    }
    for name in ["cargo.toml", "package.json.bak", "src", ""] {
        assert!(!Category::is_project_marker(name.as_bytes()), "{name}");
    }
}

#[test]
fn index_matches_position_in_all() {
    for (i, category) in Category::ALL.iter().enumerate() {
        assert_eq!(category.index(), i);
    }
}
