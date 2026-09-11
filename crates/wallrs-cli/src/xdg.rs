use std::path::{Path, PathBuf};

/// Returns the XDG data home directory (`$XDG_DATA_HOME` or `~/.local/share`).
pub fn data_home() -> PathBuf {
    if let Ok(val) = std::env::var("XDG_DATA_HOME")
        && !val.trim().is_empty()
    {
        return PathBuf::from(val);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local").join("share");
    }
    PathBuf::from(".")
}

/// Returns the list of XDG data directories (`$XDG_DATA_DIRS` or `["/usr/local/share", "/usr/share"]`).
pub fn data_dirs() -> Vec<PathBuf> {
    if let Ok(val) = std::env::var("XDG_DATA_DIRS") {
        let dirs: Vec<PathBuf> = val
            .split(':')
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .collect();
        if !dirs.is_empty() {
            return dirs;
        }
    }
    vec![
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]
}

/// Returns the default user wallpapers directory (`$XDG_DATA_HOME/wallrs/wallpapers`).
pub fn default_user_wallpapers_dir() -> PathBuf {
    data_home().join("wallrs").join("wallpapers")
}

/// Returns all standard directories where wallpapers can be located.
pub fn wallpaper_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![default_user_wallpapers_dir()];
    dirs.push(data_home().join("wallrs").join("examples"));

    // Working directory / repository fallback: ./examples, ../../examples
    dirs.push(PathBuf::from("examples"));
    dirs.push(PathBuf::from("../../examples"));

    // Workspace repository fallback during development and testing
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let p = Path::new(manifest_dir);
        if let Some(workspace_root) = p.parent().and_then(|p| p.parent()) {
            dirs.push(workspace_root.join("examples"));
        }
    }

    // Relative to current executable binary
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(bin_dir) = exe_path.parent()
    {
        dirs.push(bin_dir.join("examples"));
        dirs.push(bin_dir.join("..").join("examples"));
        dirs.push(bin_dir.join("..").join("..").join("examples"));
        dirs.push(bin_dir.join("..").join("..").join("..").join("examples"));
        dirs.push(
            bin_dir
                .join("..")
                .join("share")
                .join("wallrs")
                .join("examples"),
        );
        dirs.push(
            bin_dir
                .join("..")
                .join("share")
                .join("wallrs")
                .join("wallpapers"),
        );
    }

    for d in data_dirs() {
        dirs.push(d.join("wallrs").join("wallpapers"));
        dirs.push(d.join("wallrs").join("examples"));
    }

    dirs
}

/// Resolves a wallpaper manifest path from an input path or name.
///
/// If `input` points directly to an existing file or directory, it is returned.
/// Otherwise, standard XDG wallpaper directories are searched for matching folders or manifests.
pub fn resolve_wallpaper_path(input: &Path) -> Result<PathBuf, String> {
    let direct_manifest = if input.is_dir() {
        input.join("wallpaper.toml")
    } else {
        input.to_path_buf()
    };

    if direct_manifest.exists() {
        return direct_manifest.canonicalize().map_err(|e| {
            format!(
                "Failed to canonicalize manifest at {:?}: {}",
                direct_manifest, e
            )
        });
    }

    let input_str = input.to_string_lossy();
    if !input_str.contains('/') {
        for search_dir in wallpaper_search_dirs() {
            let candidate = search_dir.join(&*input_str);
            let candidate_manifest = candidate.join("wallpaper.toml");
            if candidate_manifest.exists() {
                return candidate_manifest.canonicalize().map_err(|e| {
                    format!(
                        "Failed to canonicalize manifest at {:?}: {}",
                        candidate_manifest, e
                    )
                });
            }
        }
    }

    Err(format!(
        "Wallpaper manifest not found for {:?}. Checked directly and in XDG directories: {:?}",
        input,
        wallpaper_search_dirs()
    ))
}

/// Returns candidate directories to search for wallpaper templates.
pub fn template_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. User templates: $XDG_DATA_HOME/wallrs/templates
    dirs.push(data_home().join("wallrs").join("templates"));

    // 2. Working directory / repository fallback: ./examples, ../../examples
    dirs.push(PathBuf::from("examples"));
    dirs.push(PathBuf::from("../../examples"));

    // Workspace repository fallback during development and testing
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let p = Path::new(manifest_dir);
        if let Some(workspace_root) = p.parent().and_then(|p| p.parent()) {
            dirs.push(workspace_root.join("examples"));
        }
    }

    // 3. Relative to current executable binary
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(bin_dir) = exe_path.parent()
    {
        dirs.push(bin_dir.join("examples"));
        dirs.push(bin_dir.join("..").join("examples"));
        dirs.push(bin_dir.join("..").join("..").join("examples"));
        dirs.push(bin_dir.join("..").join("..").join("..").join("examples"));
        dirs.push(
            bin_dir
                .join("..")
                .join("share")
                .join("wallrs")
                .join("examples"),
        );
    }

    // 4. System templates and examples
    for d in data_dirs() {
        dirs.push(d.join("wallrs").join("templates"));
        dirs.push(d.join("wallrs").join("examples"));
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xdg_data_home_not_empty() {
        let home = data_home();
        assert!(!home.as_os_str().is_empty());
    }

    #[test]
    fn test_wallpaper_search_dirs_contains_user_dir() {
        let dirs = wallpaper_search_dirs();
        assert!(!dirs.is_empty());
        assert_eq!(dirs[0], default_user_wallpapers_dir());
    }

    #[test]
    fn test_resolve_existing_example_manifest() {
        let p = Path::new("examples/aurora-shader");
        if p.exists() {
            let res = resolve_wallpaper_path(p);
            assert!(res.is_ok());
        }
    }

    #[test]
    fn test_resolve_example_by_name_without_path() {
        let p = Path::new("parallax-landscape");
        if Path::new("examples/parallax-landscape/wallpaper.toml").exists() {
            let res = resolve_wallpaper_path(p);
            assert!(res.is_ok());
        }
    }
}
