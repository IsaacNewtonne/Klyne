//! Bounded structural acceptance, never factual or source-quality certification.
use crate::{PermissionDecision, PermissionPolicy, types::Action};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRequirements {
    pub path: String,
    pub min_content_chars: usize,
    pub required_headings: Vec<String>,
    pub source_references: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    Failed,
    Inconclusive,
}

impl ArtifactRequirements {
    pub fn validate(&self) -> io::Result<()> {
        // One portable, unambiguous workspace-relative spelling. The policy
        // also checks resolved paths (including existing symlinks) at use time.
        if self.path.len() > 1024
            || self.path.contains(['\\', ':'])
            || self.path.chars().any(char::is_control)
            || self
                .path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p.trim() != p)
            || !(self.path.ends_with(".md") || self.path.ends_with(".markdown"))
            || self.min_content_chars == 0
            || self.min_content_chars > crate::tools::MAX_FILE_BYTES
            || self.required_headings.is_empty()
            || self.required_headings.len() > 32
            || self.source_references.len() > 32
        {
            return Err(io::Error::other(
                "Invalid artifact path or bounds: use a relative .md/.markdown path, 1–1048576 content characters, 1–32 headings and at most 32 source references",
            ));
        }
        let mut headings = BTreeSet::new();
        for heading in &self.required_headings {
            if heading.is_empty()
                || heading.len() > 200
                || heading.chars().any(char::is_control)
                || normalize(heading) != *heading
                || !headings.insert(heading)
            {
                return Err(io::Error::other(
                    "Required headings must be unique, normalized visible text of 1–200 bytes",
                ));
            }
        }
        let mut sources = BTreeSet::new();
        for source in &self.source_references {
            if source.len() > 2048
                || !source.starts_with("https://")
                || source.chars().any(|c| {
                    c.is_whitespace() || c.is_control() || matches!(c, '\\' | '<' | '>' | '"')
                })
                || !url::Url::parse(source).is_ok_and(|url| {
                    url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                })
                || !sources.insert(source)
            {
                return Err(io::Error::other(
                    "Source references must be unique HTTPS URLs without credentials or whitespace",
                ));
            }
        }
        Ok(())
    }

    pub fn validate_scope(&self, policy: &PermissionPolicy) -> io::Result<()> {
        self.validate()?;
        let action = Action::ReadFile {
            path: self.path.clone(),
        };
        if policy.check(&action) != PermissionDecision::Allow {
            return Err(io::Error::other(
                "Artifact path is outside the allowed workspace",
            ));
        }
        policy
            .resolve_workspace_path(&self.path)
            .map_err(io::Error::other)?;
        Ok(())
    }

    pub fn check(&self, text: &str) -> (CheckOutcome, String) {
        if let Err(error) = self.validate() {
            return (CheckOutcome::Failed, error.to_string());
        }
        if text.len() > crate::tools::MAX_FILE_BYTES {
            return (
                CheckOutcome::Inconclusive,
                "Report exceeds the bounded read limit".into(),
            );
        }
        let mut headings = BTreeSet::new();
        let mut links = BTreeSet::new();
        let mut heading = None::<String>;
        let mut excluded = 0usize;
        let mut containers = 0usize;
        let mut content_chars = 0usize;
        for event in Parser::new(text) {
            match event {
                Event::Start(Tag::CodeBlock(_) | Tag::Image { .. }) => excluded += 1,
                Event::End(TagEnd::CodeBlock | TagEnd::Image) => {
                    excluded = excluded.saturating_sub(1)
                }
                _ if excluded > 0 => {}
                Event::Html(_) | Event::InlineHtml(_) => {
                    return (CheckOutcome::Inconclusive, "Report contains raw HTML; use Markdown so visible content can be checked independently".into());
                }
                Event::Start(Tag::BlockQuote(_) | Tag::List(_)) => containers += 1,
                Event::End(TagEnd::BlockQuote(_) | TagEnd::List(_)) => {
                    containers = containers.saturating_sub(1)
                }
                Event::Start(Tag::Heading { .. }) => heading = Some(String::new()),
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(value) = heading.take().filter(|_| containers == 0) {
                        headings.insert(normalize(&value));
                    }
                }
                Event::Start(Tag::Link { dest_url, .. }) => {
                    links.insert(dest_url.to_string());
                }
                Event::Text(value) | Event::Code(value) => {
                    if let Some(heading) = &mut heading {
                        heading.push_str(&value);
                    } else {
                        content_chars += value.chars().filter(|c| !c.is_whitespace()).count();
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if let Some(heading) = &mut heading {
                        heading.push(' ');
                    }
                }
                _ => {}
            }
        }
        let missing_headings: Vec<_> = self
            .required_headings
            .iter()
            .filter(|h| !headings.contains(*h))
            .collect();
        let missing_sources: Vec<_> = self
            .source_references
            .iter()
            .filter(|s| !links.contains(*s))
            .collect();
        if content_chars < self.min_content_chars
            || !missing_headings.is_empty()
            || !missing_sources.is_empty()
        {
            return (
                CheckOutcome::Failed,
                format!(
                    "Report '{}': visible body characters {content_chars}/{}; missing headings {missing_headings:?}; missing source links {missing_sources:?}",
                    self.path, self.min_content_chars
                ),
            );
        }
        (
            CheckOutcome::Passed,
            format!(
                "Independently checked report '{}': {content_chars} visible body characters, {} required headings and {} required source links. Structural requirements only; factual accuracy, source retrieval and source quality are not verified.",
                self.path,
                self.required_headings.len(),
                self.source_references.len()
            ),
        )
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
