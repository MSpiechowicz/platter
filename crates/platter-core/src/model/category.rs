use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use super::category_table::{
    COMPOUND_EXTENSIONS, DIRECTORY_NAMES, EXTENSIONS, PROJECT_DIRECTORY_NAMES, PROJECT_MARKERS, SIZE_RULES,
    SizeRule,
};

/// Coarse content category used for colouring and the by-category summary. Wire names are camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Video,
    Image,
    Audio,
    Document,
    Archive,
    Code,
    Build,
    Package,
    App,
    DiskImage,
    /// Databases, datasets and machine-learning models.
    Data,
    Other,
}

impl Category {
    /// Every category, in index order.
    pub const ALL: [Category; 12] = [
        Category::Video,
        Category::Image,
        Category::Audio,
        Category::Document,
        Category::Archive,
        Category::Code,
        Category::Build,
        Category::Package,
        Category::App,
        Category::DiskImage,
        Category::Data,
        Category::Other,
    ];

    pub const COUNT: usize = Self::ALL.len();

    pub const fn index(self) -> usize {
        self as usize
    }

    /// Category of a file with the given `size` in bytes (its apparent size), judged by its name's
    /// extension. Names without a usable extension are `Other`. The size only matters for the few
    /// extensions in `SIZE_RULES`, such as `.raw`.
    pub fn of_file(name: &[u8], size: u64) -> Category {
        if let Some(category) = compound_extension(name) {
            return category;
        }

        let Some(ext) = extension(name) else {
            return Category::Other;
        };

        let mut buf = [0u8; MAX_EXTENSION];
        let lowered = &mut buf[..ext.len()];
        lowered.copy_from_slice(ext);
        lowered.make_ascii_lowercase();

        let Ok(key) = std::str::from_utf8(lowered) else {
            return Category::Other;
        };
        match BY_EXTENSION.get(key) {
            Some(ExtensionRule::Fixed(category)) => *category,
            Some(ExtensionRule::BySize(rule)) if size > rule.above => rule.large,
            Some(ExtensionRule::BySize(rule)) => rule.small,
            None => Category::Other,
        }
    }

    /// Category the directory `name` (inside a directory called `parent`, if known) imposes on its whole
    /// subtree, if it matches a location rule.
    ///
    /// `project_like` says whether the directory sits next to a project marker; it is called at most
    /// once, and only when a rule that needs the answer matches by name.
    pub fn of_directory(
        parent: Option<&[u8]>,
        name: &[u8],
        project_like: &mut dyn FnMut() -> bool,
    ) -> Option<Category> {
        let name = std::str::from_utf8(name).ok()?;
        let parent = parent.and_then(|parent| std::str::from_utf8(parent).ok());
        let mut project: Option<bool> = None;

        let mut applies = |rule: &DirectoryRule| {
            let parent_matches = rule.parent.is_none_or(|wanted| parent == Some(wanted));
            let project_matches = !rule.project_only || *project.get_or_insert_with(&mut *project_like);

            parent_matches && project_matches
        };

        let exact = BY_DIRECTORY.exact.get(name).into_iter().flatten();
        let wild = BY_DIRECTORY
            .wild
            .iter()
            .filter(|(wild, _)| wild.matches(name))
            .map(|(_, rule)| rule);

        exact
            .chain(wild)
            .find(|rule| applies(rule))
            .map(|rule| rule.category)
    }

    /// Whether `name` marks the directory that contains it as a software project.
    pub fn is_project_marker(name: &[u8]) -> bool {
        PROJECT_MARKERS.iter().any(|marker| marker.as_bytes() == name)
    }
}

const MAX_EXTENSION: usize = 12;

#[derive(Clone, Copy)]
enum ExtensionRule {
    Fixed(Category),
    BySize(&'static SizeRule),
}

static BY_EXTENSION: LazyLock<HashMap<&'static str, ExtensionRule>> = LazyLock::new(|| {
    let fixed = EXTENSIONS.iter().flat_map(|(category, extensions)| {
        extensions
            .iter()
            .map(move |ext| (*ext, ExtensionRule::Fixed(*category)))
    });
    let by_size = SIZE_RULES
        .iter()
        .map(|rule| (rule.extension, ExtensionRule::BySize(rule)));

    fixed.chain(by_size).collect()
});

/// One location rule, parsed from a pattern of the directory tables.
struct DirectoryRule {
    category: Category,
    /// Name the parent directory must have.
    parent: Option<&'static str>,
    /// Applies only to project-like directories.
    project_only: bool,
}

/// The part of a pattern that is not an exact name.
enum Wild {
    Prefix(&'static str),
    Suffix(&'static str),
}

impl Wild {
    fn matches(&self, name: &str) -> bool {
        match self {
            Wild::Prefix(prefix) => name.starts_with(prefix),
            Wild::Suffix(suffix) => name.ends_with(suffix),
        }
    }
}

/// Location rules indexed for lookup. Within each group the table order decides which rule wins.
struct DirectoryRules {
    exact: HashMap<&'static str, Vec<DirectoryRule>>,
    wild: Vec<(Wild, DirectoryRule)>,
}

static BY_DIRECTORY: LazyLock<DirectoryRules> = LazyLock::new(|| {
    let mut rules = DirectoryRules {
        exact: HashMap::new(),
        wild: Vec::new(),
    };

    for (tables, project_only) in [(DIRECTORY_NAMES, false), (PROJECT_DIRECTORY_NAMES, true)] {
        for (category, patterns) in tables {
            for pattern in *patterns {
                let (parent, name) = match pattern.split_once('/') {
                    Some((parent, name)) => (Some(parent), name),
                    None => (None, *pattern),
                };
                let rule = DirectoryRule {
                    category: *category,
                    parent,
                    project_only,
                };

                if let Some(prefix) = name.strip_suffix('*') {
                    rules.wild.push((Wild::Prefix(prefix), rule));
                } else if let Some(suffix) = name.strip_prefix('*') {
                    rules.wild.push((Wild::Suffix(suffix), rule));
                } else {
                    rules.exact.entry(name).or_default().push(rule);
                }
            }
        }
    }
    rules
});

/// Category of a multi-part suffix such as `.pkg.tar.zst`, matched ignoring case.
fn compound_extension(name: &[u8]) -> Option<Category> {
    COMPOUND_EXTENSIONS.iter().find_map(|(category, suffixes)| {
        suffixes
            .iter()
            .any(|suffix| has_suffix_ignoring_case(name, suffix.as_bytes()))
            .then_some(*category)
    })
}

/// Whether `name` is some stem, a dot, and then `suffix` (the stem must not be empty).
fn has_suffix_ignoring_case(name: &[u8], suffix: &[u8]) -> bool {
    let Some(dot) = name.len().checked_sub(suffix.len() + 1) else {
        return false;
    };

    dot > 0 && name[dot] == b'.' && name[dot + 1..].eq_ignore_ascii_case(suffix)
}

/// The part after the last dot, unless the dot is the first or last byte or the part is too long.
fn extension(name: &[u8]) -> Option<&[u8]> {
    let dot = name.iter().rposition(|&b| b == b'.')?;
    let ext = &name[dot + 1..];

    if dot == 0 || ext.is_empty() || ext.len() > MAX_EXTENSION {
        return None;
    }
    Some(ext)
}

#[cfg(test)]
mod tests;
