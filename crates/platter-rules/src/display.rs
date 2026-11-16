//! The path or pattern a suggestion shows. A home directory is always written `~`.

use std::path::Path;

/// `/home/me/code` becomes `~/code` and the home directory itself `~`; other paths are kept.
pub(crate) fn shorten_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        return if rest.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", rest.to_string_lossy())
        };
    }

    path.to_string_lossy().into_owned()
}

/// `base` followed by `/tail`, without doubling the slash after a base that already ends in one.
pub(crate) fn join(base: &str, tail: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{tail}")
    } else {
        format!("{base}/{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_written_as_a_tilde() {
        let home = Some(Path::new("/home/me"));

        assert_eq!(shorten_home(Path::new("/home/me"), home), "~");
        assert_eq!(shorten_home(Path::new("/home/me/code/x"), home), "~/code/x");
        assert_eq!(shorten_home(Path::new("/home/meow/x"), home), "/home/meow/x");
        assert_eq!(shorten_home(Path::new("/srv/x"), home), "/srv/x");
        assert_eq!(shorten_home(Path::new("/home/me/x"), None), "/home/me/x");
    }

    #[test]
    fn joining_keeps_a_single_slash() {
        assert_eq!(join("/", "**"), "/**");
        assert_eq!(join("~/code", "**/target"), "~/code/**/target");
    }
}
