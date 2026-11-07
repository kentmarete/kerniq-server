//! Route path normalization.
//!
//! Axum 0.8 uses `{name}` for captures and `{*name}` for wildcards, and
//! rejects the older `:name` / `*name` syntax with a panic. Kerniq accepts
//! both styles and normalizes to Axum's at registration time, so there is no
//! per-request cost and no ambiguity (a segment starting with `:` or `*` can
//! never be a literal in Axum 0.8).
//!
//! | Written | Registered as |
//! |---|---|
//! | `/users/{id}` | `/users/{id}` |
//! | `/users/:id` | `/users/{id}` |
//! | `/files/*path` | `/files/{*path}` |
//! | `users` | `/users` |

use std::borrow::Cow;

/// Normalizes a route path: ensures a leading `/` and converts `:name` and
/// `*name` segments into `{name}` and `{*name}`.
pub fn normalize(path: &str) -> Cow<'_, str> {
    let needs_slash = !path.starts_with('/');
    let needs_rewrite = path
        .split('/')
        .any(|seg| seg.starts_with(':') || seg.starts_with('*'));
    if !needs_slash && !needs_rewrite {
        return Cow::Borrowed(path);
    }
    let mut out = String::with_capacity(path.len() + 4);
    if needs_slash {
        out.push('/');
    }
    for (i, seg) in path.split('/').enumerate() {
        if i > 0 {
            out.push('/');
        }
        if let Some(name) = seg.strip_prefix(':') {
            out.push('{');
            out.push_str(name);
            out.push('}');
        } else if let Some(name) = seg.strip_prefix('*') {
            out.push_str("{*");
            out.push_str(name);
            out.push('}');
        } else {
            out.push_str(seg);
        }
    }
    Cow::Owned(out)
}

/// Normalizes a mount prefix: leading `/`, no trailing `/`; the root becomes
/// the empty string.
pub fn normalize_prefix(prefix: &str) -> String {
    let normalized = normalize(prefix);
    normalized.trim_end_matches('/').to_owned()
}

/// Joins two normalized prefixes.
pub fn join_prefix(outer: &str, inner: &str) -> String {
    let mut joined = String::with_capacity(outer.len() + inner.len());
    joined.push_str(outer);
    joined.push_str(inner);
    joined
}

/// Joins a normalized prefix and a normalized route path, for display.
pub fn join_display(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_owned()
    } else if path == "/" {
        prefix.to_owned()
    } else {
        join_prefix(prefix, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_colon_and_star_syntax() {
        assert_eq!(normalize("/users/{id}"), "/users/{id}");
        assert!(matches!(normalize("/users/{id}"), Cow::Borrowed(_)));
        assert_eq!(normalize("/users/:id"), "/users/{id}");
        assert_eq!(normalize("/a/:x/b/:y"), "/a/{x}/b/{y}");
        assert_eq!(normalize("/files/*path"), "/files/{*path}");
        assert_eq!(normalize("users"), "/users");
        assert_eq!(normalize(""), "/");
        assert_eq!(normalize("/"), "/");
    }

    #[test]
    fn normalizes_prefixes() {
        assert_eq!(normalize_prefix("/api/v1/"), "/api/v1");
        assert_eq!(normalize_prefix("api"), "/api");
        assert_eq!(normalize_prefix("/"), "");
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(normalize_prefix("/tenants/:tenant"), "/tenants/{tenant}");
        assert_eq!(join_prefix("/api", "/v1"), "/api/v1");
        assert_eq!(join_display("/api", "/"), "/api");
        assert_eq!(join_display("", "/x"), "/x");
    }
}
