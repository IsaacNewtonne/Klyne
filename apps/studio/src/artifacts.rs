//! Byte transport and bounded structural checks. No format or rendering claims
//! are inferred from a filename, signature, or digest alone.
use crate::err;
use base64::{Engine, engine::general_purpose::STANDARD};
use harness_core::{Action, PermissionDecision, PermissionPolicy};
use quick_xml::{Reader, events::Event};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_EXPANDED: usize = 32 * 1024 * 1024;
const MAX_ENTRY: usize = 8 * 1024 * 1024;
pub const INSTRUCTIONS: &str = r#"
artifact_read {path,offset?:0,length?:8192} pages arbitrary bytes as base64 (maximum 8192 bytes/page), with digest, size and next_offset. artifact_inspect {path} checks available format structure. artifact_verify {path,expected_sha256?} distinguishes byte integrity from structural validity; unknown formats need a real format adapter. artifact_write {path,encoding:"utf8|base64",data} publishes bytes without replacing different existing contents and reads them back. Base64 is transport, not an encoder. These operations are bounded to 16 MiB; use approved bounded commands/tools for larger files.
document_create {path,title?,paragraphs:[strings]} creates a real .docx package, independently checks its text and structure, and does not overwrite different contents. It does not open Word or certify rendering. document_read extracts its text. For editing/conversion preserve the source and use an explicit new output path and a real adapter. Structural ZIP/XML/Office checks are not full schemas, formulas, meaningful content or application rendering. Image/media/PDF/proprietary decoding requires a real adapter. Never claim those formats are validated from signatures or hashes alone.
"#;

#[derive(Deserialize)]
#[serde(tag = "tool", deny_unknown_fields)]
enum ArtifactAction {
    #[serde(rename = "artifact_read")]
    Read {
        path: String,
        #[serde(default)]
        offset: usize,
        #[serde(default = "page_size")]
        length: usize,
    },
    #[serde(rename = "artifact_write")]
    Write {
        path: String,
        encoding: String,
        data: String,
    },
    #[serde(rename = "artifact_inspect")]
    Inspect { path: String },
    #[serde(rename = "artifact_verify")]
    Verify {
        path: String,
        expected_sha256: Option<String>,
    },
    #[serde(rename = "document_create")]
    Document {
        path: String,
        title: Option<String>,
        paragraphs: Vec<String>,
    },
}
fn page_size() -> usize {
    8192
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn stopped(stop: &AtomicBool) -> io::Result<()> {
    if stop.load(Ordering::SeqCst) {
        Err(err("Stopped before publication"))
    } else {
        Ok(())
    }
}

pub struct Prepared {
    action: ArtifactAction,
    path: String,
    destination: PathBuf,
    bytes: Option<Vec<u8>>,
    structure: Option<Value>,
}
impl Prepared {
    pub fn intent(&self) -> Option<Value> {
        self.bytes.as_ref().map(|bytes| json!({"artifact_intent":{"version":1,"path":self.path,"sha256":digest(bytes),"bytes":bytes.len()}}))
    }
}

pub fn prepare(
    workspace: &Path,
    value: &Value,
    review: bool,
    stop: &AtomicBool,
) -> io::Result<Prepared> {
    stopped(stop)?;
    let action: ArtifactAction = serde_json::from_value(value.clone()).map_err(err)?;
    let (path, write) = match &action {
        ArtifactAction::Read { path, length, .. } => {
            if *length == 0 || *length > page_size() {
                return Err(err("length must be 1–8192"));
            }
            (path, false)
        }
        ArtifactAction::Inspect { path } => (path, false),
        ArtifactAction::Verify {
            path,
            expected_sha256,
        } => {
            if expected_sha256.as_ref().is_some_and(|v| !valid_digest(v)) {
                return Err(err("expected_sha256 must be 64 hex characters"));
            }
            (path, false)
        }
        ArtifactAction::Write { path, .. } | ArtifactAction::Document { path, .. } => (path, true),
    };
    if path.is_empty() || path.len() > 4096 {
        return Err(err("Invalid artifact path"));
    }
    if write && review {
        return Err(err("Reviewer cannot publish artifacts"));
    }
    let destination = resolve(workspace, path, write)?;
    let bytes = match &action {
        ArtifactAction::Write { encoding, data, .. } => {
            if data.len() > MAX_BYTES.div_ceil(3) * 4 {
                return Err(err("Encoded artifact exceeds 16 MiB limit"));
            }
            let bytes = match encoding.as_str() {
                "utf8" => data.as_bytes().to_vec(),
                "base64" => STANDARD.decode(data).map_err(err)?,
                _ => return Err(err("encoding must be utf8 or base64")),
            };
            if bytes.len() > MAX_BYTES {
                return Err(err("Artifact exceeds 16 MiB limit"));
            }
            Some(bytes)
        }
        ArtifactAction::Document {
            path,
            title,
            paragraphs,
        } => {
            if extension(path) != "docx" {
                return Err(err(
                    "document_create supports .docx; use resource_route for other formats",
                ));
            }
            Some(docx(title.as_deref(), paragraphs)?)
        }
        _ => None,
    };
    let structure = bytes
        .as_ref()
        .map(|bytes| inspect_bytes(path, bytes))
        .transpose()?;
    stopped(stop)?;
    Ok(Prepared {
        path: path.clone(),
        destination,
        action,
        bytes,
        structure,
    })
}

fn resolve(workspace: &Path, path: &str, write: bool) -> io::Result<PathBuf> {
    let policy = PermissionPolicy::milestone_default(workspace);
    let action = if write {
        Action::WriteFile {
            path: path.into(),
            contents: String::new(),
        }
    } else {
        Action::ReadFile { path: path.into() }
    };
    if policy.check(&action) != PermissionDecision::Allow {
        return Err(err("Artifact path denied by workspace policy"));
    }
    policy.resolve_workspace_path(path).map_err(err)
}
fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    if !fs::metadata(path)?.is_file() {
        return Err(err("Artifact must be a regular file"));
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(err("Artifact must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(err(
            "Artifact exceeds 16 MiB; use a bounded command or qualified tool",
        ));
    }
    Ok(bytes)
}
pub fn execute(prepared: Prepared, workspace: &Path, stop: &AtomicBool) -> io::Result<Value> {
    stopped(stop)?;
    // Resolve again immediately before use; discovery is not permission.
    if resolve(workspace, &prepared.path, prepared.bytes.is_some())? != prepared.destination {
        return Err(err("Artifact path changed before dispatch"));
    }
    if let Some(bytes) = &prepared.bytes {
        return publish(&prepared, bytes, workspace, stop);
    }
    let bytes = read_bytes(&prepared.destination)?;
    stopped(stop)?;
    let sha256 = digest(&bytes);
    let mut value = json!({"ok":true,"path":prepared.path,"size":bytes.len(),"sha256":sha256,"content_verified":false,"effect":"not_applied"});
    match prepared.action {
        ArtifactAction::Read { offset, length, .. } => {
            if offset > bytes.len() {
                return Err(err("offset exceeds file size"));
            }
            let end = offset.saturating_add(length).min(bytes.len());
            value["encoding"] = json!("base64");
            value["data"] = json!(STANDARD.encode(&bytes[offset..end]));
            value["offset"] = json!(offset);
            value["next_offset"] = if end < bytes.len() {
                json!(end)
            } else {
                Value::Null
            };
        }
        ArtifactAction::Inspect { .. } | ArtifactAction::Verify { .. } => {
            // A digest may still establish integrity when a format check fails.
            let structure = inspect_bytes(&prepared.path, &bytes)
                .unwrap_or_else(|e| json!({"status":"invalid","error":e.to_string()}));
            if let ArtifactAction::Verify {
                expected_sha256, ..
            } = &prepared.action
            {
                if let Some(expected) = expected_sha256 {
                    value["ok"] = json!(sha256.eq_ignore_ascii_case(expected));
                    value["verification_scope"] = json!("byte_integrity_only");
                } else {
                    value["ok"] = json!(structure["status"] == "checked");
                    value["verification_scope"] = json!("format_structure_only");
                    if structure["status"] != "checked" {
                        value["needs_adapter"] = json!(true);
                    }
                }
            }
            value["structure"] = structure;
        }
        _ => unreachable!(),
    }
    Ok(value)
}

fn publish(
    prepared: &Prepared,
    bytes: &[u8],
    workspace: &Path,
    stop: &AtomicBool,
) -> io::Result<Value> {
    let existing = match fs::symlink_metadata(&prepared.destination) {
        Ok(_) => Some(read_bytes(&prepared.destination)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if existing.as_ref().is_some_and(|value| value != bytes) {
        return Err(err(
            "Destination already contains different bytes; choose a new output path",
        ));
    }
    let already_matches = existing.is_some();
    if !already_matches {
        let parent = prepared
            .destination
            .parent()
            .ok_or_else(|| err("No artifact parent"))?;
        fs::create_dir_all(parent)?;
        resolve(workspace, &prepared.path, true)?;
        let mut stage = tempfile::NamedTempFile::new_in(parent)?;
        stage.write_all(bytes)?;
        stage.as_file().sync_all()?;
        stopped(stop)?;
        // Atomic no-clobber publication; competing writers cannot replace data.
        if let Err(error) = stage.persist_noclobber(&prepared.destination) {
            if error.error.kind() != io::ErrorKind::AlreadyExists {
                return Ok(
                    json!({"ok":false,"uncertain":true,"error":format!("Publication outcome unknown: {}", error.error)}),
                );
            }
            if read_bytes(&prepared.destination)? != bytes {
                return Err(err(
                    "Destination changed during publication; no replacement performed",
                ));
            }
        }
    }
    // Once published, a failed readback is uncertain, never a retry invitation.
    match resolve(workspace, &prepared.path, false).and_then(|p| read_bytes(&p)) {
        Ok(saved) if saved == bytes => {
            let mut value = receipt(&prepared.path, bytes);
            value["already_matches"] = json!(already_matches);
            value["structure"] = prepared.structure.clone().unwrap_or(Value::Null);
            value["summary"] = json!(
                "Destination bytes independently match; format/content checks have separate scopes"
            );
            Ok(value)
        }
        _ => Ok(
            json!({"ok":false,"uncertain":true,"error":"Published artifact could not be confirmed; inspect and reconcile before retrying"}),
        ),
    }
}
fn receipt(path: &str, bytes: &[u8]) -> Value {
    let sha256 = digest(bytes);
    json!({"ok":true,"path":path,"size":bytes.len(),"sha256":sha256,"verification_scope":"byte_integrity_only","content_verified":false,"receipt":{"version":1,"effect":"file_write","target":path,"criterion":{"FileDigest":{"path":path,"sha256":sha256}}}})
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn reconcile(workspace: &Path, pending: &Value) -> Option<Value> {
    let intent = &pending["artifact_intent"];
    if intent["version"] != 1 {
        return None;
    }
    let path = intent["path"].as_str()?;
    let expected = intent["sha256"].as_str()?;
    if !valid_digest(expected) || intent["bytes"].as_u64()? > MAX_BYTES as u64 {
        return None;
    }
    let bytes = read_bytes(&resolve(workspace, path, false).ok()?).ok()?;
    if bytes.len() as u64 != intent["bytes"].as_u64()? || digest(&bytes) != expected {
        return None;
    }
    let mut value = receipt(path, &bytes);
    value["reconciled"] = json!(true);
    value["structure"] = inspect_bytes(path, &bytes)
        .unwrap_or_else(|e| json!({"status":"invalid","error":e.to_string()}));
    Some(value)
}

pub fn inspect_bytes(path: &str, bytes: &[u8]) -> io::Result<Value> {
    let format = extension(path);
    match format.as_str() {
        "json" => {
            serde_json::from_slice::<Value>(bytes).map_err(err)?;
        }
        "xml" | "svg" => {
            let (root, _) = xml(bytes)?;
            if format == "svg" && root != "svg" {
                return Err(err("SVG requires an svg root element"));
            }
        }
        "zip" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => {
            return package(&format, bytes).map(|(value, _)| value);
        }
        "txt" | "md" | "markdown" | "csv" | "tsv" | "html" | "css" | "js" | "py" | "rs"
        | "yaml" | "yml" => {
            std::str::from_utf8(bytes).map_err(err)?;
            return Ok(
                json!({"status":"checked","format":format,"scope":"utf8_encoding_only","content_verified":false}),
            );
        }
        _ => {
            return Ok(
                json!({"status":"adapter_required","format":format,"scope":"none","content_verified":false}),
            );
        }
    }
    Ok(json!({"status":"checked","format":format,"scope":"syntax_only","content_verified":false}))
}

fn valid_xml_chars(text: &str) -> bool {
    text.chars().all(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
}
fn valid_xml_name(bytes: &[u8]) -> bool {
    fn start(c: char) -> bool {
        matches!(c, ':' | '_' | 'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}' | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}' | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}' | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
    }
    let Ok(name) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut chars = name.chars();
    chars.next().is_some_and(start) && chars.all(|c| start(c) || matches!(c, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}'))
}
fn xml(bytes: &[u8]) -> io::Result<(String, String)> {
    let source = std::str::from_utf8(bytes).map_err(err)?;
    if !valid_xml_chars(source) {
        return Err(err("Invalid XML character"));
    }
    let mut reader = Reader::from_str(source);
    reader.config_mut().check_comments = true;
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0usize;
    let mut roots = 0usize;
    let mut root = String::new();
    let mut text = String::new();
    let mut events = 0;
    loop {
        match reader.read_event().map_err(err)? {
            Event::Start(e) => {
                if !valid_xml_name(e.name().as_ref()) {
                    return Err(err("Invalid XML element name"));
                }
                if depth == 0 {
                    roots += 1;
                    root = String::from_utf8(e.local_name().as_ref().to_vec()).map_err(err)?;
                }
                for attr in e.attributes() {
                    let attr = attr.map_err(err)?;
                    if !valid_xml_name(attr.key.as_ref()) || attr.value.contains(&b'<') {
                        return Err(err("Invalid XML attribute"));
                    }
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(err)?;
                    if !valid_xml_chars(&value) {
                        return Err(err("Invalid XML attribute character"));
                    }
                }
                depth += 1;
                if depth > 128 || roots > 1 {
                    return Err(err("XML depth/root bound exceeded"));
                }
            }
            Event::End(e) => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| err("Unexpected XML closing tag"))?;
                if e.local_name().as_ref() == b"p" {
                    text.push('\n');
                }
            }
            Event::Text(e) => {
                if e.as_ref().windows(3).any(|v| v == b"]]>") {
                    return Err(err("CDATA delimiter in XML text"));
                }
                let value = e.xml_content().map_err(err)?;
                if depth == 0 && !value.trim().is_empty() {
                    return Err(err("Text outside XML root"));
                }
                text.push_str(&quick_xml::escape::unescape(&value).map_err(err)?);
            }
            Event::CData(e) => {
                if depth == 0 {
                    return Err(err("CDATA outside XML root"));
                }
                text.push_str(std::str::from_utf8(e.as_ref()).map_err(err)?);
            }
            Event::GeneralRef(e) => {
                if depth == 0 {
                    return Err(err("Entity outside XML root"));
                }
                let reference = format!("&{};", std::str::from_utf8(e.as_ref()).map_err(err)?);
                text.push_str(&quick_xml::escape::unescape(&reference).map_err(err)?);
            }
            Event::DocType(_) => {
                return Err(err("XML DTDs and external entities are not supported"));
            }
            Event::Decl(e) => {
                if events != 0 || e.version().map_err(err)?.as_ref() != b"1.0" {
                    return Err(err("Only an initial XML 1.0 declaration is supported"));
                }
                if let Some(encoding) = e.encoding() {
                    let encoding = encoding.map_err(err)?;
                    if !encoding.eq_ignore_ascii_case(b"utf-8") {
                        return Err(err("XML encoding must be UTF-8"));
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        events += 1;
    }
    if roots != 1 || depth != 0 {
        return Err(err("XML must have one complete root"));
    }
    if !valid_xml_chars(&text) {
        return Err(err("Invalid XML text character"));
    }
    Ok((root, text))
}

fn package(format: &str, bytes: &[u8]) -> io::Result<(Value, String)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
    if archive.len() > 512 {
        return Err(err("ZIP exceeds 512 entry limit"));
    }
    let mut names = BTreeSet::new();
    let mut expanded = 0usize;
    let mut document_text = String::new();
    let mut mimetype = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(err)?;
        let name = entry.name().to_string();
        if name.contains(['\\', ':'])
            || name.starts_with('/')
            || name.split('/').any(|p| p == "..")
            || !names.insert(name.clone())
            || entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)
        {
            return Err(err("ZIP has ambiguous/unsafe entries"));
        }
        if entry.size() > MAX_ENTRY as u64 {
            return Err(err("ZIP entry exceeds 8 MiB"));
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_ENTRY as u64 + 1)
            .read_to_end(&mut data)
            .map_err(err)?;
        expanded = expanded
            .checked_add(data.len())
            .ok_or_else(|| err("ZIP size overflow"))?;
        if data.len() > MAX_ENTRY || expanded > MAX_EXPANDED || data.len() as u64 != entry.size() {
            return Err(err("ZIP decompression bound exceeded"));
        }
        if format != "zip" && (name.ends_with(".xml") || name.ends_with(".rels")) {
            let (root, text) = xml(&data)?;
            let expected = match name.as_str() {
                "[Content_Types].xml" => Some("Types"),
                "_rels/.rels" => Some("Relationships"),
                "word/document.xml" => Some("document"),
                "xl/workbook.xml" => Some("workbook"),
                "ppt/presentation.xml" => Some("presentation"),
                "content.xml" => Some("document-content"),
                _ => None,
            };
            if expected.is_some_and(|expected| root != expected) {
                return Err(err("Unexpected package XML root"));
            }
            if name == "word/document.xml" {
                document_text = text;
            }
        }
        if name == "mimetype" {
            mimetype = data;
        }
    }
    let required: &[&str] = match format {
        "docx" => &["[Content_Types].xml", "_rels/.rels", "word/document.xml"],
        "xlsx" => &["[Content_Types].xml", "_rels/.rels", "xl/workbook.xml"],
        "pptx" => &["[Content_Types].xml", "_rels/.rels", "ppt/presentation.xml"],
        "odt" | "ods" | "odp" => &["mimetype", "content.xml", "META-INF/manifest.xml"],
        _ => &[],
    };
    if required.iter().any(|name| !names.contains(*name)) {
        return Err(err("Missing required package parts"));
    }
    let media = match format {
        "odt" => Some("text"),
        "ods" => Some("spreadsheet"),
        "odp" => Some("presentation"),
        _ => None,
    };
    if media.is_some_and(|media| {
        mimetype != format!("application/vnd.oasis.opendocument.{media}").as_bytes()
    }) {
        return Err(err("Wrong OpenDocument mimetype"));
    }
    Ok((
        json!({"status":"checked","format":format,"scope":"zip_crc_and_required_package_structure_only","entries":names.len(),"expanded_bytes":expanded,"content_verified":false,"limitations":"Not full application schemas, relationships, formulas, rendering or meaningful content"}),
        document_text,
    ))
}

fn docx(title: Option<&str>, paragraphs: &[String]) -> io::Result<Vec<u8>> {
    if paragraphs.is_empty() || paragraphs.len() > 1024 || title.is_some_and(|t| t.len() > 1000) {
        return Err(err(
            "Provide 1–1024 paragraphs and an optional title up to 1000 bytes",
        ));
    }
    let values: Vec<&str> = title
        .into_iter()
        .chain(paragraphs.iter().map(String::as_str))
        .collect();
    if values.iter().map(|s| s.len()).sum::<usize>() > 1024 * 1024 || values.iter().any(|s| s.chars().any(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))) { return Err(err("DOCX text exceeds bounds or contains invalid XML characters")); }
    let mut document = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
    );
    for value in &values {
        document.push_str(&format!(
            "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
            quick_xml::escape::escape(*value)
        ));
    }
    document.push_str("<w:sectPr/></w:body></w:document>");
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, text) in [
        (
            "[Content_Types].xml",
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>",
        ),
        (
            "_rels/.rels",
            "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>",
        ),
        ("word/document.xml", document.as_str()),
    ] {
        writer.start_file(name, options).map_err(err)?;
        writer.write_all(text.as_bytes())?;
    }
    let bytes = writer.finish().map_err(err)?.into_inner();
    let (_, actual) = package("docx", &bytes)?;
    let expected = values
        .iter()
        .map(|s| format!("{}\n", s.replace("\r\n", "\n").replace('\r', "\n")))
        .collect::<String>();
    if actual != expected {
        return Err(err("Generated DOCX text did not match requested content"));
    }
    Ok(bytes)
}

pub fn read_document(workspace: &Path, path: &str) -> io::Result<Value> {
    let bytes = read_bytes(&resolve(workspace, path, false)?)?;
    let (structure, mut text) = package("docx", &bytes)?;
    let truncated = text.len() > 60000;
    if truncated {
        let mut end = 60000;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Ok(
        json!({"path":path,"format":"docx","text":text,"truncated":truncated,"sha256":digest(&bytes),"structure":structure}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(root: &Path, action: Value) -> io::Result<Value> {
        let stop = AtomicBool::new(false);
        execute(prepare(root, &action, false, &stop)?, root, &stop)
    }
    #[test]
    fn binary_publication_pages_receipts_and_no_clobber() {
        let root = tempfile::tempdir().unwrap();
        let write =
            json!({"tool":"artifact_write","path":"bytes.bin","encoding":"base64","data":"AP9h"});
        let result = run(root.path(), write.clone()).unwrap();
        assert_eq!(
            fs::read(root.path().join("bytes.bin")).unwrap(),
            [0, 255, 97]
        );
        assert_eq!(result["verification_scope"], "byte_integrity_only");
        assert_eq!(result["structure"]["status"], "adapter_required");
        assert_eq!(
            run(root.path(), write.clone()).unwrap()["already_matches"],
            true
        );
        let mut different = write.clone();
        different["data"] = json!("different");
        different["encoding"] = json!("utf8");
        assert!(run(root.path(), different).is_err());
        let page = run(
            root.path(),
            json!({"tool":"artifact_read","path":"bytes.bin","offset":1,"length":1}),
        )
        .unwrap();
        assert_eq!(page["data"], "/w==");
        assert_eq!(page["next_offset"], 2);
        assert_eq!(
            run(
                root.path(),
                json!({"tool":"artifact_verify","path":"bytes.bin"})
            )
            .unwrap()["ok"],
            false
        );
        assert_eq!(run(root.path(), json!({"tool":"artifact_verify","path":"bytes.bin","expected_sha256":digest(&[0,255,97])})).unwrap()["ok"], true);
        assert_eq!(
            fs::read(root.path().join("bytes.bin")).unwrap(),
            [0, 255, 97]
        );
    }
    #[test]
    fn pending_intent_reconciles_exact_bytes_without_replay() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let prepared = prepare(
            root.path(),
            &json!({"tool":"artifact_write","path":"saved.bin","encoding":"base64","data":"AP9h"}),
            false,
            &stop,
        )
        .unwrap();
        let intent = prepared.intent().unwrap();
        assert!(reconcile(root.path(), &intent).is_none());
        execute(prepared, root.path(), &stop).unwrap();
        assert_eq!(reconcile(root.path(), &intent).unwrap()["reconciled"], true);
        fs::write(root.path().join("saved.bin"), [0, 255]).unwrap();
        assert!(reconcile(root.path(), &intent).is_none());
        assert_eq!(fs::read(root.path().join("saved.bin")).unwrap(), [0, 255]);
    }
    #[test]
    fn malformed_denied_and_cancelled_operations_do_not_publish() {
        let root = tempfile::tempdir().unwrap();
        let stop = AtomicBool::new(false);
        let write =
            json!({"tool":"artifact_write","path":"new.bin","encoding":"utf8","data":"safe"});
        assert!(prepare(root.path(), &write, true, &stop).is_err());
        for action in [
            json!({"tool":"artifact_write","path":"../escape.bin","encoding":"utf8","data":"bad"}),
            json!({"tool":"artifact_write","path":"new.bin","encoding":"base64","data":"!"}),
            json!({"tool":"artifact_read","path":"new.bin","length":8193}),
        ] {
            assert!(prepare(root.path(), &action, false, &stop).is_err());
        }
        let prepared = prepare(root.path(), &write, false, &stop).unwrap();
        stop.store(true, Ordering::SeqCst);
        assert!(execute(prepared, root.path(), &stop).is_err());
        assert!(!root.path().join("new.bin").exists());
    }
    #[test]
    fn native_docx_roundtrips_unicode_and_rejects_fake_packages() {
        let root = tempfile::tempdir().unwrap();
        run(root.path(), json!({"tool":"document_create","path":"story.docx","title":"Title & <text>","paragraphs":["Tiếng Việt 😀","Another paragraph"]})).unwrap();
        let doc = read_document(root.path(), "story.docx").unwrap();
        assert_eq!(
            doc["text"],
            "Title & <text>\nTiếng Việt 😀\nAnother paragraph\n"
        );
        assert_eq!(doc["structure"]["content_verified"], false);
        for ext in ["docx", "xlsx", "pptx", "zip"] {
            assert!(inspect_bytes(&format!("fake.{ext}"), b"ordinary prose").is_err());
        }
    }
    #[test]
    fn bounded_structure_validation_rejects_invalid_inputs() {
        for data in [
            "<a><b></a>",
            "<a/><b/>",
            "<!DOCTYPE a><a/>",
            "<a>&unknown;</a>",
            "<a>\0</a>",
            "<1invalid/>",
            "<a attr='<bad'/>",
            "<a>&#1;</a>",
            "<a>]]></a>",
        ] {
            assert!(inspect_bytes("bad.xml", data.as_bytes()).is_err(), "{data}");
        }
        assert!(inspect_bytes("bad.json", b"{broken}").is_err());
        assert!(inspect_bytes("bad.svg", b"<html/>").is_err());
        assert!(inspect_bytes("ok.svg", b"<svg><path/></svg>").is_ok());
        let bytes = docx(None, &["Text".into()]).unwrap();
        assert!(inspect_bytes("wrong.xlsx", &bytes).is_err());
        let mut corrupt = bytes.clone();
        let pos = corrupt.windows(5).position(|v| v == b"<w:p>").unwrap();
        corrupt[pos + 1] = b'x';
        assert!(inspect_bytes("bad.zip", &corrupt).is_err());
    }

    #[test]
    fn file_page_and_decompression_limits_are_enforced() {
        let root = tempfile::tempdir().unwrap();
        let file = fs::File::create(root.path().join("large.bin")).unwrap();
        file.set_len(MAX_BYTES as u64 + 1).unwrap();
        drop(file);
        assert!(
            run(
                root.path(),
                json!({"tool":"artifact_read","path":"large.bin"})
            )
            .is_err()
        );
        fs::write(root.path().join("pages.bin"), vec![7; 8193]).unwrap();
        let page = run(
            root.path(),
            json!({"tool":"artifact_read","path":"pages.bin"}),
        )
        .unwrap();
        assert_eq!(
            STANDARD
                .decode(page["data"].as_str().unwrap())
                .unwrap()
                .len(),
            8192
        );
        assert_eq!(page["next_offset"], 8192);
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "large",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(&vec![0; MAX_ENTRY + 1]).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert!(inspect_bytes("too-expanded.zip", &bytes).is_err());
    }
}
