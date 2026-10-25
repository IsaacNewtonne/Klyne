//! One path key for file receipts, used when a write is recorded and when a
//! completion names a file. Without it a receipt for `summary.txt` did not
//! match an answer naming `\\?\C:\...\files\summary.txt` (2026-10-25).

use std::path::{Path, PathBuf};

/// Strip Windows verbatim prefixes: `\\?\C:\x` -> `C:\x`,
/// `\\?\UNC\srv\share` -> `\\srv\share` (also with forward slashes).
fn strip_verbatim(path: &str) -> String {
    for prefix in [r"\\?\UNC\", "//?/UNC/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return format!(r"\\{rest}");
        }
    }
    for prefix in [r"\\?\", "//?/", r"\\.\"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    path.to_owned()
}

/// `C:` style drive or `\\server` UNC: compared case-insensitively.
fn windows_like(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
        || path.starts_with(r"\\")
        || path.starts_with("//")
}

fn is_absolute(path: &str) -> bool {
    windows_like(path) || path.starts_with('/') || path.starts_with('\\')
}

/// Lexically fold `.` and `..` with `/` separators.
fn fold(path: &str) -> String {
    let unc = path.starts_with("//");
    let rooted = path.starts_with('/') && !unc;
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                // Never pop past a drive or UNC root.
                let floor = usize::from(unc) * 2
                    + usize::from(parts.first().is_some_and(|p| p.ends_with(':')));
                if parts.len() > floor {
                    parts.pop();
                }
            }
            p => parts.push(p),
        }
    }
    let joined = parts.join("/");
    if unc {
        format!("//{joined}")
    } else if rooted {
        format!("/{joined}")
    } else {
        joined
    }
}

/// The comparison key for a file path: verbatim prefix stripped, relative
/// paths resolved against `workspace`, canonicalized when the file exists
/// (which also expands Windows 8.3 short names), `/` separators, `.`/`..`
/// folded, and lowercase for Windows-style paths.
pub fn key(path: &str, workspace: Option<&Path>) -> String {
    let raw = strip_verbatim(path.trim());
    let joined: PathBuf = match workspace {
        Some(root) if !is_absolute(&raw) => {
            PathBuf::from(strip_verbatim(&root.to_string_lossy())).join(&raw)
        }
        _ => PathBuf::from(&raw),
    };
    let real = std::fs::canonicalize(&joined).ok().map_or_else(
        || joined.to_string_lossy().into_owned(),
        |p| p.to_string_lossy().into_owned(),
    );
    let text = strip_verbatim(&real).replace('\\', "/");
    let folded = fold(&text);
    if windows_like(&folded) || cfg!(windows) {
        folded.to_lowercase()
    } else {
        folded
    }
}

/// Whether two names refer to the same file under `workspace`.
#[cfg(test)]
pub fn same(a: &str, b: &str, workspace: Option<&Path>) -> bool {
    !a.trim().is_empty() && !b.trim().is_empty() && key(a, workspace) == key(b, workspace)
}

/// Does this receipt name the file `path`? Uses the key recorded with the
/// receipt when present, else keys its target now.
pub fn receipt_names(receipt: &serde_json::Value, path: &str, workspace: Option<&Path>) -> bool {
    let wanted = key(path, workspace);
    receipt["path_key"].as_str() == Some(wanted.as_str())
        || receipt["target"]
            .as_str()
            .is_some_and(|target| !target.is_empty() && key(target, workspace) == wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str = r"\\?\C:\temp\klyne\conversations\1-1\files";

    fn ws() -> Option<&'static Path> {
        Some(Path::new(WS))
    }

    #[test]
    fn verbatim_absolute_matches_relative_receipt() {
        // The live failure: receipt "summary.txt", answer names the full
        // verbatim path under the workspace.
        assert!(same(
            r"\\?\C:\temp\klyne\conversations\1-1\files\summary.txt",
            "summary.txt",
            ws()
        ));
        assert!(same(r"\\?\C:\x\a.txt", r"C:\x\a.txt", None));
        assert!(same(r"\\?\UNC\srv\share\a.txt", r"\\srv\share\a.txt", None));
    }

    #[test]
    fn separators_case_and_dots() {
        assert!(same(
            "C:/temp/klyne/conversations/1-1/files/summary.txt",
            "summary.txt",
            ws()
        ));
        assert!(same(
            r"C:\TEMP\Klyne\conversations\1-1\FILES\Summary.TXT",
            "summary.txt",
            ws()
        ));
        assert!(same("./notes/a.md", r"notes\a.md", ws()));
        assert!(same("notes/../a.md", "a.md", ws()));
        assert!(same(r"C:\x\y\..\a.txt", "c:/x/a.txt", None));
        // `..` never climbs above the drive root.
        assert_eq!(key(r"C:\..\..\a.txt", None), "c:/a.txt");
    }

    #[test]
    fn different_files_stay_different() {
        assert!(!same("summary.txt", "other.txt", ws()));
        assert!(!same("sub/summary.txt", "summary.txt", ws()));
        assert!(!same(
            r"D:\temp\klyne\conversations\1-1\files\summary.txt",
            "summary.txt",
            ws()
        ));
        assert!(!same("", "", ws()));
        // Unix paths keep their case off Windows.
        if !cfg!(windows) {
            assert!(!same("/tmp/A.txt", "/tmp/a.txt", None));
        }
    }

    #[test]
    fn existing_files_are_canonicalized() {
        let dir = std::env::temp_dir().join(format!("klyne-paths-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("f.txt"), "x").unwrap();
        let root = dir.to_string_lossy().into_owned();
        assert!(same(
            "sub/f.txt",
            &format!("{root}/sub/./f.txt"),
            Some(&dir)
        ));
        assert!(same("sub/../sub/f.txt", "sub/f.txt", Some(&dir)));
        let receipt = serde_json::json!({"target":"sub/f.txt"});
        assert!(receipt_names(
            &receipt,
            &format!("{root}/sub/f.txt"),
            Some(&dir)
        ));
        let keyed =
            serde_json::json!({"target":"elsewhere","path_key":key("sub/f.txt", Some(&dir))});
        assert!(receipt_names(&keyed, "sub/f.txt", Some(&dir)));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 8.3 short names resolve through canonicalize when the volume has
    /// them; skipped where short names are disabled.
    #[cfg(windows)]
    #[test]
    fn short_names_match_long_names() {
        let dir =
            std::env::temp_dir().join(format!("klyne long folder name {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("summary.txt"), "x").unwrap();
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("cmd")
            .raw_arg(format!(
                "/c for %I in (\"{}\") do @echo %~sI",
                dir.display()
            ))
            .output()
            .unwrap();
        let short = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !short.is_empty() && !short.eq_ignore_ascii_case(&dir.to_string_lossy()) {
            assert!(
                same(&format!(r"{short}\summary.txt"), "summary.txt", Some(&dir)),
                "{short}"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
