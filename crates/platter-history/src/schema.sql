-- History file, version 1. Sizes are bytes. `day` is the local date YYYY-MM-DD, `at` is Unix seconds.
-- A device or inode number is kept as the signed integer with the same 64 bits.

CREATE TABLE volume (
    id INTEGER PRIMARY KEY,
    mount_point TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL
);

CREATE TABLE usage_sample (
    volume_id INTEGER NOT NULL REFERENCES volume (id),
    day TEXT NOT NULL,
    at INTEGER NOT NULL,
    total INTEGER NOT NULL,
    used INTEGER NOT NULL,
    PRIMARY KEY (volume_id, day)
) WITHOUT ROWID;

CREATE TABLE scan (
    id INTEGER PRIMARY KEY,
    volume_id INTEGER NOT NULL REFERENCES volume (id),
    at INTEGER NOT NULL,
    allocated INTEGER NOT NULL,
    apparent INTEGER NOT NULL,
    files INTEGER NOT NULL,
    dirs INTEGER NOT NULL,
    errors INTEGER NOT NULL
);
CREATE INDEX scan_at ON scan (at);

CREATE TABLE cleanup (
    id INTEGER PRIMARY KEY,
    volume_id INTEGER NOT NULL REFERENCES volume (id),
    at INTEGER NOT NULL,
    freed_measured INTEGER,
    freed INTEGER NOT NULL,
    trashed INTEGER NOT NULL,
    cancelled INTEGER NOT NULL,
    emptied_trash INTEGER NOT NULL,
    what TEXT NOT NULL,
    methods TEXT NOT NULL
);
CREATE INDEX cleanup_volume_at ON cleanup (volume_id, at);

-- Only what a run moved to the trash.
CREATE TABLE cleanup_item (
    id INTEGER PRIMARY KEY,
    cleanup_id INTEGER NOT NULL REFERENCES cleanup (id) ON DELETE CASCADE,
    original BLOB NOT NULL,
    trashed BLOB NOT NULL,
    dev INTEGER NOT NULL,
    ino INTEGER NOT NULL,
    is_dir INTEGER NOT NULL,
    allocated INTEGER NOT NULL,
    restored_at INTEGER
);
CREATE INDEX cleanup_item_cleanup ON cleanup_item (cleanup_id);
