//! Resolve user-authored project references, never model or tool-output text.
use std::path::{Path, PathBuf};

pub fn text_revision(message: &str) -> bool {
    let words: Vec<String> = message
        .split(|c: char| !c.is_alphabetic())
        .map(str::to_lowercase)
        .collect();
    let has = |choices: &[&str]| words.iter().any(|word| choices.contains(&word.as_str()));
    has(&[
        "revise", "revision", "rewrite", "improve", "better", "dope", "dopest", "edit",
    ]) && has(&[
        "album",
        "lyrics",
        "lyrical",
        "verse",
        "poem",
        "prose",
        "manuscript",
        "draft",
    ]) && !has(&["send", "publish", "upload", "deploy", "email", "browser"])
}

pub fn project_directory(message: &str) -> Option<PathBuf> {
    let mut found = Vec::new();
    for (start, _) in message.char_indices() {
        if start > 0
            && !message[..start]
                .ends_with(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '('))
        {
            continue;
        }
        let tail = &message[start..];
        let bytes = tail.as_bytes();
        let rooted = if cfg!(windows) {
            bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/')
        } else {
            tail.starts_with('/')
        };
        if !rooted {
            continue;
        }
        let end = tail.find(['\n', '\r', '"', '`', ';']).unwrap_or(tail.len());
        let tail = &tail[..end];
        let mut ends: Vec<usize> = tail
            .char_indices()
            .filter_map(|(i, c)| (c.is_whitespace() || matches!(c, ',' | ')' | '\'')).then_some(i))
            .collect();
        ends.push(tail.len());
        let mut selected = false;
        for &end in ends.iter().rev() {
            let mut path = Path::new(tail[..end].trim_end().trim_end_matches(['.', '?', '!']));
            // A named file selects its containing folder (2026-10-24), so
            // "read C:\\Users\\me\\Desktop\\notes.txt" works on the Desktop.
            if path.is_file()
                && let Some(parent) = path.parent()
            {
                path = parent;
            }
            if crate::safe_dir(path).is_ok()
                && let Ok(path) = std::fs::canonicalize(path)
            {
                if crate::safe_dir(&path).is_ok() && !found.contains(&path) {
                    found.push(path);
                }
                selected = true;
                break;
            }
        }
        // A file or folder the request will create selects its existing
        // parent (2026-10-24), so "write C:\\temp\\demo\\new.txt" or "make a
        // folder C:\\temp\\demo" uses the file tools instead of the shell.
        if !selected {
            for &end in ends.iter().rev() {
                let path = Path::new(tail[..end].trim_end().trim_end_matches(['.', '?', '!']));
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if name.contains(char::is_whitespace) || path.exists() {
                    continue;
                }
                if let Some(parent) = path.parent()
                    && parent.is_dir()
                    && crate::safe_dir(parent).is_ok()
                    && let Ok(parent) = std::fs::canonicalize(parent)
                {
                    if crate::safe_dir(&parent).is_ok() && !found.contains(&parent) {
                        found.push(parent);
                    }
                    break;
                }
            }
        }
    }
    (found.len() == 1).then(|| found.remove(0))
}

/// Convert absolute paths only when they name a location inside the selected project.
/// The ordinary file policy still checks traversal, reserved names and symlinks.
pub fn relative_path(workspace: &Path, raw: &str) -> String {
    let path = Path::new(raw);
    if !path.is_absolute() {
        return raw.to_owned();
    }
    let root = workspace.to_string_lossy();
    let root = root.strip_prefix("\\\\?\\").unwrap_or(&root);
    let raw = raw.strip_prefix("\\\\?\\").unwrap_or(raw);
    let root_path = Path::new(root);
    let path = Path::new(raw);
    match path.strip_prefix(root_path) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".into(),
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => raw.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chat_folder_with_spaces_and_prose_is_resolved_without_guessing_between_projects() {
        let temp = tempfile::tempdir().unwrap();
        let album = temp.path().join("10 - MORAL HAZARD");
        std::fs::create_dir(&album).unwrap();
        let canonical = std::fs::canonicalize(&album).unwrap();
        assert_eq!(
            project_directory(&format!(
                "Improve this album {}, go all out",
                album.display()
            )),
            Some(canonical.clone())
        );
        assert_eq!(
            project_directory(&format!("Use \"{}\"", album.display())),
            Some(canonical.clone())
        );
        let other = temp.path().join("other");
        std::fs::create_dir(&other).unwrap();
        assert_eq!(
            project_directory(&format!(
                "Compare \"{}\" and \"{}\"",
                album.display(),
                other.display()
            )),
            None
        );
        assert_eq!(
            relative_path(&canonical, &album.join("song.txt").to_string_lossy()),
            "song.txt"
        );
        assert_eq!(relative_path(&canonical, &album.to_string_lossy()), ".");
        assert_eq!(
            relative_path(&canonical, &other.to_string_lossy()),
            other.to_string_lossy()
        );
    }
    #[test]
    fn a_file_or_folder_to_be_created_selects_its_existing_parent() {
        let temp = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(temp.path()).unwrap();
        let new_file = temp.path().join("caps.txt");
        assert_eq!(
            project_directory(&format!(
                "write {} containing 'ok', then read it back",
                new_file.display()
            )),
            Some(canonical.clone())
        );
        let new_dir = temp.path().join("demo");
        assert_eq!(
            project_directory(&format!(
                "make a folder {}, write notes.txt",
                new_dir.display()
            )),
            Some(canonical)
        );
        // Two missing levels are not guessed.
        assert_eq!(
            project_directory(&format!(
                "write {}",
                temp.path().join("a").join("b.txt").display()
            )),
            None
        );
    }

    #[test]
    fn a_named_file_selects_its_folder() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        let canonical = std::fs::canonicalize(temp.path()).unwrap();
        assert_eq!(
            project_directory(&format!("read {} and summarize it.", file.display())),
            Some(canonical.clone())
        );
        assert_eq!(
            project_directory(&format!("delete {}.", file.display())),
            Some(canonical)
        );
    }
}
