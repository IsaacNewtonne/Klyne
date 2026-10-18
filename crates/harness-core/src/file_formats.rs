//! Text writes must not impersonate encoded binary documents.
use std::path::Path;

pub fn binary_extension(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "doc"
            | "docx"
            | "docm"
            | "dotx"
            | "xls"
            | "xlsx"
            | "xlsm"
            | "xlsb"
            | "ppt"
            | "pptx"
            | "pptm"
            | "odt"
            | "ods"
            | "odp"
            | "pdf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "avif"
            | "heic"
            | "zip"
            | "7z"
            | "rar"
            | "gz"
            | "bz2"
            | "xz"
            | "tar"
            | "mp3"
            | "mp4"
            | "wav"
            | "flac"
            | "ogg"
            | "m4a"
            | "aac"
            | "avi"
            | "mov"
            | "mkv"
            | "webm"
            | "exe"
            | "dll"
            | "sqlite"
            | "sqlite3"
            | "glb"
            | "fbx"
    )
}

#[cfg(test)]
mod tests {
    use crate::{Action, PermissionPolicy, ToolRegistry};
    #[test]
    fn text_writes_cannot_impersonate_binary_formats() {
        let root = tempfile::tempdir().unwrap();
        let policy = PermissionPolicy::milestone_default(root.path());
        for ext in ["docx", "XLSX", "pptx", "pdf", "PNG", "zip", "mp4"] {
            let path = format!("fake.{ext}");
            let observed = ToolRegistry::milestone_default().execute(
                &Action::WriteFile {
                    path: path.clone(),
                    contents: "ordinary prose".into(),
                },
                &policy,
            );
            assert!(!observed.ok, "{ext}");
            assert!(!root.path().join(path).exists());
        }
        assert!(
            ToolRegistry::milestone_default()
                .execute(
                    &Action::WriteFile {
                        path: "report.md".into(),
                        contents: "ordinary prose".into()
                    },
                    &policy
                )
                .ok
        );
    }
}
