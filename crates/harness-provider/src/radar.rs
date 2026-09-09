//! Repo radar: scan public open-source listings through the supervised
//! fetch boundary and render a digest.
//!
//! The only source wired in is the GitHub repository search API over
//! `api.github.com` (JSON, no credentials). Rendered digests are labeled
//! untrusted network data: verify anything before acting on it.

use harness_core::permissions::PermissionPolicy;
use harness_core::tools::ToolRegistry;
use harness_core::types::Action;

/// GitHub-flavored query encoding: unreserved marks plus `:`/`>`/`/` pass
/// through (search operators), spaces become `+`, everything else is
/// percent-encoded.
pub fn encode_query(query: &str) -> String {
    let mut out = String::new();
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b':'
            | b'>'
            | b'/' => out.push(byte as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Date `days` ago as `YYYY-MM-DD`, for `created:>` ranges. Dependency-free
/// civil-date conversion (Howard Hinnant's algorithm).
pub fn days_ago_ymd(days_ago: u64) -> String {
    let days = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400)
        .saturating_sub(days_ago) as i64;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (day_of_year * 5 + 2) / 153;
    let day = day_of_year - (month * 153 + 2) / 5 + 1;
    let (year, month) = if month < 10 {
        (year, month + 3)
    } else {
        (year + 1, month - 9)
    };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Default scan: repositories created in the last 30 days, hottest first.
pub fn default_query() -> String {
    format!("created:>{}", days_ago_ymd(30))
}

/// Repository search URL. `per_page` clamps to GitHub's 1..=100 range.
pub fn github_search_url(query: &str, per_page: u8) -> String {
    let per_page = per_page.clamp(1, 100);
    format!(
        "https://api.github.com/search/repositories?q={}&sort=stars&order=desc&per_page={per_page}",
        encode_query(query)
    )
}

/// Fetch through the permission-checked registry. Returns the body text or
/// the denial/failure reason; credentials are never attached.
pub fn fetch_text(
    registry: &ToolRegistry,
    policy: &PermissionPolicy,
    url: &str,
) -> Result<String, String> {
    let observation = registry.execute(&Action::FetchUrl { url: url.into() }, policy);
    if observation.ok {
        Ok(observation.data)
    } else {
        Err(format!("{}: {}", observation.summary, observation.data))
    }
}

/// Render a GitHub search payload as a digest table. Strict about shape:
/// missing `items` or wrong types fail instead of guessing.
pub fn render_digest(payload: &str, limit: usize) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(payload).map_err(|e| format!("invalid radar JSON: {e}"))?;
    let items = value
        .get("items")
        .and_then(|items| items.as_array())
        .ok_or_else(|| "radar payload has no items array".to_string())?;
    let total = value
        .get("total_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let mut out =
        format!("# Repo radar ({total} matches; untrusted network data — verify before acting)\n");
    for (index, item) in items.iter().take(limit.max(1)).enumerate() {
        let name = item
            .get("full_name")
            .and_then(|value| value.as_str())
            .unwrap_or("?");
        let stars = item
            .get("stargazers_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let language = item
            .get("language")
            .and_then(|value| value.as_str())
            .unwrap_or("—");
        let description = item
            .get("description")
            .and_then(|value| value.as_str())
            .unwrap_or("—");
        let url = item
            .get("html_url")
            .and_then(|value| value.as_str())
            .unwrap_or("?");
        out.push_str(&format!(
            "{}. {name} ★{stars} ({language}) — {description}\n   {url}\n",
            index + 1
        ));
    }
    Ok(out)
}
