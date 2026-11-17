//! Pins the wire format: each DTO must serialize to exactly this JSON and read it back.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::*;

fn assert_wire<T>(value: &T, expected: Value)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let actual = serde_json::to_value(value).expect("serializes");
    assert_eq!(actual, expected, "wire shape");

    let back: T = serde_json::from_value(expected).expect("deserializes");
    assert_eq!(&back, value, "round trip");
}

fn node(id: u32) -> NodeDto {
    NodeDto {
        id: NodeId(id),
        name: "movies".into(),
        kind: NodeKind::Dir,
        allocated: 4096,
        apparent: 100,
        files: 3,
        has_children: true,
        unreadable: false,
        skipped: None,
        category: Category::DiskImage,
        mtime: 1_700_000_000,
    }
}

fn node_json(id: u32) -> Value {
    json!({
        "id": id, "name": "movies", "kind": "dir", "allocated": 4096, "apparent": 100, "files": 3,
        "hasChildren": true, "unreadable": false, "category": "diskImage", "mtime": 1_700_000_000
    })
}

#[test]
fn skipped_is_present_only_on_mount_nodes_and_spelled_in_camel_case() {
    let plain = serde_json::to_value(node(1)).unwrap();
    assert!(
        plain.get("skipped").is_none(),
        "omitted, not null, for every node that is not a mount: {plain}"
    );

    for (reason, wire) in [
        (SkipReason::OtherDisk, "otherDisk"),
        (SkipReason::System, "system"),
        (SkipReason::Duplicate, "duplicate"),
        (SkipReason::Excluded, "excluded"),
        (SkipReason::Unknown, "unknown"),
    ] {
        let mount = NodeDto {
            kind: NodeKind::Mount,
            skipped: Some(reason),
            ..node(2)
        };
        let json = serde_json::to_value(&mount).unwrap();

        assert_eq!(json["kind"], "mount");
        assert_eq!(json["skipped"], wire);
        assert_eq!(serde_json::from_value::<NodeDto>(json).unwrap(), mount);
    }

    // A consumer that sends no `skipped` at all still parses.
    let parsed: NodeDto = serde_json::from_value(node_json(3)).unwrap();
    assert_eq!(parsed.skipped, None);
}

#[test]
fn node_always_carries_unreadable() {
    let wire = serde_json::to_value(node(1)).unwrap();
    assert_eq!(wire["unreadable"], json!(false), "present even when false");

    let locked = NodeDto {
        unreadable: true,
        ..node(1)
    };
    assert_eq!(serde_json::to_value(&locked).unwrap()["unreadable"], json!(true));
    assert_wire(&locked, {
        let mut expected = node_json(1);
        expected["unreadable"] = json!(true);
        expected
    });
}

#[test]
fn volume() {
    let dto = VolumeDto {
        id: "/home".into(),
        mount_point: "/home".into(),
        label: "Home".into(),
        device: "/dev/nvme0n1p2".into(),
        fs_type: "btrfs".into(),
        total: 10,
        available: 4,
        removable: false,
    };
    assert_wire(
        &dto,
        json!({
            "id": "/home", "mountPoint": "/home", "label": "Home", "device": "/dev/nvme0n1p2",
            "fsType": "btrfs", "total": 10, "available": 4, "removable": false
        }),
    );
}

#[test]
fn scan_options() {
    let dto = ScanOptions {
        root: "/home".into(),
        one_file_system: true,
        excluded: vec!["/home/me/.cache".into()],
    };
    assert_wire(
        &dto,
        json!({ "root": "/home", "oneFileSystem": true, "excluded": ["/home/me/.cache"] }),
    );
}

#[test]
fn scan_options_without_excluded_folders_parse_as_an_empty_list() {
    let parsed: ScanOptions =
        serde_json::from_value(json!({ "root": "/home", "oneFileSystem": false })).unwrap();

    assert!(parsed.excluded.is_empty());
}

#[test]
fn totals() {
    let totals = Totals {
        allocated: 1,
        apparent: 2,
        files: 3,
        dirs: 4,
        errors: 5,
    };
    assert_wire(
        &totals,
        json!({ "allocated": 1, "apparent": 2, "files": 3, "dirs": 4, "errors": 5 }),
    );
}

#[test]
fn scan_events() {
    assert_wire(
        &ScanEvent::Progress {
            files: 1,
            dirs: 2,
            bytes: 3,
            errors: 4,
            current_path: "/a".into(),
        },
        json!({ "type": "progress", "files": 1, "dirs": 2, "bytes": 3, "errors": 4, "currentPath": "/a" }),
    );
    assert_wire(
        &ScanEvent::Finished {
            scan_id: 7,
            root_id: NodeId(0),
            totals: Totals {
                allocated: 1,
                apparent: 2,
                files: 3,
                dirs: 4,
                errors: 5,
            },
            elapsed_ms: 99,
            warnings: vec![],
        },
        json!({
            "type": "finished", "scanId": 7, "rootId": 0, "elapsedMs": 99,
            "totals": { "allocated": 1, "apparent": 2, "files": 3, "dirs": 4, "errors": 5 }
        }),
    );
    assert_wire(
        &ScanEvent::Finished {
            scan_id: 7,
            root_id: NodeId(0),
            totals: Totals::default(),
            elapsed_ms: 99,
            warnings: vec![ScanWarning::MountTableUnavailable],
        },
        json!({
            "type": "finished", "scanId": 7, "rootId": 0, "elapsedMs": 99,
            "totals": { "allocated": 0, "apparent": 0, "files": 0, "dirs": 0, "errors": 0 },
            "warnings": ["mountTableUnavailable"]
        }),
    );
    assert_wire(&ScanEvent::Cancelled, json!({ "type": "cancelled" }));
    assert_wire(
        &ScanEvent::Failed {
            message: "boom".into(),
        },
        json!({ "type": "failed", "message": "boom" }),
    );
}

#[test]
fn node_and_enums() {
    assert_wire(&node(5), node_json(5));

    for (kind, wire) in [
        (NodeKind::Dir, "dir"),
        (NodeKind::File, "file"),
        (NodeKind::Symlink, "symlink"),
        (NodeKind::Mount, "mount"),
        (NodeKind::Other, "other"),
        (NodeKind::Collapsed, "collapsed"),
    ] {
        assert_wire(&kind, json!(wire));
    }

    let categories = [
        "video",
        "image",
        "audio",
        "document",
        "archive",
        "code",
        "build",
        "package",
        "app",
        "diskImage",
        "data",
        "other",
    ];
    for (category, wire) in Category::ALL.iter().zip(categories) {
        assert_wire(category, json!(wire));
    }
}

#[test]
fn sort_keys() {
    for (key, wire) in [
        (SortKey::SizeDesc, "sizeDesc"),
        (SortKey::SizeAsc, "sizeAsc"),
        (SortKey::NameAsc, "nameAsc"),
        (SortKey::FilesDesc, "filesDesc"),
        (SortKey::MtimeDesc, "mtimeDesc"),
    ] {
        assert_wire(&key, json!(wire));
    }
}

#[test]
fn children_page_and_lod() {
    assert_wire(
        &ChildrenPage {
            parent_id: NodeId(1),
            total: 9,
            items: vec![node(2)],
        },
        json!({ "parentId": 1, "total": 9, "items": [node_json(2)] }),
    );
    assert_wire(
        &LodNode {
            node: node(1),
            children: vec![LodNode {
                node: node(2),
                children: vec![],
            }],
        },
        json!({ "node": node_json(1), "children": [{ "node": node_json(2), "children": [] }] }),
    );
}

#[test]
fn path() {
    let dto = PathDto {
        path: "/home/me".into(),
        segments: vec![
            PathSegment {
                id: NodeId(0),
                name: "/".into(),
            },
            PathSegment {
                id: NodeId(3),
                name: "me".into(),
            },
        ],
    };
    assert_wire(
        &dto,
        json!({ "path": "/home/me", "segments": [{ "id": 0, "name": "/" }, { "id": 3, "name": "me" }] }),
    );
}

#[test]
fn summary() {
    let dto = SummaryDto {
        node: node(1),
        by_category: vec![CategoryStat {
            category: Category::Video,
            allocated: 10,
            files: 2,
        }],
        by_age: vec![AgeStat {
            bucket: AgeBucket::TwoYears,
            allocated: 5,
            files: 1,
        }],
        largest_dirs: vec![node(4)],
    };
    assert_wire(
        &dto,
        json!({
            "node": node_json(1),
            "byCategory": [{ "category": "video", "allocated": 10, "files": 2 }],
            "byAge": [{ "bucket": "twoYears", "allocated": 5, "files": 1 }],
            "largestDirs": [node_json(4)]
        }),
    );

    for (bucket, wire) in [
        (AgeBucket::Week, "week"),
        (AgeBucket::Month, "month"),
        (AgeBucket::Quarter, "quarter"),
        (AgeBucket::Year, "year"),
        (AgeBucket::TwoYears, "twoYears"),
        (AgeBucket::Older, "older"),
    ] {
        assert_wire(&bucket, json!(wire));
    }
}

#[test]
fn api_error() {
    for (code, wire) in [
        (ApiErrorCode::NotFound, "notFound"),
        (ApiErrorCode::Cancelled, "cancelled"),
        (ApiErrorCode::Io, "io"),
        (ApiErrorCode::Invalid, "invalid"),
    ] {
        assert_wire(&ApiError::new(code, "m"), json!({ "code": wire, "message": "m" }));
    }
}

#[test]
fn sizes_and_counts_above_the_javascript_safe_integer_are_clamped_on_the_wire() {
    let max = json!(9_007_199_254_740_991_u64);

    let huge = NodeDto {
        allocated: u64::MAX,
        apparent: (1 << 53) + 1,
        files: u64::MAX,
        ..node(1)
    };
    let wire = serde_json::to_value(&huge).unwrap();
    assert_eq!(
        (&wire["allocated"], &wire["apparent"], &wire["files"]),
        (&max, &max, &max)
    );
    assert_eq!(huge.allocated, u64::MAX, "the value itself is untouched");

    let exact = serde_json::to_value(NodeDto {
        allocated: MAX_WIRE_INTEGER,
        apparent: MAX_WIRE_INTEGER - 1,
        ..node(1)
    })
    .unwrap();
    assert_eq!(exact["allocated"], max, "the safe maximum itself passes");
    assert_eq!(exact["apparent"], json!(9_007_199_254_740_990_u64));

    let totals = Totals {
        allocated: u64::MAX,
        apparent: u64::MAX,
        files: u64::MAX,
        dirs: u64::MAX,
        errors: u64::MAX,
    };
    let wire = serde_json::to_value(totals).unwrap();
    for field in ["allocated", "apparent", "files", "dirs", "errors"] {
        assert_eq!(wire[field], max, "totals.{field}");
    }

    let volume = serde_json::to_value(VolumeDto {
        id: "/".into(),
        mount_point: "/".into(),
        label: "Root".into(),
        device: "d".into(),
        fs_type: "ext4".into(),
        total: u64::MAX,
        available: u64::MAX,
        removable: false,
    })
    .unwrap();
    assert_eq!((&volume["total"], &volume["available"]), (&max, &max));

    let progress = serde_json::to_value(ScanEvent::Progress {
        files: u64::MAX,
        dirs: u64::MAX,
        bytes: u64::MAX,
        errors: u64::MAX,
        current_path: "/".into(),
    })
    .unwrap();
    for field in ["files", "dirs", "bytes", "errors"] {
        assert_eq!(progress[field], max, "progress.{field}");
    }

    let stats = serde_json::to_value(CategoryStat {
        category: Category::Video,
        allocated: u64::MAX,
        files: u64::MAX,
    })
    .unwrap();
    assert_eq!((&stats["allocated"], &stats["files"]), (&max, &max));
    let page = serde_json::to_value(ChildrenPage {
        parent_id: NodeId(0),
        total: u64::MAX,
        items: vec![],
    })
    .unwrap();
    assert_eq!(page["total"], max);
}

fn suggestion(verdict: Verdict) -> SuggestionDto {
    SuggestionDto {
        id: "pacman-cache".into(),
        title: "Pacman package cache".into(),
        source: SuggestionSource::Rule,
        path: "/var/cache/pacman/pkg".into(),
        verdict,
        reason: "Old package files.".into(),
        allocated: 36,
        items: 1,
        shared_files: 2,
        nodes: vec![NodeId(7), NodeId(3)],
    }
}

#[test]
fn verdicts_are_lowercase_and_sort_safe_first() {
    for (verdict, name) in [
        (Verdict::Safe, "safe"),
        (Verdict::Review, "review"),
        (Verdict::Keep, "keep"),
    ] {
        assert_wire(&verdict, json!(name));
    }

    let mut verdicts = vec![Verdict::Keep, Verdict::Safe, Verdict::Review];
    verdicts.sort();
    assert_eq!(verdicts, [Verdict::Safe, Verdict::Review, Verdict::Keep]);
}

#[test]
fn suggestions() {
    let dto = SuggestionsDto {
        suggestions: vec![suggestion(Verdict::Safe)],
        totals: VerdictTotals {
            safe: 36,
            review: 0,
            keep: 5,
        },
    };

    assert_wire(
        &dto,
        json!({
            "suggestions": [{
                "id": "pacman-cache", "title": "Pacman package cache", "source": "rule", "path": "/var/cache/pacman/pkg",
                "verdict": "safe", "reason": "Old package files.", "allocated": 36, "items": 1,
                "sharedFiles": 2, "nodes": [7, 3]
            }],
            "totals": { "safe": 36, "review": 0, "keep": 5 }
        }),
    );
}

#[test]
fn suggestion_sources_are_lowercase_and_a_missing_source_counts_as_a_rule() {
    for (source, name) in [
        (SuggestionSource::Rule, "rule"),
        (SuggestionSource::User, "user"),
        (SuggestionSource::Model, "model"),
    ] {
        assert_wire(&source, json!(name));
    }

    let old: SuggestionDto = serde_json::from_value(json!({
        "id": "pacman-cache", "title": "Pacman package cache", "path": "/var/cache/pacman/pkg",
        "verdict": "safe", "reason": "Old package files.", "allocated": 36, "items": 1,
        "sharedFiles": 2, "nodes": [7, 3]
    }))
    .unwrap();
    assert_eq!(old, suggestion(Verdict::Safe));
}

#[test]
fn suggestion_sizes_and_counts_are_clamped_on_the_wire() {
    let max = json!(MAX_WIRE_INTEGER);
    let huge = SuggestionDto {
        allocated: u64::MAX,
        items: u64::MAX,
        shared_files: u64::MAX,
        ..suggestion(Verdict::Review)
    };

    let wire = serde_json::to_value(&huge).unwrap();
    assert_eq!((&wire["allocated"], &wire["items"]), (&max, &max));
    assert_eq!(wire["sharedFiles"], max);

    let totals = serde_json::to_value(VerdictTotals {
        safe: u64::MAX,
        review: u64::MAX,
        keep: u64::MAX,
    })
    .unwrap();
    assert_eq!(totals, json!({ "safe": max, "review": max, "keep": max }));
}

#[test]
fn the_operating_system_is_a_closed_lowercase_set() {
    assert_wire(&OsDto::Linux, json!("linux"));
    assert_wire(&OsDto::Windows, json!("windows"));
    assert_wire(&OsDto::Macos, json!("macos"));

    assert_eq!(OsDto::from(Os::Windows), OsDto::Windows);
    assert_eq!(OsDto::from(Os::MacOs), OsDto::Macos);
    assert_eq!(OsDto::from(Os::Linux), OsDto::Linux);
    assert_eq!(OsDto::from(Os::Other), OsDto::Linux);
}
