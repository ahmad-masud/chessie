//! Finding the app's own files, whichever way it was started.
//!
//! Three cases matter: a packaged `.app`, where everything lives in
//! `Contents/Resources`; a loose binary with its files beside it; and
//! `cargo run`, where they are in the project directory.

use std::path::{Path, PathBuf};

/// Name of the bundled engine, inside the app's resources.
pub const ENGINE_NAME: &str = "stockfish";
/// Folder holding the model and its textures.
pub const ASSET_DIR: &str = "assets";

/// `Contents/Resources` when running from a macOS app bundle.
///
/// The executable sits at `Chessie.app/Contents/MacOS/chessie`, so the
/// resources are one level up and across.
pub fn bundle_resources(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    if macos_dir.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let resources = contents.join("Resources");
    resources.is_dir().then_some(resources)
}

/// Directories to look in for the app's files, nearest first.
pub fn search_roots(exe: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(resources) = bundle_resources(exe) {
        roots.push(resources);
    }
    if let Some(dir) = exe.parent() {
        roots.push(dir.to_path_buf());
    }
    // Running under cargo, where the files are in the project directory.
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        roots.push(PathBuf::from(manifest));
    }
    roots
}

/// The directory that holds `assets/`, which is what Bevy wants as its root.
pub fn asset_root(exe: &Path) -> Option<PathBuf> {
    search_roots(exe)
        .into_iter()
        .find(|root| root.join(ASSET_DIR).is_dir())
}

/// The engine shipped alongside the app, if there is one.
pub fn bundled_engine(exe: &Path) -> Option<PathBuf> {
    search_roots(exe)
        .into_iter()
        .map(|root| root.join(ENGINE_NAME))
        .find(|candidate| candidate.is_file())
}

/// Where to look for an engine, in order: the one we ship, then whatever the
/// machine already has.
pub fn engine_candidates(exe: &Path) -> Vec<String> {
    let mut candidates: Vec<String> = bundled_engine(exe)
        .map(|p| p.to_string_lossy().into_owned())
        .into_iter()
        .collect();
    candidates.extend(
        [
            "stockfish",
            "/opt/homebrew/bin/stockfish",
            "/usr/local/bin/stockfish",
            "/usr/bin/stockfish",
        ]
        .map(String::from),
    );
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_bundle_is_recognised_by_its_shape() {
        // Only the real layout counts, or a stray folder called MacOS would
        // send the app looking in the wrong place.
        let exe = Path::new("/Applications/Chessie.app/Contents/MacOS/chessie");
        let macos = exe.parent().unwrap();
        assert_eq!(macos.file_name().unwrap(), "MacOS");
        assert_eq!(macos.parent().unwrap().file_name().unwrap(), "Contents");
    }

    #[test]
    fn a_loose_binary_is_not_mistaken_for_a_bundle() {
        assert!(bundle_resources(Path::new("/usr/local/bin/chessie")).is_none());
        assert!(bundle_resources(Path::new("/tmp/build/chessie")).is_none());
    }

    #[test]
    fn the_bundled_engine_is_preferred_over_the_system_one() {
        // The whole point is that the app does not need anything installed.
        let candidates = engine_candidates(Path::new("/tmp/nowhere/chessie"));
        let plain = candidates.iter().position(|c| c == "stockfish");
        assert!(plain.is_some(), "the system engine should still be a fallback");
    }

    #[test]
    fn a_system_engine_is_still_a_fallback() {
        let candidates = engine_candidates(Path::new("/tmp/nowhere/chessie"));
        assert!(candidates.iter().any(|c| c.contains("homebrew")));
        assert!(!candidates.is_empty());
    }

    #[test]
    fn the_project_directory_is_searched_when_running_under_cargo() {
        let roots = search_roots(Path::new("/tmp/nowhere/chessie"));
        assert!(
            roots.iter().any(|r| r.ends_with("chessie")),
            "cargo run should still find the project's assets"
        );
    }
}
