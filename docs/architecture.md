# Platter architecture

This file is the contract between the crates and between backend and frontend. Change it first, then the code.

## Layout

```
crates/platter-core      domain: model, scanner, queries, ports. No OS, no Tauri, no I/O of its own.
crates/platter-platform  OS adapters that implement the core ports (Linux reader, generic std::fs reader, volumes).
crates/platter-cli       headless binary `platter-cli` (scan, volumes, export-fixture).
apps/desktop             Svelte 5 + TypeScript + Vite frontend.
apps/desktop/src-tauri   Tauri 2 crate `platter-desktop` (binary `platter`): thin IPC layer over core + platform.
docs/mockups             Accepted HTML mockups. They are the visual reference, not code to copy wholesale.
```

`crates/platter-rules` holds the cleanup rules. In this milestone it only labels what could be cleaned (read-only suggestions); deleting arrives with the Cleanup milestone. `crates/platter-ai` (advisor model) comes later; do not create it or a stub for it before then.

## Dependency rules

- `platter-core` depends on no other workspace crate and on nothing OS-specific (`std::os::*`, rustix, sysinfo, tauri are forbidden there).
- `platter-platform` depends on `platter-core`. All `cfg(target_os)` code and all `std::os::unix` use live here.
- `platter-rules` depends on `platter-core` only (`Environment` and `Os` live in core so that both rules and platform can depend on core alone). It never touches the filesystem: it reads a finished `Tree` plus an `Environment` value (home directory, scan time) supplied by the caller.
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
| `get_children` | `scan`, `node`, `offset`, `limit`, `sort: SortKey` | `ChildrenPage` |
| `get_lod` | `scan`, `node`, `maxNodes`, `maxDepth` | `LodNode` |
| `get_path` | `scan`, `node` | `PathDto` |
| `get_summary` | `scan`, `node` | `SummaryDto` |
| `get_suggestions` | `scan` | `SuggestionsDto` |
| `open_in_file_manager` | `scan`, `node` | nothing |
| `pick_folder` | none | `string \| null` (system folder dialog; `null` when dismissed; `invalid` when the folder name is not valid UTF-8) |
| `open_external` | `link: ExternalLink` | nothing (opens one of the fixed project pages in the browser; the UI cannot pass a URL) |

`drop_scan` and `cancel_scan` succeed for an unknown or already ended id. `limit` is clamped to 1000 and `maxNodes` to 5000. At most 4 scans run at once; only the most recent finished scan is kept.

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

## Suggestions (read-only in this milestone)

`get_suggestions` runs the rules over a finished scan. Nothing is deleted or changed.

- A rule is data: an id, a title, a verdict, a reason, and a matcher (a fixed path, a path under the home directory, or a directory name with conditions such as "next to `Cargo.toml`" and "not modified for N days").
- A node is covered by at most one suggestion: the outermost matching rule wins, and nothing inside a matched node is matched again. So the totals never count a byte twice.
- `keep` rules exist so the UI can say what was deliberately left alone (for example a games library). A `keep` match also stops rules from matching inside it.
- A rule that matches nothing produces no suggestion. Unreadable folders and skipped mounts are never suggested.
- Verdicts come from rules only. A later milestone lets a local model label folders no rule covers; it can never turn `keep` into something else.

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
