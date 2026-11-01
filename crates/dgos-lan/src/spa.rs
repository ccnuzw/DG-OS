//! The remote SPA build (`dist-lan/`, technical design "安全与日志"): `index.html`
//! plus flat files under `assets/`. No directory listing, no user paths, no
//! traversal: an asset name is one path segment of `[A-Za-z0-9._-]`.

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SpaFiles {
    root: PathBuf,
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or_default() {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "json" => "application/json",
        "map" => "application/json",
        _ => "application/octet-stream",
    }
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl SpaFiles {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub async fn index(&self) -> Option<Vec<u8>> {
        tokio::fs::read(self.root.join("index.html")).await.ok()
    }

    /// `/assets/<name>` with its content type.
    pub async fn asset(&self, name: &str) -> Option<(Vec<u8>, &'static str)> {
        if !safe_name(name) {
            return None;
        }
        let path = self.root.join("assets").join(name);
        let meta = tokio::fs::symlink_metadata(&path).await.ok()?;
        if !meta.is_file() {
            return None;
        }
        Some((tokio::fs::read(path).await.ok()?, content_type(name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_flat_asset_names_are_served() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
        std::fs::write(dir.path().join("assets/app-1.js"), "x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "s").unwrap();
        let spa = SpaFiles::new(dir.path());
        assert_eq!(spa.index().await.as_deref(), Some(&b"<html></html>"[..]));
        let (body, kind) = spa.asset("app-1.js").await.unwrap();
        assert_eq!(
            (body.as_slice(), kind),
            (&b"x"[..], "text/javascript; charset=utf-8")
        );
        for name in [
            "../secret.txt",
            "..",
            ".hidden",
            "a/b.js",
            "",
            "%2e%2e",
            "app-1.js/",
        ] {
            assert!(spa.asset(name).await.is_none(), "{name}");
        }
        assert!(spa.asset("missing.js").await.is_none());
    }
}
