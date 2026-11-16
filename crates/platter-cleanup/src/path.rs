//! Paths as exact bytes, compared component by component.

/// The components of an absolute path. `None` for anything that is not plainly absolute: a relative
/// path, or one with `.`, `..` or an empty component inside (a trailing `/` is allowed).
pub(crate) fn components(path: &[u8]) -> Option<Vec<&[u8]>> {
    let rest = path.strip_prefix(b"/")?;
    let rest = rest.strip_suffix(b"/").unwrap_or(rest);
    if rest.is_empty() {
        return Some(Vec::new());
    }

    let parts: Vec<&[u8]> = rest.split(|&byte| byte == b'/').collect();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == b"." || *part == b"..")
    {
        return None;
    }

    Some(parts)
}

/// Owned components, for paths that are kept.
pub(crate) fn owned(path: &[u8]) -> Option<Vec<Vec<u8>>> {
    components(path).map(|parts| parts.into_iter().map(<[u8]>::to_vec).collect())
}

/// Whether `inner` is `outer` or lies below it.
pub(crate) fn is_within<A: AsRef<[u8]>, B: AsRef<[u8]>>(inner: &[A], outer: &[B]) -> bool {
    inner.len() >= outer.len() && inner.iter().zip(outer).all(|(a, b)| a.as_ref() == b.as_ref())
}

/// A path for people to read (lossy for names that are not valid UTF-8).
pub(crate) fn display(path: &[u8]) -> String {
    String::from_utf8_lossy(path).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_exact_and_refuse_anything_that_could_point_elsewhere() {
        assert_eq!(components(b"/").unwrap(), Vec::<&[u8]>::new());
        assert_eq!(components(b"/a/b/").unwrap(), [&b"a"[..], b"b"]);
        assert_eq!(components(b"/a/\xff").unwrap(), [&b"a"[..], b"\xff"]);
        for bad in [&b"a/b"[..], b"", b"/a/../b", b"/a/./b", b"/a//b"] {
            assert!(components(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn within_compares_whole_components() {
        let home = components(b"/home/u").unwrap();

        assert!(is_within(&components(b"/home/u").unwrap(), &home));
        assert!(is_within(&components(b"/home/u/x").unwrap(), &home));
        assert!(!is_within(&components(b"/home/user").unwrap(), &home));
        assert!(!is_within(&components(b"/home").unwrap(), &home));
    }
}
