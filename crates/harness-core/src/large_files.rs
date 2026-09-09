//! Bounded observations over larger regular files in a trusted workspace.
use crate::{permissions::PermissionPolicy, tools::MAX_FILE_BYTES, types::Action};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

pub const MAX_STREAM_BYTES: u64 = 64 * 1024 * 1024;
const BUFFER_BYTES: usize = 64 * 1024;
/// Needles are short identifiers or snippets, not file dumps.
pub const MAX_SEARCH_NEEDLE_BYTES: usize = 1024;
/// Upper bound on reported matches per call; extra matches set `truncated`.
pub const MAX_SEARCH_MATCHES: u64 = 50;
const SEARCH_EXCERPT_BYTES: u64 = 160;

fn open_regular(path: &Path) -> io::Result<File> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::other("regular file required"));
    }
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("regular file required"));
    }
    Ok(file)
}

fn digest(file: &mut File) -> io::Result<(String, u64)> {
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; BUFFER_BYTES];
    let mut size = 0;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > MAX_STREAM_BYTES {
            return Err(io::Error::other("stream exceeds 64 MiB limit"));
        }
        hasher.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(position) = haystack[start..]
        .windows(needle.len())
        .position(|window| window == needle)
    {
        let index = start + position;
        out.push(index);
        start = index + 1;
    }
    out
}

struct SearchReport {
    /// (byte offset, 1-based line number) pairs in file order.
    matches: Vec<(u64, u64)>,
    truncated: bool,
    bytes_scanned: u64,
}

fn search_streaming(file: &mut File, needle: &[u8], max_matches: u64) -> io::Result<SearchReport> {
    let overlap = needle.len() - 1;
    let mut buffer = vec![0u8; BUFFER_BYTES + overlap];
    let mut carried = 0usize;
    let mut consumed = 0u64;
    let mut lines_before = 0u64;
    let mut matches = Vec::new();
    let mut truncated = false;
    loop {
        let count = file.read(&mut buffer[carried..carried + BUFFER_BYTES])?;
        if count == 0 {
            break;
        }
        consumed = consumed
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("stream overflow"))?;
        if consumed > MAX_STREAM_BYTES {
            return Err(io::Error::other("stream exceeds 64 MiB limit"));
        }
        let valid = carried + count;
        let base = consumed - count as u64 - carried as u64;
        for index in find_all(&buffer[..valid], needle) {
            if index + needle.len() <= carried {
                continue;
            }
            let absolute = base + index as u64;
            let line = if index >= carried {
                lines_before
                    + buffer[carried..index]
                        .iter()
                        .filter(|b| **b == b'\n')
                        .count() as u64
                    + 1
            } else {
                lines_before
                    - buffer[index..carried]
                        .iter()
                        .filter(|b| **b == b'\n')
                        .count() as u64
                    + 1
            };
            if matches.len() < max_matches as usize {
                matches.push((absolute, line));
            } else {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
        lines_before += buffer[carried..valid]
            .iter()
            .filter(|b| **b == b'\n')
            .count() as u64;
        let carry = overlap.min(valid);
        buffer.copy_within(valid - carry..valid, 0);
        carried = carry;
    }
    Ok(SearchReport {
        matches,
        truncated,
        bytes_scanned: consumed,
    })
}

fn search(action: &Action, policy: &PermissionPolicy) -> io::Result<String> {
    let (path, needle, max_matches) = match action {
        Action::SearchFile {
            path,
            needle,
            max_matches,
        } => (path, needle, *max_matches),
        _ => return Err(io::Error::other("unsupported search operation")),
    };
    if needle.is_empty() || needle.len() > MAX_SEARCH_NEEDLE_BYTES {
        return Err(io::Error::other("needle must be 1..1024 bytes"));
    }
    if max_matches == 0 || max_matches > MAX_SEARCH_MATCHES {
        return Err(io::Error::other("max_matches must be 1..50"));
    }
    let full = policy
        .resolve_workspace_path(path)
        .map_err(io::Error::other)?;
    let mut file = open_regular(&full)?;
    if file.metadata()?.len() > MAX_STREAM_BYTES {
        return Err(io::Error::other("stream exceeds 64 MiB limit"));
    }
    let needle_bytes = needle.as_bytes();
    let report = search_streaming(&mut file, needle_bytes, max_matches)?;
    let (found, truncated, scanned) = (report.matches, report.truncated, report.bytes_scanned);
    let mut details = Vec::with_capacity(found.len());
    for (offset, line) in found {
        file.seek(SeekFrom::Start(offset))?;
        let mut excerpt = vec![0u8; SEARCH_EXCERPT_BYTES as usize];
        let mut filled = 0;
        while filled < excerpt.len() {
            match file.read(&mut excerpt[filled..])? {
                0 => break,
                n => filled += n,
            }
        }
        excerpt.truncate(filled);
        details.push(serde_json::json!({
            "offset": offset,
            "line": line,
            "excerpt": String::from_utf8_lossy(&excerpt),
        }));
    }
    Ok(serde_json::json!({
        "path": path,
        "needle": needle,
        "matches": details,
        "truncated": truncated,
        "bytes_scanned": scanned,
    })
    .to_string())
}

pub(crate) fn execute(action: &Action, policy: &PermissionPolicy) -> io::Result<String> {
    if matches!(action, Action::SearchFile { .. }) {
        return search(action, policy);
    }
    let path = match action {
        Action::ReadFileRange { path, .. }
        | Action::HashFile { path }
        | Action::PatchFile { path, .. } => path,
        _ => return Err(io::Error::other("unsupported large-file operation")),
    };
    let full = policy
        .resolve_workspace_path(path)
        .map_err(io::Error::other)?;
    let mut file = open_regular(&full)?;
    match action {
        Action::ReadFileRange { offset, length, .. } => {
            if *length > MAX_FILE_BYTES as u64 {
                return Err(io::Error::other("range exceeds 1 MiB limit"));
            }
            let size = file.metadata()?.len();
            let end = offset
                .checked_add(*length)
                .ok_or_else(|| io::Error::other("range overflow"))?;
            if end > size {
                return Err(io::Error::other("range extends past EOF"));
            }
            file.seek(SeekFrom::Start(*offset))?;
            let mut bytes = vec![0; *length as usize];
            file.read_exact(&mut bytes)?;
            let text = String::from_utf8(bytes).map_err(|_| {
                io::Error::other("range is not valid UTF-8; adjust byte boundaries")
            })?;
            Ok(serde_json::json!({"offset":offset,"length":length,"file_size":size,"eof":end == size,"text":text}).to_string())
        }
        Action::HashFile { .. } => {
            let (sha256, bytes) = digest(&mut file)?;
            Ok(serde_json::json!({"algorithm":"sha256","sha256":sha256,"bytes":bytes}).to_string())
        }
        Action::PatchFile {
            offset,
            expected,
            replacement,
            expected_sha256,
            ..
        } => {
            if expected.len() > MAX_FILE_BYTES || replacement.len() > MAX_FILE_BYTES {
                return Err(io::Error::other("patch operands exceed 1 MiB limit"));
            }
            if expected_sha256.len() != 64
                || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(io::Error::other(
                    "expected_sha256 must be a SHA-256 hex digest",
                ));
            }
            let metadata = file.metadata()?;
            if metadata.permissions().readonly() {
                return Err(io::Error::other("target is read-only"));
            }
            let (before, size) = digest(&mut file)?;
            if !before.eq_ignore_ascii_case(expected_sha256) {
                return Err(io::Error::other("stale file digest"));
            }
            let end = offset
                .checked_add(expected.len() as u64)
                .ok_or_else(|| io::Error::other("patch offset overflow"))?;
            if end > size {
                return Err(io::Error::other("patch extends past EOF"));
            }
            let output_size = size - expected.len() as u64 + replacement.len() as u64;
            if output_size > MAX_STREAM_BYTES {
                return Err(io::Error::other("patched file exceeds 64 MiB limit"));
            }
            file.seek(SeekFrom::Start(*offset))?;
            let mut actual = vec![0; expected.len()];
            file.read_exact(&mut actual)?;
            if actual != expected.as_bytes() {
                return Err(io::Error::other("patch expected bytes do not match"));
            }
            let mut staged = tempfile::NamedTempFile::new_in(
                full.parent()
                    .ok_or_else(|| io::Error::other("missing parent"))?,
            )?;
            file.seek(SeekFrom::Start(0))?;
            if io::copy(&mut (&mut file).take(*offset), &mut staged)? != *offset {
                return Err(io::Error::other("file changed during patch"));
            }
            staged.write_all(replacement.as_bytes())?;
            file.seek(SeekFrom::Start(end))?;
            if io::copy(&mut (&mut file).take(size - end), &mut staged)? != size - end {
                return Err(io::Error::other("file changed during patch"));
            }
            staged.as_file().set_permissions(metadata.permissions())?;
            staged.as_file().sync_all()?;
            let (after, _) = digest(staged.as_file_mut())?;
            // Recheck permission/path and whole-file precondition before publication.
            if policy.check(action) != crate::permissions::PermissionDecision::Allow {
                return Err(io::Error::other("patch permission changed"));
            }
            let mut current = open_regular(&full)?;
            if digest(&mut current)?.0 != before {
                return Err(io::Error::other("file changed during patch"));
            }
            drop(current);
            drop(file);
            staged.persist(&full).map_err(|e| e.error)?;
            Ok(serde_json::json!({"before_sha256":before,"after_sha256":after,"bytes":output_size}).to_string())
        }
        _ => Err(io::Error::other("unsupported large-file operation")),
    }
}
