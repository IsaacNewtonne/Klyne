//! Frozen regression reference. This detects changes to the checks, not
//! adversarial execution by a process with access to the host filesystem.
use quote::ToTokens;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io, path::Path};

#[derive(Default)]
pub(crate) struct Reference {
    files: BTreeMap<String, Option<Vec<u8>>>,
    tests: BTreeMap<String, String>,
}

fn rust_tests(path: &str, bytes: &[u8]) -> io::Result<BTreeMap<String, String>> {
    let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
    let file = syn::parse_file(text).map_err(io::Error::other)?;
    let mut found = BTreeMap::new();
    found.insert(
        format!("{path}:file_attributes"),
        file.attrs
            .iter()
            .map(|a| a.to_token_stream().to_string())
            .collect(),
    );
    fn items(prefix: &str, source: &[syn::Item], into: &mut BTreeMap<String, String>) {
        for item in source {
            match item {
                syn::Item::Mod(module) => {
                    let key = format!("{prefix}::{}", module.ident);
                    let attrs = module
                        .attrs
                        .iter()
                        .map(|a| a.to_token_stream().to_string())
                        .collect::<String>();
                    if attrs.contains("test") {
                        into.insert(key, module.to_token_stream().to_string());
                    } else {
                        into.insert(format!("{key}:attributes"), attrs);
                        if let Some((_, children)) = &module.content {
                            items(&key, children, into);
                        }
                    }
                }
                syn::Item::Fn(function)
                    if function.attrs.iter().any(|a| {
                        a.path().segments.last().is_some_and(|s| s.ident == "test")
                            || a.to_token_stream().to_string().contains("test")
                    }) =>
                {
                    into.insert(
                        format!("{prefix}::{}", function.sig.ident),
                        function.to_token_stream().to_string(),
                    );
                }
                // A macro can generate tests; retain its baseline invocation.
                syn::Item::Macro(m) => {
                    into.insert(
                        format!("{prefix}:macro:{}", into.len()),
                        m.to_token_stream().to_string(),
                    );
                }
                _ => {}
            }
        }
    }
    items(path, &file.items, &mut found);
    Ok(found)
}

fn read(root: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let mut path = root.to_path_buf();
    for part in Path::new(name).components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(io::Error::other("Invalid reference path"));
        }
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(io::Error::other("Reparse reference refused"));
                    }
                }
                if meta.file_type().is_symlink() {
                    return Err(io::Error::other("Symlink reference refused"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
        return Err(io::Error::other("Reference file exceeds 16 MiB"));
    }
    fs::read(path).map(Some)
}

impl Reference {
    pub(crate) fn capture(root: &Path) -> io::Result<Self> {
        let mut reference = Self::default();
        let files = super::git(root, &["ls-files", "-z"])?;
        for name in files.split('\0').filter(|n| !n.is_empty()) {
            let filename = Path::new(name)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let bytes = read(root, name)?;
            if name.split('/').any(|p| p == "tests" || p == ".cargo")
                || matches!(
                    filename.as_ref(),
                    "Cargo.toml"
                        | "Cargo.lock"
                        | "build.rs"
                        | "rust-toolchain"
                        | "rust-toolchain.toml"
                )
            {
                reference.files.insert(name.into(), bytes);
            } else if name.ends_with(".rs") {
                let bytes = bytes.ok_or_else(|| io::Error::other("Missing tracked Rust source"))?;
                reference.tests.extend(rust_tests(name, &bytes)?);
            }
        }
        for name in [
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            ".cargo/config",
            ".cargo/config.toml",
            "rust-toolchain",
            "rust-toolchain.toml",
        ] {
            reference.files.insert(name.into(), read(root, name)?);
        }
        Ok(reference)
    }

    pub(crate) fn verify(&self, root: &Path) -> io::Result<()> {
        for (name, expected) in &self.files {
            if &read(root, name)? != expected {
                return Err(io::Error::other(format!(
                    "Protected evaluation input changed: {name}"
                )));
            }
        }
        let current = Self::capture(root)?;
        for name in current.files.keys() {
            if !self.files.contains_key(name) && !name.split('/').any(|part| part == "tests") {
                return Err(io::Error::other(format!(
                    "New evaluation configuration requires host review: {name}"
                )));
            }
        }
        for (name, expected) in &self.tests {
            if current.tests.get(name) != Some(expected) {
                return Err(io::Error::other(format!(
                    "Protected Rust test or configuration changed: {name}"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn digest(&self) -> String {
        let bytes =
            serde_json::to_vec(&(&self.files, &self.tests)).expect("serializable reference");
        format!("{:x}", Sha256::digest(bytes))
    }
}
