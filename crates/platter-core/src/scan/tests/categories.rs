//! Classification as the scanner applies it: file extensions with sizes, location rules on directories,
//! the project check against the directory listing, and the outermost match winning.

use super::find;
use crate::model::Category;
use crate::testing::{FakeFs, scan_fake};

const MIB: u64 = 1024 * 1024;

/// Scans a fake filesystem with one file per `(path, size)` and checks each file's category.
fn assert_files(files: &[(&str, u64, Category)]) {
    let mut fs = FakeFs::new();
    for (path, size, _) in files {
        fs.file(path, *size);
    }
    let tree = scan_fake(&fs, false);

    for (path, _, expected) in files {
        let found = tree.category(find(&tree, path));
        assert_eq!(found, *expected, "{path}");
    }
}

#[test]
fn well_known_directories_impose_their_category_on_files_inside() {
    assert_files(&[
        ("/h/.git/HEAD", 1, Category::Code),
        ("/h/.git/objects/pack/pack-1.pack", 9, Category::Code),
        ("/h/.git/objects/pack/pack-1.idx", 9, Category::Code),
        ("/h/Steam/steamapps/common/g/data.bin", 9, Category::App),
        ("/h/.steam/root/x.png", 9, Category::App),
        ("/h/Proton-9/files/a.txt", 9, Category::App),
        ("/h/.local/share/lutris/runners/a.txt", 9, Category::App),
        ("/h/Games/g/readme.txt", 9, Category::App),
        ("/h/.cache/thumbs/a.png", 9, Category::Package),
        ("/h/app/GPUCache/data_0", 9, Category::Package),
        ("/h/app/Code Cache/js/a.js", 9, Category::Package),
        ("/h/lib/python3/site-packages/x/y.py", 9, Category::Package),
        ("/h/p/node_modules/x/index.js", 9, Category::Package),
    ]);
}

#[test]
fn pack_files_are_code_inside_git_and_apps_elsewhere() {
    assert_files(&[
        ("/r/proj/.git/objects/pack/pack-abc.pack", 9, Category::Code),
        ("/r/Games/title/data/main.pack", 9, Category::App),
        ("/r/loose/data.pack", 9, Category::App),
        ("/r/loose/DATA.PACK", 9, Category::App),
    ]);
}

#[test]
fn the_outermost_location_rule_wins() {
    assert_files(&[
        // `Games` is outside `node_modules`, `.git` and `.cache`.
        ("/h/Games/g/node_modules/a.js", 9, Category::App),
        ("/h/Games/g/.git/HEAD", 9, Category::App),
        // `node_modules` is outside `Games`.
        ("/h/node_modules/Games/a.js", 9, Category::Package),
        // `.git` is outside `.cache` and `node_modules`.
        ("/h/.git/modules/node_modules/a.js", 9, Category::Code),
        ("/h/.cache/.git/a.txt", 9, Category::Package),
    ]);
}

#[test]
fn a_directory_takes_the_category_its_name_imposes_even_when_empty() {
    let mut fs = FakeFs::new();
    fs.dir("/h/node_modules").dir("/h/.git").dir("/h/plain");
    let tree = scan_fake(&fs, false);

    assert_eq!(tree.category(find(&tree, "h/node_modules")), Category::Package);
    assert_eq!(tree.category(find(&tree, "h/.git")), Category::Code);
    assert_eq!(tree.category(find(&tree, "h/plain")), Category::Other);
}

#[test]
fn dsym_bundles_are_build_output() {
    assert_files(&[("/h/out/App.dSYM/Contents/Resources/DWARF/app", 9, Category::Build)]);
}

#[test]
fn parent_qualified_rules_look_at_the_real_parent() {
    assert_files(&[
        ("/h/.cargo/registry/src/crate/lib.rs", 9, Category::Package),
        ("/h/.cargo/bin/tool.rs", 9, Category::Code),
        ("/h/registry/src/lib.rs", 9, Category::Code),
        ("/h/go/pkg/mod/x/a.go", 9, Category::Package),
        ("/h/go/src/a.go", 9, Category::Code),
        ("/h/pkg/a.go", 9, Category::Code),
    ]);
}

#[test]
fn a_parent_qualified_rule_matches_below_the_scan_root_too() {
    let mut fs = FakeFs::new();
    fs.file("/home/me/.cargo/registry/x/lib.rs", 9);

    let config = crate::scan::ScanConfig::default();
    let tree = crate::scan::scan(
        &fs,
        std::path::Path::new("/home/me/.cargo"),
        &config,
        &crate::scan::CancelToken::new(),
        &|_| {},
    )
    .unwrap();

    assert_eq!(tree.category(find(&tree, "registry/x/lib.rs")), Category::Package);
}

#[test]
fn build_directories_count_only_next_to_a_project_marker() {
    let markers = [
        "Cargo.toml",
        "package.json",
        "CMakeLists.txt",
        "pyproject.toml",
        "pom.xml",
        "go.mod",
        "Makefile",
    ];

    for marker in markers {
        for dir in [
            "target",
            "build",
            "dist",
            "out",
            ".next",
            ".turbo",
            "cmake-build-debug",
        ] {
            let project = format!("/h/proj/{dir}/a.png");
            let plain = format!("/h/folder/{dir}/a.png");

            let mut fs = FakeFs::new();
            fs.file(&format!("/h/proj/{marker}"), 1)
                .file(&project, 9)
                .file(&plain, 9);
            let tree = scan_fake(&fs, false);

            let kind = |path: &str| tree.category(find(&tree, path.trim_start_matches('/')));
            assert_eq!(kind(&project), Category::Build, "{dir} next to {marker}");
            assert_eq!(kind(&plain), Category::Image, "{dir} without a marker");
        }
    }
}

#[test]
fn a_marker_directory_such_as_git_makes_a_project_and_only_its_own_listing_counts() {
    let mut fs = FakeFs::new();
    fs.dir("/h/proj/.git")
        .file("/h/proj/dist/a.png", 9)
        // The marker is in a sibling folder, one level up, or one level down: not next to `out`.
        .file("/h/other/Cargo.toml", 1)
        .file("/h/other/x/out/a.png", 9)
        .file("/h/up/out/a.png", 9)
        .file("/h/up/Cargo.toml", 1)
        .file("/h/below/out/Cargo.toml", 1)
        .file("/h/below/out/a.png", 9);
    let tree = scan_fake(&fs, false);

    assert_eq!(tree.category(find(&tree, "h/proj/dist/a.png")), Category::Build);
    assert_eq!(tree.category(find(&tree, "h/other/x/out/a.png")), Category::Image);
    assert_eq!(tree.category(find(&tree, "h/up/out/a.png")), Category::Build);
    assert_eq!(
        tree.category(find(&tree, "h/below/out/a.png")),
        Category::Image,
        "a marker inside the folder is not beside it"
    );
}

#[test]
fn a_user_folder_called_build_at_the_top_of_the_scan_stays_unclassified() {
    let mut fs = FakeFs::new();
    fs.file("/home/me/build/holiday.png", 9)
        .file("/home/me/out/notes.txt", 9);
    let tree = scan_fake(&fs, false);

    assert_eq!(
        tree.category(find(&tree, "home/me/build/holiday.png")),
        Category::Image
    );
    assert_eq!(
        tree.category(find(&tree, "home/me/out/notes.txt")),
        Category::Document
    );
}

#[test]
fn raw_files_are_classified_by_apparent_size() {
    let mut fs = FakeFs::new();
    fs.file("/p/photo.raw", 24 * MIB)
        .file("/p/edge.raw", 256 * MIB)
        .file("/p/over.raw", 256 * MIB + 1)
        .file("/p/disk.raw", 3_500 * MIB)
        // Sparse: few blocks allocated, but the file is a long disk image.
        .file_sized("/p/sparse.raw", 4096, 10_000 * MIB);
    let tree = scan_fake(&fs, false);

    let of = |name: &str| tree.category(find(&tree, &format!("p/{name}")));
    assert_eq!(of("photo.raw"), Category::Image);
    assert_eq!(of("edge.raw"), Category::Image);
    assert_eq!(of("over.raw"), Category::DiskImage);
    assert_eq!(of("disk.raw"), Category::DiskImage);
    assert_eq!(of("sparse.raw"), Category::DiskImage);
}

#[test]
fn files_without_an_extension_are_other_unless_a_location_rule_applies() {
    assert_files(&[
        ("/h/docs/LICENSE", 9, Category::Other),
        ("/h/docs/data", 9, Category::Other),
        ("/h/.cache/blob", 9, Category::Package),
        ("/h/.git/objects/ab/cdef", 9, Category::Code),
    ]);
}

#[test]
fn a_big_game_library_ends_up_as_apps_and_models_as_data() {
    let mut fs = FakeFs::new();
    fs.file("/h/Steam/steamapps/common/g/data.ucas", 1000)
        .file("/h/ml/model.safetensors", 500)
        .file("/h/ml/notes.bin", 100);
    let tree = scan_fake(&fs, false);

    assert_eq!(tree.category(find(&tree, "h/Steam")), Category::App);
    assert_eq!(
        tree.category(find(&tree, "h/ml/model.safetensors")),
        Category::Data
    );
    assert_eq!(tree.category(find(&tree, "h/ml")), Category::Data);
    assert_eq!(tree.category(find(&tree, "h/ml/notes.bin")), Category::Other);
}
