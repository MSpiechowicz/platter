# Platter architecture

This file is the contract between the crates and between backend and frontend. Change it first, then the code.

## Layout

```
crates/platter-core      domain: model, scanner, queries, ports. No OS, no Tauri, no I/O of its own.
crates/platter-platform  OS adapters that implement the core ports (Linux reader, generic std::fs reader, volumes).
crates/platter-rules     cleanup rules: labels what could be cleaned. Read-only.
crates/platter-cleanup   cleanup plan, guard and runner: verified targets from selected rules, then deleting, emptying and trashing them.
crates/platter-cli       headless binary `platter-cli` (scan, volumes, suggest, clean, export-fixture).
apps/desktop             Svelte 5 + TypeScript + Vite frontend.
apps/desktop/src-tauri   Tauri 2 crate `platter-desktop` (binary `platter`): thin IPC layer over core + platform.
docs/mockups             Accepted HTML mockups. They are the visual reference, not code to copy wholesale.
```

`crates/platter-rules` holds the cleanup rules and only labels what could be cleaned. `crates/platter-cleanup` turns selected rules into a plan of verified targets and carries it out; running the tools that own a cache arrives with C3. `crates/platter-ai` (advisor model) comes later; do not create it or a stub for it before then.

## Dependency rules

- `platter-core` depends on no other workspace crate and on nothing OS-specific (`std::os::*`, rustix, sysinfo, tauri are forbidden there).
- `platter-platform` depends on `platter-core`. All `cfg(target_os)` code and all `std::os::unix` use live here.
- `platter-rules` depends on `platter-core` only (`Environment` and `Os` live in core so that both rules and platform can depend on core alone). It never touches the filesystem: it reads a finished `Tree` plus an `Environment` value (home directory, the user's personal folders, scan time) supplied by the caller. The platform crate fills the personal folders (desktop, documents, music, pictures, videos) from XDG `user-dirs.dirs`, so a localised `~/Dokumenty` is known; a folder not listed there is assumed under its English name. A home of `/` counts as unknown.
- `platter-cleanup` depends on `platter-core` and `platter-rules`. It looks at the disk only through the `EntryProbe` port and acts on it only through the `EntryRemover` port.
- `platter-cli` and `platter-desktop` depend on all of them and contain no domain logic.
- The frontend talks to the backend only through `src/lib/api` (one interface, two implementations: Tauri and mock).
- `unsafe_code = "forbid"` for the whole workspace.

## Ports (traits in `platter-core::ports`)

Only what truly differs per OS is a port.

- `DirSource`: handle-based directory reading, so Linux can do fd-relative `statx`.
  - `open_root(path, SourceOptions { one_file_system }) -> RootInfo { handle, meta }`
  - `read_dir(&handle, &mut Lease) -> io::Result<DirListing { entries, errors }>`. The source calls `lease.admit(name_len, is_dir)` before it keeps an entry and stops listing when that is refused. The lease reserves room from one scan-wide budget, so parallel listings cannot overshoot it.
  - A mount root that is not entered carries a `SkipReason` (`otherDisk`, `system`, `duplicate`, `unknown`). A failed directory read is the `Err`; entries whose metadata could not be read are counted in `errors`.
  - Each `RawEntry` is `{ name, meta, child }`: name bytes, an `EntryMeta` (kind, allocated bytes, apparent bytes, mtime in unix seconds, dev, ino, nlink, `is_mount_root`) and, for directories, a child handle. `child: None` on a directory means "do not descend".
  - `Handle` is an associated type, so the walker is generic and `Platform` picks one concrete reader per OS at compile time.
  - Must be `Send + Sync`; never follows symlinks below the root.
- `VolumeProvider`: `list() -> Vec<Volume>`.
- `EntryProbe`: `probe(path_bytes) -> io::Result<Vec<ProbedEntry { meta, mount_id }>>`, one entry per component below `/`. Each component is looked at below the previous one by descriptor (Linux: `openat(O_PATH | O_NOFOLLOW)` then `statx` of the descriptor), never following a symlink; a non-directory on the way fails with `NotADirectory`, `.`/`..`/empty components with `InvalidInput`. `meta.is_mount_root` is set on every component that starts a mount.
- `EntryRemover`: `remove(path, ExpectedEntry, Whole | Contents) -> Removed { entries, skipped_mounts, errors }`, `trash(path, ExpectedEntry, trash_dir) -> path in the trash`, `free_bytes(path)`. `ExpectedEntry` is what the plan saw (device, inode, mount id, file or folder, scan root depth). Linux: the parent is reached by `O_PATH | O_NOFOLLOW` descriptors from `/` (no mount below the scan root), the target is `statx`ed without following and compared; a folder is opened with `O_NOFOLLOW`, compared again by descriptor, and emptied through descriptors (names read first, each entry described again, `unlinkat`); folders of another mount inside are left alone and counted; a read-only folder is made writable for its owner when a removal is refused; problems inside a target are listed, not fatal. Trash: freedesktop layout under `$XDG_DATA_HOME/Trash` (default `~/.local/share/Trash`), `.trashinfo` created with `O_EXCL` first, the entry moved with `renameat(RENAME_NOREPLACE)`; an entry on another filesystem goes to that filesystem's `$topdir/.Trash-$uid` (created 0700; refused unless owned by the user and closed to group and others; `Path=` relative to the top folder). The shared `$topdir/.Trash/$uid` is not used, and nothing is ever copied between filesystems. `DeletionDate` is local time, from the system's TZif files (`platter_platform::localtime`; `TZ` may be a zone name, a file or a POSIX rule; files are read only when regular, without blocking, at most 1 MiB). The home trash must belong to the user. Names too long for a `.trashinfo` are shortened (UTF-8 kept whole). A folder of a read-only directory is made writable for its owner only on `EACCES`, and its own mode is restored afterwards.
- `ToolRunner`: `run(ToolCommand { program, args, elevated }) -> Result<(), ToolFailure>` with `NotInstalled`, `NotAuthorized`, `Failed { code, message }`, `Io`. Linux: the program is looked up only in `/usr/bin`, `/usr/sbin`, `/bin`, `/sbin`, and accepted only as a root-owned executable regular file that group and others cannot change, with every folder above it the same; its resolved path is what runs (under its own name as `argv[0]`); never through `PATH`. `NoElevation` when there is no `pkexec`; 126/127 mean a refusal only when `pkexec` itself says so. `Tool` values exist only in `action_for`; Docker always uses `--context default` (the local daemon). It runs with a cleared environment (`PATH` fixed, `LC_ALL=C`), no input. Elevated: `pkexec <absolute program> <args>`, `pkexec` found the same way; exit 126 or 127 means the password was not given.

The walker, the tree, aggregation and all queries are in core and are tested against an in-memory fake `DirSource`.

Adding Windows or macOS means: a faster `DirSource` for that OS (the generic `std::fs` one already works there), plus that OS's default exclusions. Nothing in core, CLI, IPC or UI changes.

## Scanner rules

- Never follow symlinks. A symlink counts its own size only.
- Hardlinks: a file with `nlink > 1` is counted once per `(dev, ino)`.
- Two sizes everywhere: `allocated` (blocks on disk) and `apparent` (file length). Directories add their blocks to `allocated` and nothing to `apparent`, which matches `du`.
- Mount boundaries are decided by the `DirSource`, not the walker: a mount root that must not be entered is returned with `child: None` and becomes a node of kind `mount` with size 0.
  - `one_file_system` means "same disk". On Linux a mount below the scan root is entered when it has the same superblock device, filesystem type and source as the scan root's mount (btrfs subvolumes mounted at `/home`, `/var/log`, ...). Other disks, tmpfs, network and pseudo filesystems are not entered.
  - Nothing is counted twice: a same-disk mount is refused when the part of the filesystem it shows equals, contains or lies inside an area already covered. The rule is documented in `platter-platform/src/linux/same_fs.rs`.
  - Mount roots are detected with `STATX_ATTR_MOUNT_ROOT` and mount ids, not by comparing `st_dev` (btrfs subvolumes each have their own `st_dev`). The generic reader can only compare `st_dev`, so it stops at every subvolume.
  - Excluded filesystem types are never entered, with or without `one_file_system`.
- A directory that cannot be read is counted in `errors` and kept as an empty node; it never aborts the scan.
- Names are stored as raw bytes and converted to lossy UTF-8 only when a DTO is built.
- Automounts are never triggered (`AT_NO_AUTOMOUNT`), and a mount that was not present when the scan started is not descended.
- A directory that is its own ancestor (same device and inode) is not descended and counts as an error. A scan stops with `TooLarge` when it would exceed its memory budget (half of the free memory; each entry is estimated at 208 bytes plus twice its name, plus 320 for a directory), the entry cap (`--max-entries`, otherwise the 32-bit id space) or 4 GiB of names. Room is reserved before an entry is kept, so the limits hold with any number of worker threads.
- Every opened directory is checked against the device, inode and mount id seen when it was listed; on a mismatch it is not read and counts as an error.
- Sizes are added with saturating arithmetic. `mtime` in DTOs is clamped to the years 1970 to 9999.
- Cancellation is an atomic flag checked per directory. Progress is throttled to about 10 per second. The `bytes` in a progress event is a lower bound (hardlinked files are left out until the end); the UI shows `finished.totals` once the scan is done.
- The scan root is resolved once; the scan fails if the folder opened is not the one that was resolved.
- Parallel forking is bounded so one worker stack never holds more than 2048 directory frames.
- Tree: arena with `u32` node ids, parents before children, children of one directory contiguous, so aggregation is one reverse pass.

## IPC contract

Scans live in the backend and are addressed by `ScanId`. The UI never receives a whole tree. All sizes are bytes as JSON numbers (safe up to 9 PB). Field names are camelCase on the wire.

### Commands

| Command | Arguments | Returns |
|---|---|---|
| `list_volumes` | none | `VolumeDto[]` |
| `start_scan` | `options: ScanOptions`, `onEvent: Channel<ScanEvent>` | `ScanId` |
| `cancel_scan` | `scan: ScanId` | nothing |
| `drop_scan` | `scan: ScanId` | nothing (frees the tree) |
| `get_node` | `scan`, `node: NodeId` | `NodeDto` |
| `get_children` | `scan`, `node`, `offset`, `limit`, `sort: SortKey`, `query?: string` | `ChildrenPage` |
| `get_lod` | `scan`, `node`, `maxNodes`, `maxDepth` | `LodNode` |
| `get_path` | `scan`, `node` | `PathDto` |
| `get_summary` | `scan`, `node` | `SummaryDto` |
| `get_suggestions` | `scan` | `SuggestionsDto` |
| `open_in_file_manager` | `scan`, `node` | nothing |
| `pick_folder` | none | `string \| null` (system folder dialog; `null` when dismissed; `invalid` when the folder name is not valid UTF-8) |
| `open_external` | `link: ExternalLink` | nothing (opens one of the fixed project pages in the browser; the UI cannot pass a URL) |
| `plan_cleanup` | `scan`, `suggestions: string[]` (rule ids) | `CleanupPlanDto` |
| `start_cleanup` | `plan: PlanId`, `onEvent: Channel<CleanupEvent>` | `CleanupId` |
| `cancel_cleanup` | `cleanup: CleanupId` | nothing |
| `undo_cleanup` | `cleanup: CleanupId` | `UndoDto[]` |

`start_scan` accepts only a root the backend offered: a mount point from `list_volumes` or a folder returned by `pick_folder` (`invalid` otherwise), so the web view cannot name an arbitrary folder to scan and clean. A picked folder is pinned by identity (device, inode) when it is picked; a scan finding another folder there (a swap for a link) is `invalid`.

Cleanup (C4): plans live in the backend. `plan_cleanup` takes 1 to 64 known rule ids (`invalid` otherwise) of a finished scan that is not outdated (`invalid`), one plan being made at a time (`invalid` while another is made), and returns a description with a `planId` (at most 200 refusals, `refusedMore` counts the rest). `start_cleanup` checks, in order: no cleanup running (`invalid`, the plan stays usable), the plan exists and was not used (`notFound`), no confirmation was declined in the last 2 seconds (`invalid`, the plan stays usable), then takes the plan (used once) and checks its age (10 minutes, `invalid`), that it has something to do (`invalid` for an empty plan) and that its scan still exists and is not outdated (`invalid`). Every plan is then confirmed in a native dialog the backend builds from its own plan, moving to the trash included (`needsConfirmation` is true for every plan with groups): first one line per group (action, rule, count, size; the trash with its scanned size; the exact tool command), then up to 20 paths, shown safely (control characters, invisible formatting characters and line or paragraph separators written as `\u{...}`, shortened before escaping, names that are not valid text marked, long paths shortened in the middle, the whole text capped). Declining, or a dialog that fails, is `cancelled`. The plan and its scan are checked again after the dialog. The running slot is freed however the run ends. After a run (or an undo), every finished scan is outdated, and so is a scan that was running at that moment; the UI offers "Rescan" (`finished.outdatedScan`, also on `failed`). `undo_cleanup` puts back, once, what one of the last 4 runs moved to the trash (runs that trashed nothing are not kept; emptying the trash forgets them all): each entry is checked against what was trashed and against its `.trashinfo` record (`Path=` must name the original place), nothing is overwritten, and entries that could not be put back stay for another try. Quitting the app while a cleanup acts on the disk cancels it after the item in progress and waits up to 30 seconds; while its dialog is still open, quitting cancels it without waiting. `undo_cleanup` is `invalid` while a cleanup runs. A picked folder whose identity cannot be read is refused at the pick. The cleanup types are only ever sent, never read back.

`get_children` with a `query` returns only the children whose name contains it, ignoring case (Unicode lower case; a name that is not valid UTF-8 is compared as shown, with U+FFFD); `total` is then the number of matches. A blank `query` filters nothing; one longer than 1024 bytes is `invalid`. `drop_scan` and `cancel_scan` succeed for an unknown or already ended id. `limit` is clamped to 1000 and `maxNodes` to 5000. At most 4 scans run at once; only the most recent finished scan is kept.

Errors are returned as `ApiError { code, message }` with `code` one of `notFound`, `cancelled`, `io`, `invalid`.

### Types

```ts
type ScanId = number;
type ExternalLink = 'source' | 'issues' | 'license';
type NodeId = number;

interface VolumeDto { id: string; mountPoint: string; label: string; device: string; fsType: string; total: number; available: number; removable: boolean; }

interface ScanOptions { root: string; oneFileSystem: boolean; }

type ScanEvent =
  | { type: 'progress'; files: number; dirs: number; bytes: number; errors: number; currentPath: string }
  | { type: 'finished'; scanId: ScanId; rootId: NodeId; totals: Totals; elapsedMs: number; warnings?: 'mountTableUnavailable'[] }
  | { type: 'cancelled' }
  | { type: 'failed'; message: string };

interface Totals { allocated: number; apparent: number; files: number; dirs: number; errors: number; }

type NodeKind = 'dir' | 'file' | 'symlink' | 'mount' | 'other' | 'collapsed';
type Category = 'video' | 'image' | 'audio' | 'document' | 'archive' | 'code' | 'build' | 'package' | 'app' | 'diskImage' | 'data' | 'other';

interface NodeDto { id: NodeId; name: string; kind: NodeKind; allocated: number; apparent: number; files: number; hasChildren: boolean; category: Category; mtime: number; unreadable: boolean; skipped?: SkipReason; }
type SkipReason = 'otherDisk' | 'system' | 'duplicate' | 'unknown';   // only on kind 'mount'

type SortKey = 'sizeDesc' | 'sizeAsc' | 'nameAsc' | 'filesDesc' | 'mtimeDesc';
interface ChildrenPage { parentId: NodeId; total: number; items: NodeDto[]; }

interface LodNode { node: NodeDto; children: LodNode[]; }

interface PathDto { path: string; segments: { id: NodeId; name: string }[]; }

type Verdict = 'safe' | 'review' | 'keep';
interface SuggestionDto {
  id: string;            // rule id, stable, e.g. 'pacman-cache'
  title: string;         // 'Pacman package cache'
  path: string;          // display path or pattern, e.g. '/var/cache/pacman/pkg' or '~/Projekty/**/target'
  verdict: Verdict;
  reason: string;        // one or two plain sentences
  allocated: number;     // bytes this suggestion covers
  items: number;         // how many folders or files matched
  nodes: NodeId[];       // up to 20 largest matched nodes, largest first
  sharedFiles: number;   // files inside that have hard links; what is freed can differ from `allocated`
}
interface SuggestionsDto {
  suggestions: SuggestionDto[];                        // safe first, then review, then keep; largest first within each
  totals: { safe: number; review: number; keep: number };
}

type AgeBucket = 'week' | 'month' | 'quarter' | 'year' | 'twoYears' | 'older';
interface SummaryDto {
  node: NodeDto;
  byCategory: { category: Category; allocated: number; files: number }[];
  byAge: { bucket: AgeBucket; allocated: number; files: number }[];
  largestDirs: NodeDto[];
}
```

Notes:

- `get_lod` returns at most `maxNodes` nodes and at most `maxDepth` levels below `node`. It expands the largest directories first. Children that do not fit are merged into one synthetic child of kind `collapsed` named `N smaller items`, with id `0xFFFFFFFF` (never a real node). The synthetic child counts toward `maxNodes`.
- `category` of a directory: a well-known directory name (`node_modules`, `target`, `.cache`, ...) sets the category for its whole subtree; otherwise it is the category holding most of the allocated bytes below it.
- `mtime` is unix seconds; for a directory it is the newest of its own mtime and everything inside it.
- `byAge` buckets are by mtime relative to scan time: up to and including 7 days, 30 days, 90 days, 1 year, 2 years, then older. All six buckets are always present. `byCategory` lists only non-empty categories, largest first. Both add up exactly to the node's `allocated`.
- `largestDirs` is up to 8 directories, largest first, ties by id. It starts from the node's non-empty directory children (never mounts). A candidate with a child directory holding at least 80% of its bytes is replaced by its directory children, repeatedly, down to 6 levels. While fewer than 8 remain, the largest candidate with at least two non-empty directory children is replaced by them and the 80% rule is applied again.
- `unreadable` is true for a directory that could not be listed (no permission, a loop, or the depth limit). Its size is unknown, not zero. It is false for mounts that were skipped on purpose.
- `warnings` on `finished` is left out when empty. `mountTableUnavailable` means the mount table could not be read, so no mount was entered.
- All sizes and counts are clamped to 9007199254740991 on the wire.
- Categories come from one data table, `platter-core/src/model/category_table.rs`: extensions, size rules (a `.raw` over 256 MiB is a disk image), compound extensions, directory names that set the category of a whole subtree (`.git`, `Steam`, `.cache`, `node_modules`, ...), and build-output names (`target`, `build`, `dist`, ...) that only count next to a project marker such as `Cargo.toml` or `package.json`.
- DTOs carry names, never full paths; a path is fetched with `get_path` when it is shown.

Cleanup types (C4):

```ts
type PlanId = number; type CleanupId = number;
type CleanupActionKind = 'delete' | 'emptyFolder' | 'emptyTrash' | 'trash' | 'runTool';
interface PlanGroupDto { ruleId: string; title: string; verdict: Verdict; action: CleanupActionKind; items: number; allocated: number; command?: string; needsElevation: boolean; samplePaths: string[]; } // samplePaths: up to 20
interface RefusedDto { ruleId: string; path: string; reason: string; }
interface CleanupPlanDto { planId: PlanId; scanId: ScanId; groups: PlanGroupDto[]; refused: RefusedDto[]; refusedMore: number; notFound: string[]; allocated: number; needsConfirmation: boolean; }
type OutcomeKind = 'removed' | 'trashed' | 'byTool' | 'skipped' | 'failed';
interface OutcomeDto { kind: OutcomeKind; detail?: string; problems: number; } // detail: why skipped/failed, or the tool command
type CleanupEvent =
  | { type: 'groupStarted'; ruleId: string; targets: number }
  | { type: 'targetDone'; ruleId: string; path: string; allocated: number; outcome: OutcomeDto }
  | { type: 'groupFinished'; ruleId: string }
  | { type: 'finished'; cleanupId: CleanupId; outdatedScan: ScanId; cleaned: number; withProblems: number; freed?: number; cancelled: boolean; toolsNotRun: string[]; canUndo: boolean }
  | { type: 'failed'; message: string; outdatedScan: ScanId; canUndo: boolean; cleanupId: CleanupId };
interface UndoDto { path: string; restored: boolean; error?: string; }
```

## Suggestions (read-only in this milestone)

`get_suggestions` runs the rules over a finished scan. Nothing is deleted or changed.

- A rule is data: an id, a title, a verdict, a reason, and a matcher (a fixed path, a path under the home directory, one of the user's personal folders, or a directory name with conditions such as "next to a `Cargo.toml` file", "next to a lockfile", "not inside a folder called `.nvm`", "not below `/usr`" and "not modified for N days"). A marker next to a folder must be a regular file, not a link.
- A node is covered by at most one suggestion: the outermost matching rule wins, and nothing inside a matched node is matched again. So the totals never count a byte twice.
- `keep` rules exist so the UI can say what was deliberately left alone (for example a games library). A `keep` match also stops rules from matching inside it, and it wins over any other rule on the same folder, whatever the table order. A kept folder is never entered, even when part of it is unreadable. A scan that starts inside a kept folder gets no suggestions at all.
- `safe` is only given to what the tree really describes: a folder with an unreadable folder anywhere inside is never `safe` (it can still be `review`). Without a known home directory nothing can be kept, so name rules are at most `review`.
- Anything that can hold what cannot be downloaded or rebuilt again is `review`, not `safe`: `~/.cache`, `~/.m2/repository`, the system journal and the pacman cache.
- A rule that matches nothing produces no suggestion. Unreadable folders and skipped mounts are never suggested.
- Verdicts come from rules only. A later milestone lets a local model label folders no rule covers; it can never turn `keep` into something else.

## Cleanup plan and run (C1, C2)

`platter_cleanup::plan_cleanup(tree, env, rules, selected_rule_ids, probe) -> CleanupPlan` runs the rules again (`platter_rules::matches`, every match, not the 20 a suggestion lists), then checks every match of a selected rule with the guard. Nothing on disk changes.

- Actions per rule (`action_for`): `delete` (build output, `node_modules`, Python caches), `emptyFolder` (caches and package stores: the folder stays), `emptyTrash` (the trash: the contents of `files/`, `info/` and `expunged/` and the `directorysizes` cache go, the folders stay; a part that is a link, another mount or on another disk is refused), `trash` (old downloads, disk images, the Docker Desktop VM: the user's own data), `runTool` with fixed arguments (pacman `paccache -rk2`, `apt-get clean`, `dnf clean packages`, `journalctl --vacuum-time=2weeks`, core dumps `find /var/lib/systemd/coredump -xdev -mindepth 1 -delete`, all as administrator; `docker system prune --force`). `keep` rules and btrfs snapshots are not cleaned.
- The guard refuses, in this order: an unknown home directory (everything); a `keep` verdict; the scan root; a link, a mount node, or anything but a file or folder; a node that is or holds an unreadable folder; a path that is not below the scan root (compared component by component as exact bytes); a protected place; then, from the probe at plan time: a path that is gone or leads elsewhere, a link or a non-folder on the way, a mount root anywhere below the scan root, and a kind that differs from the scan.
- Protected places: `/`, the home folder, the personal folders and the system folders (`/usr`, `/var`, `/var/lib`, `/home`, ...) may be neither a target nor inside one (no target may hold them); nothing inside `~/.ssh`, `~/.gnupg`, `~/.password-store`, `~/.pki`, `~/.config`, `~/.local/share/keyrings`, `~/.mozilla`, `~/.thunderbird`, the personal folders, `/etc`, `/usr`, `/boot`, `/root` and the other system folders may be a target.
- Protected places are also compared by identity: at plan time each one is looked up on disk (device, inode), and a target is refused when it, or any folder on its way, is one of them, so another path to the home (a bind mount) does not get past them. A protected place that is itself a link (`~/.config -> dotfiles/config`) is protected where it really is: `EntryProbe::resolve` (canonical path) is used for protected places only, never for a target. The folders above the home and the personal folders are known by identity as exact places.
- Freshness: a file is acted on only with the modification time and length the scan saw (length not compared for a second hard link, whose bytes the scan charged elsewhere); a folder matched by a name rule only while the marker file next to it (`Cargo.toml`, `package.json` and a lockfile) is still a regular file. Both are checked at plan time and again right before acting. A folder's own age is not checked again (that needs a new scan).
- For a rule its tool cleans (`runTool`), a part Platter could not read does not refuse the target; every other check still applies, and the targets are only shown: the tool acts on its own fixed path.
- A `VerifiedTarget` can only be made by the guard. It records the exact path, file or folder, device, inode and mount id from the probe, and size and age from the scan. Acting code must check identity again right before it acts (C2).
- A target inside another target is refused (the rules never produce one; checked anyway).
- `platter_cleanup::execute(plan, Disk { probe, remover, trash_dir }, cancel, on_event) -> CleanupReport` carries a plan out. Every target is probed again and compared with the plan right before it is acted on, and the remover checks again as it acts; a changed target is skipped. Cancelling takes effect between targets. A failure on one target does not stop the others. Events: `GroupStarted`, `TargetDone`, `GroupFinished`. `freed` is free space after minus before on the scan root's filesystem, never the plan's sum. `runTool` groups run their tool once per rule through the `ToolRunner` (outcome `byTool`, or the failure); without a runner they are listed in `toolsNotRun`. Once the administrator password was not given, no further elevated tool runs in the same run (no second prompt); each needs its own prompt otherwise, until a shipped polkit policy with `auth_admin_keep` exists (packaging, D1).
- After a cleanup the scan is outdated: the backend marks it so, the UI says so and offers "Rescan"; no part of the tree is patched or rescanned on its own (decided 2026-11-16, C4).
- `platter-cli clean <path> (--safe | --rule ID ...) (--dry-run | --yes) [--verbose]`: `--dry-run` prints the plan; `--yes` prints it and carries it out; with neither it refuses. With `--yes` it exits with an error when any item could not be cleaned completely (errors inside, a mount left alone, skipped, failed), when nothing could be cleaned, or when a rule named with `--rule` found nothing. On btrfs both the "before" and the "after" free space are measured after `syncfs` (other filesystems are not synced); a run that only moved things to the trash says that nothing is freed until the trash is emptied.

## Frontend rules

- Svelte 5 runes. Shared state lives in `src/lib/state/*.svelte.ts` modules, not in stores.
- `src/lib/api/types.ts` mirrors the types above. `client.ts` defines the `Api` interface; `tauri.ts` and `mock.ts` implement it; `index.ts` picks one (Tauri when `window.__TAURI_INTERNALS__` exists, otherwise the mock). The app must run in a plain browser on the mock.
- Features are folders under `src/lib/features/<name>`; reusable pieces are under `src/lib/components`.
- Design tokens are CSS custom properties in `src/app.css`, taken from `docs/mockups/shared.css`. Flat styling: no gradients, no glows; one accent colour; green means "safe" only.

## Milestones

1. Scaffold, scan, Overview and Explorer on real data (this milestone).
2. Distribution: release workflow, AppImage/deb/rpm, installer script, updater (needs signing keys created by the maintainer).
3. More analysis and visualizations.
4. Cleanup: rules, plan, dry run, guarded execution, elevation.
5. Local AI advisor.
6. Windows and macOS builds and fast readers.
