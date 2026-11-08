//! Installed font families from the platform font database (Core Text / DirectWrite / Fontconfig).
//! Only successful scans are cached. Restart the process to discover newly installed fonts.
use font_kit::source::SystemSource;
use std::sync::{LazyLock, Mutex, OnceLock};
use serde::Serialize;
use std::fmt;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct LocalFont {
    pub family: String,
    pub monospace: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ScanError {
    Unavailable,
    NoUsableFamilies,
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "local font catalogue unavailable",
            Self::NoUsableFamilies => "local font catalogue contains no usable families",
        })
    }
}

static CATALOGUE: OnceLock<Vec<LocalFont>> = OnceLock::new();
static SCAN_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Call from a blocking worker, never the renderer/event-loop thread. Concurrent first callers
/// share one initialization; a failed scan leaves the cache empty and returns an explicit error.
pub fn catalogue() -> Result<&'static [LocalFont], ScanError> {
    if let Some(fonts) = CATALOGUE.get() {
        return Ok(fonts);
    }
    let _scan = SCAN_LOCK.lock().map_err(|_| ScanError::Unavailable)?;
    if let Some(fonts) = CATALOGUE.get() {
        return Ok(fonts);
    }
    // Store only a completed successful scan; errors remain observable and can be retried.
    let fonts = scan()?;
    let _ = CATALOGUE.set(fonts);
    CATALOGUE.get().map(Vec::as_slice).ok_or(ScanError::Unavailable)
}

fn usable_family_name(family: &str) -> bool {
    // Dot-prefixed Core Text families are private system faces, unavailable as CSS local families.
    !family.is_empty()
        && !family.starts_with('.')
        && family.trim() == family
        && family.chars().count() <= 200
        && !family.chars().any(char::is_control)
}

fn classify_faces(faces: impl Iterator<Item = Option<bool>>) -> Option<bool> {
    let mut usable = false;
    let mut monospace = true;
    for face in faces {
        usable |= face.is_some();
        // Missing, mismatched or proportional faces cannot establish a fixed-pitch family.
        monospace &= face == Some(true);
    }
    usable.then_some(monospace)
}

fn scan() -> Result<Vec<LocalFont>, ScanError> {
    let source = SystemSource::new();
    let mut families = source.all_families().map_err(|_| ScanError::Unavailable)?;
    families.sort_unstable();
    families.dedup();
    let mut catalogue = Vec::with_capacity(families.len());
    for family in families {
        if !usable_family_name(&family) {
            continue;
        }
        let Ok(handles) = source.select_family_by_name(&family) else {
            // A font can disappear or be disabled between enumeration and selection.
            continue;
        };
        let monospace = classify_faces(handles.fonts().iter().map(|handle| {
            let font = handle.load().ok()?;
            // Native fixed-pitch metadata, never a guess based on the family name.
            (font.family_name() == family).then(|| font.is_monospace())
        }));
        if let Some(monospace) = monospace {
            catalogue.push(LocalFont { family, monospace });
        }
    }
    if catalogue.is_empty() {
        return Err(ScanError::NoUsableFamilies);
    }
    Ok(catalogue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_names_are_persistable_real_family_names() {
        for family in ["Inter", "Source Sans 3", "苹方-简", "A \"quoted\" family", "A\\B"] {
            assert!(usable_family_name(family), "{family}");
        }
        assert!(usable_family_name(&"字".repeat(200)));
        for family in ["", " Inter", "Inter ", ".AppleSystemUIFont", "A\nB", "A\0B", "A\u{7f}B"] {
            assert!(!usable_family_name(family), "{family:?}");
        }
        assert!(!usable_family_name(&"字".repeat(201)));
    }

    #[test]
    fn fixed_pitch_requires_consistent_metadata_across_the_family() {
        assert_eq!(classify_faces([Some(true), Some(true)].into_iter()), Some(true));
        assert_eq!(classify_faces([Some(true), Some(false)].into_iter()), Some(false));
        assert_eq!(classify_faces([Some(true), None].into_iter()), Some(false));
        assert_eq!(classify_faces([None, None].into_iter()), None);
        assert_eq!(classify_faces([].into_iter()), None);
    }
}
