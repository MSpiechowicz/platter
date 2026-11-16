//! What the model is told about a folder, and how it is written down.
//!
//! [`FolderFacts`] holds exactly the fields the model may see (docs/architecture.md, "What the model
//! sees") and nothing else: names, sizes, counts, dates and the rules that matched nearby. It has no
//! field for file contents, so contents cannot reach a prompt. [`facts_json`] writes the block the prompt
//! puts in front of the model, in the shape the prompt was tuned on.
//!
//! Names are untrusted text. They are written as JSON in which every control, bidirectional, line or
//! paragraph separator and other invisible format character is escaped as `\uXXXX`: the JSON writer
//! alone leaves most of them as they are. So is `<`: the runtime reads the special tokens of the chat
//! format (`<|im_end|>`) out of the prompt text, so a folder named like one must not reach it raw.

use std::path::Path;

use platter_core::{Category, NodeId, NodeKind, Tree};
use serde::Serialize;

/// Items listed as the largest inside a folder.
const LARGEST_ITEMS: usize = 10;
/// Characters of one item name that the model sees.
const NAME_CHARS: usize = 64;
/// Rule names listed on each side.
const RULES_LISTED: usize = 8;
/// Kinds of file listed.
const KINDS_LISTED: usize = 3;
/// A kind that holds less of the folder's bytes than this share (in percent) is not listed.
const KIND_MIN_SHARE: u128 = 5;
const DAY: i64 = 86_400;

/// The rules that matched near a folder, told by the caller (this crate does not know the rules).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleContext {
    /// Rules that matched inside the folder, with the bytes each covers.
    pub inside: Vec<(String, u64)>,
    /// Rules that matched beside the folder.
    pub beside: Vec<String>,
}

/// How the folder's bytes divide by age, in percent. They add up to 100 (all 0 for an empty folder).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgeShares {
    /// Changed within 30 days before the scan.
    pub month: u8,
    /// Changed within a year, but not within 30 days.
    pub year: u8,
    /// Older.
    pub older: u8,
}

/// Everything the model may know about one folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderFacts {
    /// The folder's path, home written as `~`.
    pub path: String,
    /// Bytes on disk, the whole folder.
    pub size: u64,
    pub files: u64,
    /// Days between the newest change in the folder and the scan; `None` when not known.
    pub newest_days: Option<u64>,
    pub age_shares: AgeShares,
    /// Kinds of file by the bytes they hold, the biggest first.
    pub kinds: Vec<String>,
    /// Names (not paths) of the largest items directly inside, at most 10, each cut to 64 characters.
    pub largest_items: Vec<String>,
    pub rules_inside: Vec<(String, u64)>,
    pub rules_beside: Vec<String>,
}

/// Gathers the facts of `node`. File contents are never read: only the scanned tree is used. Ages count
/// from the time of the scan, so the same tree always gives the same facts.
pub fn facts(tree: &Tree, home: Option<&Path>, node: NodeId, ctx: RuleContext) -> FolderFacts {
    let scanned_at = tree.scanned_at();
    let mut walk = Walk::default();
    walk.visit(tree, node, scanned_at);

    let newest = tree.mtime(node);
    let newest_days = (newest > 0).then(|| (scanned_at - newest).max(0) as u64 / DAY as u64);

    FolderFacts {
        path: display_path(&tree.path_of(node), home),
        size: tree.allocated(node),
        files: tree.files(node),
        newest_days,
        age_shares: walk.age_shares(),
        kinds: walk.kinds(),
        largest_items: largest_items(tree, node),
        rules_inside: ctx
            .inside
            .into_iter()
            .take(RULES_LISTED)
            .map(|(name, bytes)| (cut(&name), bytes))
            .collect(),
        rules_beside: ctx
            .beside
            .into_iter()
            .take(RULES_LISTED)
            .map(|name| cut(&name))
            .collect(),
    }
}

/// The path with `home` written as `~`. An unknown home, or one that is the root, changes nothing.
fn display_path(path: &Path, home: Option<&Path>) -> String {
    let shortened = home
        .filter(|home| home.has_root() && home.parent().is_some())
        .and_then(|home| path.strip_prefix(home).ok());

    match shortened {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.to_string_lossy()),
        None => path.to_string_lossy().into_owned(),
    }
}

fn cut(name: &str) -> String {
    name.chars().take(NAME_CHARS).collect()
}

fn largest_items(tree: &Tree, node: NodeId) -> Vec<String> {
    let mut children: Vec<NodeId> = tree.children(node).collect();
    children.sort_by(|a, b| {
        tree.allocated(*b)
            .cmp(&tree.allocated(*a))
            .then_with(|| tree.name(*a).cmp(tree.name(*b)))
    });

    children
        .into_iter()
        .take(LARGEST_ITEMS)
        .map(|child| cut(&tree.name_lossy(child)))
        .collect()
}

/// What one pass over a folder's files adds up to.
#[derive(Default)]
struct Walk {
    bytes_by_kind: [u64; Category::COUNT],
    files_by_kind: [u64; Category::COUNT],
    bytes_by_age: [u64; 3],
}

impl Walk {
    fn visit(&mut self, tree: &Tree, root: NodeId, scanned_at: i64) {
        let mut stack = vec![root];

        while let Some(id) = stack.pop() {
            let kind = tree.kind(id);

            if kind == NodeKind::Dir {
                stack.extend(tree.children(id));
            } else if kind.is_file_like() {
                self.add_file(tree, id, scanned_at);
            }
        }
    }

    fn add_file(&mut self, tree: &Tree, id: NodeId, scanned_at: i64) {
        let bytes = tree.allocated(id);
        let category = tree.category(id).index();
        self.bytes_by_kind[category] += bytes;
        self.files_by_kind[category] += 1;

        let age = scanned_at - tree.mtime(id);
        let bucket = if age <= 30 * DAY {
            0
        } else if age <= 365 * DAY {
            1
        } else {
            2
        };
        self.bytes_by_age[bucket] += bytes;
    }

    fn age_shares(&self) -> AgeShares {
        let total: u64 = self.bytes_by_age.iter().sum();
        if total == 0 {
            return AgeShares::default();
        }

        let percent = |part: u64| ((part as f64 / total as f64) * 100.0).round() as u8;
        let month = percent(self.bytes_by_age[0]);
        let year = percent(self.bytes_by_age[1]).min(100 - month);

        AgeShares {
            month,
            year,
            older: 100 - month - year,
        }
    }

    /// The kinds that hold at least 5 % of the bytes (of the files, when no file has any size), the
    /// biggest first, at most three.
    fn kinds(&self) -> Vec<String> {
        let by_bytes: u64 = self.bytes_by_kind.iter().sum();
        let weights = if by_bytes > 0 {
            &self.bytes_by_kind
        } else {
            &self.files_by_kind
        };
        let total: u64 = weights.iter().sum();

        let mut found: Vec<(u64, Category)> = Category::ALL
            .iter()
            .map(|category| (weights[category.index()], *category))
            .filter(|(weight, _)| {
                *weight > 0 && u128::from(*weight) * 100 >= u128::from(total) * KIND_MIN_SHARE
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

        found
            .into_iter()
            .take(KINDS_LISTED)
            .map(|(_, category)| kind_name(category).to_owned())
            .collect()
    }
}

/// How a category is called in the prompt.
fn kind_name(category: Category) -> &'static str {
    match category {
        Category::Video => "video",
        Category::Image => "images",
        Category::Audio => "audio",
        Category::Document => "documents",
        Category::Archive => "archives",
        Category::Code => "code",
        Category::Build => "build output",
        Category::Package => "packages",
        Category::App => "apps",
        Category::DiskImage => "disk images",
        Category::Data => "data",
        Category::Other => "other",
    }
}

/// The block the prompt shows the model: one line of JSON with these keys in this order (the order the
/// prompt was tuned on), names escaped as described at the top of this file.
///
/// `rules_nearby` lists the rules inside the folder (`"<rule> inside"`) and beside it (`"<rule>
/// beside"`); it is empty when none matched.
pub fn facts_json(facts: &FolderFacts) -> String {
    let nearby = facts
        .rules_inside
        .iter()
        .map(|(name, _)| format!("{name} inside"))
        .chain(facts.rules_beside.iter().map(|name| format!("{name} beside")))
        .collect();

    let block = Block {
        files: facts.files,
        kinds: &facts.kinds,
        largest_items: &facts.largest_items,
        newest: newest_text(facts.newest_days),
        path: &facts.path,
        rules_nearby: nearby,
        size: size_text(facts.size),
    };
    let json = serde_json::to_string(&block).expect("plain strings and numbers always serialize");

    escape_unsafe(&json)
}

/// The fields in the order they are written (the derive keeps declaration order; the prompt was tuned
/// on alphabetical keys, which is what this order is).
#[derive(Serialize)]
struct Block<'a> {
    files: u64,
    kinds: &'a [String],
    largest_items: &'a [String],
    newest: String,
    path: &'a str,
    rules_nearby: Vec<String>,
    size: String,
}

fn newest_text(days: Option<u64>) -> String {
    match days {
        None => "unknown".to_owned(),
        Some(0) => "today".to_owned(),
        Some(1) => "1 day ago".to_owned(),
        Some(days) if days < 365 => format!("{days} days ago"),
        Some(days) => match days / 365 {
            1 => "1 year ago".to_owned(),
            years => format!("{years} years ago"),
        },
    }
}

/// `700 MiB`, `3.2 GiB`, `6 GiB`: at most two figures, binary units.
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut unit = 1;
    while unit < UNITS.len() - 1 && bytes >= 1u64 << (10 * (unit + 1)) {
        unit += 1;
    }
    let mut value = bytes as f64 / (1u64 << (10 * unit)) as f64;
    let mut rounded = round_to_two_figures(value);
    if rounded >= 1024.0 && unit < UNITS.len() - 1 {
        unit += 1;
        value = bytes as f64 / (1u64 << (10 * unit)) as f64;
        rounded = round_to_two_figures(value);
    }

    if rounded.fract() == 0.0 {
        format!("{rounded:.0} {}", UNITS[unit])
    } else {
        format!("{rounded:.1} {}", UNITS[unit])
    }
}

fn round_to_two_figures(value: f64) -> f64 {
    if value < 10.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    }
}

/// Whether `c` is a character that may not appear raw in text shown to the model or the user: control
/// characters, line and paragraph separators, bidirectional controls and the other invisible format
/// characters.
pub(crate) fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c as u32,
            0xAD
                | 0x600..=0x605
                | 0x61C
                | 0x6DD
                | 0x70F
                | 0x8E2
                | 0x180E
                | 0x200B..=0x200F
                | 0x2028
                | 0x2029
                | 0x202A..=0x202E
                | 0x2060..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0x110BD
                | 0x110CD
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0001
                | 0xE0020..=0xE007F
        )
}

/// Writes every invisible character and every `<` of a JSON text as `\uXXXX` (a pair for one beyond the
/// first plane). Only characters inside strings can be among them; the JSON's own syntax has no `<`.
pub(crate) fn escape_unsafe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());

    for c in json.chars() {
        if c != '<' && !is_invisible(c) {
            out.push(c);
            continue;
        }

        let mut units = [0u16; 2];
        for unit in c.encode_utf16(&mut units) {
            out.push_str(&format!("\\u{unit:04x}"));
        }
    }
    out
}
