//! Web reach (2026-10-25): typed web errors, anti-bot challenge detection,
//! a Jina Reader fallback, GitHub/YouTube routing inside `fetch_url`, and
//! the `video_transcript` tool.
//!
//! Inspired by Agent-Reach (MIT, github.com/Panniantong/Agent-Reach) and
//! reimplemented natively; Klyne does not depend on it. Rules: no cookies,
//! no logged-in scraping, no stealth browser. Fetched text is untrusted
//! data. The original URL is permission-checked by the caller; the fixed
//! host-owned backends (r.jina.ai, api.github.com, raw.githubusercontent.com
//! and yt-dlp) only ever receive that public URL, plus an optional GitHub
//! token the user granted for https://api.github.com.
use serde_json::{Value, json};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Typed failure classes shown to the model with a one-line hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    NeedLogin,
    RateLimited,
    PlatformBlocked,
    NotFound,
    Network,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NeedLogin => "NEED_LOGIN",
            Self::RateLimited => "RATE_LIMITED",
            Self::PlatformBlocked => "PLATFORM_BLOCKED",
            Self::NotFound => "NOT_FOUND",
            Self::Network => "NETWORK",
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "NEED_LOGIN" => Self::NeedLogin,
            "RATE_LIMITED" => Self::RateLimited,
            "PLATFORM_BLOCKED" => Self::PlatformBlocked,
            "NOT_FOUND" => Self::NotFound,
            "NETWORK" => Self::Network,
            _ => return None,
        })
    }
    /// Retrying the same source cannot help. NETWORK may be transient, so
    /// it gets one more attempt before the host treats it as blocked too.
    pub fn final_for_source(self) -> bool {
        self != Self::Network
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::NeedLogin => {
                "This source needs a sign-in. Klyne does not use logins or cookies: ask the user to paste the content or give a public link."
            }
            Self::RateLimited => {
                "The site is rate limiting requests. Do not retry now: use what you already have, or try again later."
            }
            Self::PlatformBlocked => {
                "The site blocked automated access (anti-bot check). Do not retry it: use another public source or ask the user to paste the content."
            }
            Self::NotFound => {
                "The page or item does not exist. Check the link or id; do not retry the same URL."
            }
            Self::Network => {
                "Network error reaching the site. Retry once at most; if it fails again, report it."
            }
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct WebError {
    pub code: ErrorCode,
    pub hint: String,
}

impl WebError {
    fn new(code: ErrorCode) -> Self {
        Self {
            code,
            hint: code.hint().to_owned(),
        }
    }
    fn with_hint(code: ErrorCode, hint: impl Into<String>) -> Self {
        Self {
            code,
            hint: hint.into(),
        }
    }
}

/// One tool result plus the metadata recorded in evidence.
#[derive(Clone, Debug)]
pub struct Reach {
    pub ok: bool,
    pub summary: String,
    pub data: String,
    /// direct | jina | github_api | github_raw | yt-dlp | host
    pub backend: &'static str,
    /// web | github | youtube | video_transcript
    pub route: &'static str,
    pub error: Option<WebError>,
    pub fallback_reason: Option<String>,
    pub source: String,
}

impl Reach {
    fn ok(route: &'static str, backend: &'static str, summary: String, data: String) -> Self {
        Self {
            ok: true,
            summary,
            data,
            backend,
            route,
            error: None,
            fallback_reason: None,
            source: String::new(),
        }
    }
    fn fail(
        route: &'static str,
        backend: &'static str,
        error: Option<WebError>,
        detail: impl Into<String>,
    ) -> Self {
        let detail = detail.into();
        let (summary, data) = match &error {
            Some(e) => (
                format!("{}: {detail}", e.code),
                format!("{}: {detail}\nHint: {}", e.code, e.hint),
            ),
            None => (detail.clone(), detail),
        };
        Self {
            ok: false,
            summary,
            data,
            backend,
            route,
            error,
            fallback_reason: None,
            source: String::new(),
        }
    }
    /// Extra evidence fields: which backend answered and any typed error.
    pub fn meta(&self) -> Value {
        let mut meta = json!({"backend":self.backend,"route":self.route,"source":self.source});
        if let Some(error) = &self.error {
            meta["error_code"] = json!(error.code.as_str());
            meta["hint"] = json!(error.hint);
        }
        if let Some(reason) = &self.fallback_reason {
            meta["fallback_reason"] = json!(reason);
        }
        meta
    }
    /// Merge `meta()` into an evidence entry.
    pub fn annotate(&self, entry: &mut Value) {
        if let (Some(target), Value::Object(meta)) = (entry.as_object_mut(), self.meta()) {
            target.extend(meta);
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub jina_fallback: bool,
    pub github_token: Option<String>,
    pub ytdlp: Option<PathBuf>,
    pub jina_base: String,
    pub github_api: String,
    pub github_raw: String,
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl Options {
    pub fn new(jina_fallback: bool, github_token: Option<String>, ytdlp: Option<PathBuf>) -> Self {
        Self {
            jina_fallback,
            github_token,
            ytdlp,
            jina_base: "https://r.jina.ai".into(),
            github_api: "https://api.github.com".into(),
            github_raw: "https://raw.githubusercontent.com".into(),
            timeout: Duration::from_secs(30),
            max_bytes: 512 * 1024,
        }
    }
}

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) klyne-studio/0.1";
/// Characters of text kept in one result (evidence keeps ~12k).
const MAX_TEXT: usize = 11_000;

struct Http {
    status: u16,
    headers: reqwest::header::HeaderMap,
    body: String,
    url: String,
}

fn client(timeout: Duration) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .tls_built_in_webpki_certs(true)
        // Same-host redirects only: the permission check covered this host.
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let first = attempt.previous().first().cloned();
            let same = first.is_some_and(|f| {
                f.host_str() == attempt.url().host_str() && f.scheme() == attempt.url().scheme()
            });
            if attempt.previous().len() >= 5 || !same {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| format!("client build: {e}"))
}

fn get(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &[(&str, String)],
    max: usize,
) -> Result<Http, String> {
    let mut request = client.get(url).header("User-Agent", USER_AGENT);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request.send().map_err(|e| {
        let text = e.to_string();
        if e.is_timeout() {
            format!("timed out: {text}")
        } else {
            text
        }
    })?;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let final_url = response.url().to_string();
    let mut bytes = Vec::new();
    response
        .take(max as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(Http {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
        url: final_url,
    })
}

fn header<'a>(http: &'a Http, name: &str) -> &'a str {
    http.headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

fn clip(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_owned(), false);
    }
    (text.chars().take(max).collect(), true)
}

fn short(text: &str) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    clip(&one, 240).0
}

/// Lower-cased visible text of an HTML page (scripts, styles and tags
/// removed). Used only for size and phrase heuristics.
pub fn visible_text(html: &str) -> String {
    let lower = html.to_lowercase();
    if !lower.contains('<') {
        return lower.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let mut out = String::new();
    let mut i = 0;
    while i < lower.len() {
        if lower.as_bytes()[i] == b'<' {
            let skip_to = ["script", "style", "noscript", "template", "svg"]
                .iter()
                .find(|t| lower[i + 1..].starts_with(*t))
                .and_then(|t| lower[i..].find(&format!("</{t}")).map(|end| i + end));
            let from = skip_to.unwrap_or(i);
            match lower[from..].find('>') {
                Some(e) => {
                    out.push(' ');
                    i = from + e + 1;
                }
                None => break,
            }
            continue;
        }
        let next = lower[i..].find('<').map(|n| i + n).unwrap_or(lower.len());
        out.push_str(&lower[i..next]);
        i = next;
    }
    decode_entities(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// High-confidence anti-bot / challenge page (Cloudflare, DataDome,
/// PerimeterX, Imperva, DDoS-Guard, Jina's captcha warning). Such pages are
/// never evidence. `cf_mitigated` is the `cf-mitigated` response header.
pub fn is_challenge(body: &str, cf_mitigated: &str) -> bool {
    if cf_mitigated.eq_ignore_ascii_case("challenge") {
        return true;
    }
    let sample = body
        .chars()
        .take(32 * 1024)
        .collect::<String>()
        .to_lowercase();
    const STRONG: &[&str] = &[
        "<title>just a moment...</title>",
        "title: just a moment...",
        "/cdn-cgi/challenge-platform/",
        "cf-browser-verification",
        "cf_chl_opt",
        "attention required! | cloudflare",
        "captcha-delivery.com",
        "_incapsula_resource",
        "px-captcha",
        "ddos-guard.net/",
        "requiring captcha",
    ];
    if STRONG.iter().any(|m| sample.contains(m)) {
        return true;
    }
    // Weaker phrases count only on short pages, so an article that merely
    // mentions them is not mistaken for a challenge.
    const WEAK: &[&str] = &[
        "performing security verification",
        "checking your browser before accessing",
        "verify you are a human",
        "verifying you are human",
        "enable javascript and cookies to continue",
        "please complete the security check",
        "are you a robot",
        "unusual traffic from your computer network",
        "checking if the site connection is secure",
        "verify you are human",
        "pardon our interruption",
        "access to this page has been denied",
        "press & hold to confirm",
        "incapsula incident id",
    ];
    WEAK.iter().any(|m| sample.contains(m)) && visible_text(&sample).chars().count() < 3000
}

/// Too little visible text to be the real page (JS-only app shell).
pub fn near_empty(body: &str) -> bool {
    let lower = body.to_lowercase();
    if lower.contains("you need to enable javascript to run this app")
        || lower.contains("please enable javascript to continue")
    {
        return true;
    }
    let html = lower.contains("<html") || lower.contains("<body") || lower.contains("<script");
    if html {
        visible_text(body).chars().count() < 200
    } else {
        body.trim().chars().count() < 20
    }
}

/// Typed classification of an HTTP status (None when it is not an error
/// Klyne can type, e.g. 400).
pub fn classify_status(status: u16, retry_after: &str) -> Option<WebError> {
    Some(match status {
        401 | 407 => WebError::new(ErrorCode::NeedLogin),
        403 | 451 => WebError::new(ErrorCode::PlatformBlocked),
        404 | 410 => WebError::new(ErrorCode::NotFound),
        429 => {
            let wait = retry_after.trim();
            if wait.is_empty() {
                WebError::new(ErrorCode::RateLimited)
            } else {
                WebError::with_hint(
                    ErrorCode::RateLimited,
                    format!(
                        "The site is rate limiting requests (retry after {wait}). Do not retry now: use what you already have."
                    ),
                )
            }
        }
        500..=599 => WebError::with_hint(
            ErrorCode::Network,
            "The site returned a server error. Retry once at most; if it fails again, report it.",
        ),
        _ => return None,
    })
}

fn login_like(location: &str) -> bool {
    let l = location.to_lowercase();
    [
        "/login",
        "/signin",
        "/sign-in",
        "/sign_in",
        "/auth",
        "accounts.",
        "/sso",
        "/session",
    ]
    .iter()
    .any(|m| l.contains(m))
}

/// Canonical source key used to recognise repeats of a blocked source.
pub fn source_key(tool: &str, target: &str, lang: Option<&str>) -> String {
    let mut target = target.trim().to_owned();
    if let Some(hash) = target.find('#') {
        target.truncate(hash);
    }
    let target = target.trim_end_matches('/');
    let target = youtube_id(target)
        .map(|id| format!("youtube:{id}"))
        .unwrap_or_else(|| target.to_owned());
    match lang {
        Some(lang) if !lang.is_empty() => format!("{tool}:{target}:{lang}"),
        _ => format!("{tool}:{target}"),
    }
}

/// Fetch a URL for the `fetch_url` tool: GitHub/YouTube routing, direct
/// fetch, then the Jina Reader fallback when the direct result is a
/// failure, near-empty/JS-only or an anti-bot challenge.
pub fn fetch(url: &str, options: &Options, stop: &AtomicBool) -> Reach {
    let mut reach = match github_route(url) {
        Some(GitHub::Blob { owner, repo, rest }) => {
            let raw = format!("{}/{owner}/{repo}/{rest}", options.github_raw);
            let mut r = fetch_web(&raw, options);
            if r.backend == "direct" {
                r.backend = "github_raw";
            }
            r.route = "github";
            r
        }
        Some(route) => match github(&route, options) {
            Ok(r) => r,
            // API unreachable: fall back to the ordinary page fetch.
            Err(reason) => {
                let mut r = fetch_web(url, options);
                r.fallback_reason = Some(reason);
                r
            }
        },
        None if youtube_id(url).is_some() => youtube(url, None, options, stop, false),
        None => fetch_web(url, options),
    };
    reach.source = source_key("fetch_url", url, None);
    reach
}

fn fetch_web(url: &str, options: &Options) -> Reach {
    let client = match client(options.timeout) {
        Ok(c) => c,
        Err(e) => return Reach::fail("web", "direct", None, e),
    };
    // Why the direct result was not accepted, and what to report if the
    // fallback does not help.
    let (reason, direct) = match get(&client, url, &[], options.max_bytes) {
        Err(e) => (
            format!("direct fetch failed ({})", short(&e)),
            Reach::fail(
                "web",
                "direct",
                Some(WebError::new(ErrorCode::Network)),
                format!("could not reach {url} ({})", short(&e)),
            ),
        ),
        Ok(http) if (300..400).contains(&http.status) => {
            let location = header(&http, "location");
            let target = reqwest::Url::parse(&http.url)
                .ok()
                .and_then(|u| u.join(location).ok())
                .map(|u| u.to_string())
                .unwrap_or_default();
            if login_like(&target) {
                return Reach::fail(
                    "web",
                    "direct",
                    Some(WebError::new(ErrorCode::NeedLogin)),
                    format!("{url} redirects to a sign-in page ({})", short(&target)),
                );
            }
            return Reach::fail(
                "web",
                "direct",
                None,
                format!(
                    "fetch failed for {url}: HTTP {} redirect to {} (not followed; fetch that URL directly)",
                    http.status,
                    short(&target)
                ),
            );
        }
        Ok(http) => {
            let challenge = is_challenge(&http.body, header(&http, "cf-mitigated"));
            let typed = classify_status(http.status, header(&http, "retry-after"));
            if challenge {
                (
                    format!(
                        "direct fetch returned an anti-bot challenge (HTTP {})",
                        http.status
                    ),
                    Reach::fail(
                        "web",
                        "direct",
                        Some(WebError::new(ErrorCode::PlatformBlocked)),
                        format!(
                            "{url} answered with an anti-bot challenge page (HTTP {})",
                            http.status
                        ),
                    ),
                )
            } else if (200..300).contains(&http.status) {
                if !near_empty(&http.body) {
                    return Reach::ok("web", "direct", format!("fetched {url}"), http.body);
                }
                (
                    "direct page was near-empty or JavaScript-only".into(),
                    Reach::ok(
                        "web",
                        "direct",
                        format!("fetched {url} (page looks JavaScript-only; little visible text)"),
                        http.body,
                    ),
                )
            } else {
                let fail = Reach::fail(
                    "web",
                    "direct",
                    typed.clone(),
                    format!("fetch failed for {url}: HTTP {}", http.status),
                );
                // Missing pages, sign-ins and rate limits are final: another
                // backend would not change them.
                if matches!(
                    typed.as_ref().map(|t| t.code),
                    Some(ErrorCode::NotFound | ErrorCode::NeedLogin | ErrorCode::RateLimited)
                ) {
                    return fail;
                }
                (format!("direct fetch returned HTTP {}", http.status), fail)
            }
        }
    };
    if !options.jina_fallback {
        return direct;
    }
    let reader = format!("{}/{url}", options.jina_base.trim_end_matches('/'));
    let near_empty_page = direct.ok.then(|| direct.clone());
    let mut result = match get(
        &client,
        &reader,
        &[("X-Return-Format", "markdown".to_owned())],
        options.max_bytes,
    ) {
        Err(e) => {
            let mut f = direct;
            f.summary = format!(
                "{} (Jina Reader fallback also failed: {})",
                f.summary,
                short(&e)
            );
            f
        }
        Ok(http) => {
            let lower = http
                .body
                .chars()
                .take(4096)
                .collect::<String>()
                .to_lowercase();
            if is_challenge(&http.body, "") {
                Reach::fail(
                    "web",
                    "jina",
                    Some(WebError::new(ErrorCode::PlatformBlocked)),
                    format!("{url} is behind an anti-bot challenge (direct and Jina Reader)"),
                )
            } else if lower.contains("target url returned error 404")
                || lower.contains("target url returned error 410")
            {
                Reach::fail(
                    "web",
                    "jina",
                    Some(WebError::new(ErrorCode::NotFound)),
                    format!("{url} does not exist (Jina Reader saw HTTP 404)"),
                )
            } else if http.status == 429 {
                Reach::fail(
                    "web",
                    "jina",
                    Some(WebError::with_hint(
                        ErrorCode::RateLimited,
                        "The Jina Reader fallback is rate limited (free tier). Do not retry now: use what you already have.",
                    )),
                    format!("Jina Reader rate limit while fetching {url}"),
                )
            } else if (200..300).contains(&http.status) && http.body.trim().chars().count() > 40 {
                Reach::ok(
                    "web",
                    "jina",
                    format!("fetched {url} via Jina Reader (backend: jina)"),
                    http.body,
                )
            } else {
                let mut f = direct;
                f.summary = format!(
                    "{} (Jina Reader fallback returned HTTP {})",
                    f.summary, http.status
                );
                f
            }
        }
    };
    if !result.ok
        && let Some(mut page) = near_empty_page
    {
        // The direct page was thin but real; keep it rather than an error.
        page.summary = format!("{} (Jina Reader fallback did not help)", page.summary);
        result = page;
    }
    result.fallback_reason = Some(reason);
    result
}

// ---------------------------------------------------------------- GitHub

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHub {
    Repo {
        owner: String,
        repo: String,
    },
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
    Pull {
        owner: String,
        repo: String,
        number: u64,
    },
    Blob {
        owner: String,
        repo: String,
        rest: String,
    },
}

const GITHUB_RESERVED: &[&str] = &[
    "about",
    "apps",
    "codespaces",
    "collections",
    "customer-stories",
    "dashboard",
    "enterprise",
    "explore",
    "features",
    "issues",
    "join",
    "login",
    "logout",
    "marketplace",
    "new",
    "notifications",
    "organizations",
    "orgs",
    "pricing",
    "pulls",
    "search",
    "security",
    "settings",
    "site",
    "sponsors",
    "topics",
    "trending",
    "users",
];

/// github.com repo, issue, PR and file links.
pub fn github_route(url: &str) -> Option<GitHub> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if !matches!(parsed.host_str()?, "github.com" | "www.github.com") {
        return None;
    }
    let parts: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    let owner = (*parts.first()?).to_owned();
    if GITHUB_RESERVED.contains(&owner.to_lowercase().as_str()) {
        return None;
    }
    let repo = parts.get(1)?.trim_end_matches(".git").to_owned();
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if !valid(&owner) || !valid(&repo) {
        return None;
    }
    match parts.get(2).copied() {
        None => Some(GitHub::Repo { owner, repo }),
        Some("issues") => Some(GitHub::Issue {
            owner,
            repo,
            number: parts.get(3)?.parse().ok()?,
        }),
        Some("pull") => Some(GitHub::Pull {
            owner,
            repo,
            number: parts.get(3)?.parse().ok()?,
        }),
        Some("blob") if parts.len() >= 5 => Some(GitHub::Blob {
            owner,
            repo,
            rest: parts[3..].join("/"),
        }),
        _ => None,
    }
}

fn github_error(http: &Http) -> WebError {
    let remaining = header(http, "x-ratelimit-remaining");
    if http.status == 429 || (http.status == 403 && remaining == "0") {
        let reset = header(http, "x-ratelimit-reset")
            .parse::<u64>()
            .ok()
            .map(|t| format!(" (resets at unix time {t})"))
            .unwrap_or_default();
        return WebError::with_hint(
            ErrorCode::RateLimited,
            format!(
                "GitHub API rate limit reached{reset}. Do not retry now; granting a GITHUB_TOKEN secret for https://api.github.com raises the limit."
            ),
        );
    }
    match http.status {
        401 => WebError::with_hint(
            ErrorCode::NeedLogin,
            "GitHub rejected the token. Fix or remove the GITHUB_TOKEN grant; public content works without one.",
        ),
        404 => WebError::with_hint(
            ErrorCode::NotFound,
            "Not found on GitHub (wrong link, or private: Klyne uses no login). Do not retry the same link.",
        ),
        403 | 451 => WebError::new(ErrorCode::PlatformBlocked),
        _ => WebError::new(ErrorCode::Network),
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

fn names(list: &Value, field: Option<&str>) -> String {
    list.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|x| match field {
                    Some(f) => x[f].as_str(),
                    None => x.as_str(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|joined| !joined.is_empty())
        .unwrap_or_else(|| "none".into())
}

struct GitHubApi<'a> {
    client: reqwest::blocking::Client,
    base: String,
    token: Option<&'a str>,
    max: usize,
}

impl GitHubApi<'_> {
    fn call(&self, path: &str, accept: &str) -> Result<Http, String> {
        let mut headers = vec![
            ("Accept", accept.to_owned()),
            ("X-GitHub-Api-Version", "2022-11-28".to_owned()),
        ];
        if let Some(token) = self.token {
            headers.push(("Authorization", format!("Bearer {token}")));
        }
        get(
            &self.client,
            &format!("{}{path}", self.base),
            &headers,
            self.max,
        )
        .map_err(|e| format!("GitHub API unreachable ({})", short(&e)))
    }
    fn json(&self, path: &str) -> Result<(Http, Value), String> {
        let http = self.call(path, "application/vnd.github+json")?;
        let value = serde_json::from_str(&http.body).unwrap_or(Value::Null);
        Ok((http, value))
    }
    fn comments(&self, owner: &str, repo: &str, number: u64, total: u64) -> String {
        if total == 0 {
            return String::new();
        }
        let Ok((_, list)) = self.json(&format!(
            "/repos/{owner}/{repo}/issues/{number}/comments?per_page=30"
        )) else {
            return "\n--- Comments: could not be loaded ---\n".into();
        };
        let items = list.as_array().cloned().unwrap_or_default();
        let mut out = format!("\n--- Comments (showing {} of {total}) ---\n", items.len());
        for c in items {
            let (body, cut) = clip(c["body"].as_str().unwrap_or(""), 2000);
            out.push_str(&format!(
                "\n[{} · {}]\n{}{}\n",
                s(&c["user"]["login"]),
                s(&c["created_at"]),
                body.trim(),
                if cut { " …" } else { "" }
            ));
        }
        out
    }
}

/// GitHub REST API answer for a routed link. Err means the API could not
/// be reached (the caller falls back to the page).
fn github(route: &GitHub, options: &Options) -> Result<Reach, String> {
    let api = GitHubApi {
        client: client(options.timeout)?,
        base: options.github_api.trim_end_matches('/').to_owned(),
        token: options.github_token.as_deref().filter(|t| !t.is_empty()),
        max: options.max_bytes,
    };
    let failed = |http: &Http, what: &str| -> Result<Reach, String> {
        let error = github_error(http);
        if error.code == ErrorCode::Network {
            return Err(format!("GitHub API returned HTTP {}", http.status));
        }
        Ok(Reach::fail(
            "github",
            "github_api",
            Some(error),
            format!("{what}: GitHub API HTTP {}", http.status),
        ))
    };
    let (summary, text) = match route {
        GitHub::Repo { owner, repo } => {
            let (http, r) = api.json(&format!("/repos/{owner}/{repo}"))?;
            if http.status != 200 {
                return failed(&http, &format!("repository {owner}/{repo}"));
            }
            let readme = match api.call(
                &format!("/repos/{owner}/{repo}/readme"),
                "application/vnd.github.raw",
            ) {
                Ok(h) if h.status == 200 => h.body,
                Ok(h) if h.status == 404 => "(no README)".into(),
                Ok(h) => format!("(README not loaded: HTTP {})", h.status),
                Err(e) => format!("(README not loaded: {e})"),
            };
            (
                format!(
                    "GitHub repository {owner}/{repo} via the GitHub API (backend: github_api)"
                ),
                format!(
                    "GitHub repository {}\nDescription: {}\nStars: {} · Forks: {} · Open issues: {} · Language: {} · License: {} · Default branch: {} · Last push: {}\nTopics: {}\nURL: {}\n\nREADME:\n{}",
                    s(&r["full_name"]),
                    s(&r["description"]),
                    r["stargazers_count"],
                    r["forks_count"],
                    r["open_issues_count"],
                    s(&r["language"]),
                    s(&r["license"]["spdx_id"]),
                    s(&r["default_branch"]),
                    s(&r["pushed_at"]),
                    names(&r["topics"], None),
                    s(&r["html_url"]),
                    readme
                ),
            )
        }
        GitHub::Issue {
            owner,
            repo,
            number,
        } => {
            let (http, i) = api.json(&format!("/repos/{owner}/{repo}/issues/{number}"))?;
            if http.status != 200 {
                return failed(&http, &format!("issue {owner}/{repo}#{number}"));
            }
            let kind = if i["pull_request"].is_object() {
                "pull request"
            } else {
                "issue"
            };
            (
                format!(
                    "GitHub {kind} {owner}/{repo}#{number} via the GitHub API (backend: github_api)"
                ),
                format!(
                    "GitHub {kind} {owner}/{repo}#{number}: {}\nState: {}{} · Author: {} · Labels: {} · Created: {} · Comments: {}\nURL: {}\n\n{}\n{}",
                    s(&i["title"]),
                    s(&i["state"]),
                    i["state_reason"]
                        .as_str()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default(),
                    s(&i["user"]["login"]),
                    names(&i["labels"], Some("name")),
                    s(&i["created_at"]),
                    i["comments"],
                    s(&i["html_url"]),
                    s(&i["body"]).trim(),
                    api.comments(owner, repo, *number, i["comments"].as_u64().unwrap_or(0))
                ),
            )
        }
        GitHub::Pull {
            owner,
            repo,
            number,
        } => {
            let (http, p) = api.json(&format!("/repos/{owner}/{repo}/pulls/{number}"))?;
            if http.status != 200 {
                return failed(&http, &format!("pull request {owner}/{repo}#{number}"));
            }
            let state = if p["merged"] == true {
                "merged".to_owned()
            } else {
                s(&p["state"])
            };
            (
                format!(
                    "GitHub pull request {owner}/{repo}#{number} via the GitHub API (backend: github_api)"
                ),
                format!(
                    "GitHub pull request {owner}/{repo}#{number}: {}\nState: {state} · Author: {} · {} <- {} · +{} -{} in {} files · Created: {} · Comments: {}\nURL: {}\n\n{}\n{}",
                    s(&p["title"]),
                    s(&p["user"]["login"]),
                    s(&p["base"]["ref"]),
                    s(&p["head"]["label"]),
                    p["additions"],
                    p["deletions"],
                    p["changed_files"],
                    s(&p["created_at"]),
                    p["comments"],
                    s(&p["html_url"]),
                    s(&p["body"]).trim(),
                    api.comments(owner, repo, *number, p["comments"].as_u64().unwrap_or(0))
                ),
            )
        }
        GitHub::Blob { .. } => return Err("file links use raw content".into()),
    };
    let (text, cut) = clip(&text, MAX_TEXT);
    Ok(Reach::ok(
        "github",
        "github_api",
        summary,
        if cut {
            format!("{text}\n[truncated]")
        } else {
            text
        },
    ))
}

// ---------------------------------------------------------------- YouTube

fn is_video_id(text: &str) -> bool {
    text.len() == 11
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The 11-character video id of a YouTube link, if it is one.
pub fn youtube_id(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim_start_matches("www.");
    let segments: Vec<&str> = parsed.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = match host {
        "youtu.be" => segments.first().map(|s| s.to_string()),
        "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtube-nocookie.com" => {
            match segments.first().copied() {
                Some("watch") => parsed
                    .query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.into_owned()),
                Some("shorts" | "embed" | "live" | "v") => segments.get(1).map(|s| s.to_string()),
                _ => None,
            }
        }
        _ => None,
    }?;
    is_video_id(&id).then_some(id)
}

/// Locate yt-dlp: `KLYNE_YTDLP`, Klyne's tools folder (`<repo>/tools/yt-dlp/`
/// or `<studio root>/tools/`), next to the executable, then PATH.
pub fn find_ytdlp(studio_root: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("KLYNE_YTDLP").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let names: &[&str] = if cfg!(windows) {
        &["yt-dlp.exe"]
    } else {
        &["yt-dlp", "yt-dlp_linux"]
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    let repo = studio_root.parent().and_then(Path::parent);
    for base in [Some(studio_root), repo].into_iter().flatten() {
        dirs.push(base.join("tools").join("yt-dlp"));
        dirs.push(base.join("tools"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join("tools"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|p| p.is_file())
}

/// Convert WebVTT or SRT to plain text, dropping cue numbers, timings,
/// tags and the rolling duplicate lines of auto-generated captions.
pub fn subtitles_to_text(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut skipping = false;
    for line in raw.lines() {
        let t = line.trim().trim_start_matches('\u{feff}');
        if t.is_empty() {
            skipping = false;
            continue;
        }
        if skipping
            || t.starts_with("WEBVTT")
            || t.starts_with("Kind:")
            || t.starts_with("Language:")
            || t.contains("-->")
            || t.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        if t.starts_with("NOTE") || t == "STYLE" || t.starts_with("REGION") {
            skipping = true;
            continue;
        }
        let mut text = String::new();
        let mut in_tag = false;
        for c in t.chars() {
            match c {
                '<' => in_tag = true,
                '>' if in_tag => in_tag = false,
                _ if !in_tag => text.push(c),
                _ => {}
            }
        }
        let text = decode_entities(&text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() || out.iter().rev().take(2).any(|p| *p == text) {
            continue;
        }
        out.push(text);
    }
    out.join("\n")
}

fn track_keys(info: &Value, field: &str) -> Vec<String> {
    info[field]
        .as_object()
        .map(|m| m.keys().filter(|k| *k != "live_chat").cloned().collect())
        .unwrap_or_default()
}

/// Pick a subtitle track: manual before auto-generated; the requested
/// language (default English), then the video's original language.
/// Returns (language key, manual).
pub fn choose_track(info: &Value, lang: Option<&str>) -> Option<(String, bool)> {
    let manual = track_keys(info, "subtitles");
    let auto = track_keys(info, "automatic_captions");
    let wanted = lang.unwrap_or("en").to_lowercase();
    let find = |list: &[String], order: &[String]| {
        order
            .iter()
            .find_map(|w| list.iter().find(|k| k.to_lowercase() == *w).cloned())
            .or_else(|| {
                list.iter()
                    .find(|k| k.to_lowercase().starts_with(&format!("{wanted}-")))
                    .cloned()
            })
    };
    if let Some(key) = find(&manual, std::slice::from_ref(&wanted)) {
        return Some((key, true));
    }
    if let Some(key) = find(&auto, &[format!("{wanted}-orig"), wanted.clone()]) {
        return Some((key, false));
    }
    if lang.is_some() {
        return None;
    }
    if let Some(key) = manual.first() {
        return Some((key.clone(), true));
    }
    auto.iter()
        .find(|k| k.ends_with("-orig"))
        .map(|k| (k.clone(), false))
}

fn classify_ytdlp(stderr: &str) -> Option<WebError> {
    let l = stderr.to_lowercase();
    let has = |m: &[&str]| m.iter().any(|x| l.contains(x));
    Some(if has(&["not a bot"]) {
        WebError::with_hint(
            ErrorCode::PlatformBlocked,
            "YouTube asked this network for a bot check. Klyne does not use cookies or logins: try later or ask the user for the transcript.",
        )
    } else if has(&[
        "private video",
        "members-only",
        "join this channel",
        "confirm your age",
        "age-restricted",
        "only available for registered users",
        "sign in to view",
        "login required",
    ]) {
        WebError::new(ErrorCode::NeedLogin)
    } else if has(&["http error 429", "too many requests"]) {
        WebError::new(ErrorCode::RateLimited)
    } else if has(&[
        "video unavailable",
        "does not exist",
        "http error 404",
        "incomplete youtube id",
        "not a valid url",
        "unsupported url",
        "this video has been removed",
        "is not available",
    ]) {
        WebError::new(ErrorCode::NotFound)
    } else if has(&[
        "http error 403",
        "blocked it in your country",
        "geo restrict",
    ]) {
        WebError::new(ErrorCode::PlatformBlocked)
    } else if has(&[
        "getaddrinfo",
        "timed out",
        "connection",
        "network is unreachable",
        "name or service not known",
        "temporary failure in name resolution",
        "ssl",
    ]) {
        WebError::new(ErrorCode::Network)
    } else {
        return None;
    })
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn run_ytdlp(
    path: &Path,
    args: &[String],
    stop: &AtomicBool,
    timeout: Duration,
) -> Result<Run, String> {
    let mut command = Command::new(path);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Minimal environment: no API keys or tokens reach the child process.
    for name in [
        "PATH",
        "SYSTEMROOT",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "LANG",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start yt-dlp: {e}"))?;
    let _job = match harness_core::process_job::ProcessJob::attach(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e.to_string());
        }
    };
    let out = child.stdout.take();
    let err = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(out) = out {
            let _ = out.take(16 * 1024 * 1024).read_to_end(&mut buf);
        }
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(err) = err {
            let _ = err.take(256 * 1024).read_to_end(&mut buf);
        }
        buf
    });
    let started = Instant::now();
    let status = loop {
        if stop.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".into());
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("yt-dlp timed out after {}s", timeout.as_secs()));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok(Run {
        ok: status.success(),
        stdout: String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned(),
        stderr: String::from_utf8_lossy(&err_reader.join().unwrap_or_default()).into_owned(),
    })
}

fn duration_text(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn valid_lang(lang: &str) -> bool {
    !lang.is_empty()
        && lang.len() <= 16
        && lang
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Normalise a `video_transcript` target: a bare id becomes a watch URL.
pub fn video_url(target: &str) -> String {
    let target = target.trim();
    if is_video_id(target) {
        format!("https://www.youtube.com/watch?v={target}")
    } else {
        target.to_owned()
    }
}

/// The `video_transcript` tool: metadata plus subtitles via yt-dlp.
pub fn video_transcript(
    target: &str,
    lang: Option<&str>,
    options: &Options,
    stop: &AtomicBool,
) -> Reach {
    let url = video_url(target);
    let lang = lang.map(str::trim).filter(|l| !l.is_empty());
    let mut reach = match reqwest::Url::parse(&url) {
        Ok(u) if u.scheme() == "https" && u.username().is_empty() && u.password().is_none() => {
            match lang {
                Some(l) if !valid_lang(l) => Reach::fail(
                    "video_transcript",
                    "host",
                    None,
                    "lang must be a language code such as en, de or pt-BR",
                ),
                l => youtube(&url, l, options, stop, true),
            }
        }
        _ => Reach::fail(
            "video_transcript",
            "host",
            None,
            "url_or_id must be an https video URL or an 11-character YouTube video id",
        ),
    };
    reach.route = "video_transcript";
    reach.source = source_key("video_transcript", &url, lang);
    reach
}

fn available_tracks(meta: &Value) -> String {
    let mut langs = track_keys(meta, "subtitles");
    langs.extend(
        track_keys(meta, "automatic_captions")
            .into_iter()
            .filter(|k| k.ends_with("-orig"))
            .map(|k| format!("{} (auto)", k.trim_end_matches("-orig"))),
    );
    if langs.is_empty() {
        "none".to_owned()
    } else {
        clip(&langs.join(", "), 300).0
    }
}

fn youtube(
    url: &str,
    lang: Option<&str>,
    options: &Options,
    stop: &AtomicBool,
    require_transcript: bool,
) -> Reach {
    let Some(ytdlp) = options.ytdlp.as_ref() else {
        return Reach::fail(
            "youtube",
            "host",
            None,
            "yt-dlp is not installed: put yt-dlp.exe in Klyne's tools folder (tools\\yt-dlp\\yt-dlp.exe; scripts/install-yt-dlp.ps1 does this)",
        );
    };
    let tmp = match tempfile::Builder::new().prefix("klyne-subs-").tempdir() {
        Ok(t) => t,
        Err(e) => return Reach::fail("youtube", "yt-dlp", None, e.to_string()),
    };
    let base = |extra: &[&str]| -> Vec<String> {
        [
            "--ignore-config",
            "--no-warnings",
            "--no-playlist",
            "--skip-download",
        ]
        .iter()
        .chain(extra.iter())
        .map(|s| s.to_string())
        .collect()
    };
    let timeout = Duration::from_secs(90);
    let failure = |stderr: &str, what: &str| -> Reach {
        let line = stderr
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .or(stderr.lines().last())
            .unwrap_or("");
        Reach::fail(
            "youtube",
            "yt-dlp",
            classify_ytdlp(stderr),
            format!("{what}: {}", short(line)),
        )
    };
    let process_error = |e: String| -> Reach {
        let code = e
            .contains("timed out")
            .then(|| WebError::new(ErrorCode::Network));
        Reach::fail("youtube", "yt-dlp", code, e)
    };
    let mut args = base(&["--dump-single-json"]);
    args.push(url.to_owned());
    let info = match run_ytdlp(ytdlp, &args, stop, timeout) {
        Err(e) => return process_error(e),
        Ok(run) if !run.ok => return failure(&run.stderr, "yt-dlp could not read the video"),
        Ok(run) => run.stdout,
    };
    let meta: Value = serde_json::from_str(&info).unwrap_or(Value::Null);
    if !meta.is_object() {
        return Reach::fail(
            "youtube",
            "yt-dlp",
            None,
            "yt-dlp returned no video metadata",
        );
    }
    let title = s(&meta["title"]);
    let header = format!(
        "Title: {}\nChannel: {}\nDuration: {}\nURL: {}\nUploaded: {}",
        title,
        meta["channel"]
            .as_str()
            .or(meta["uploader"].as_str())
            .unwrap_or(""),
        meta["duration"]
            .as_f64()
            .map(duration_text)
            .unwrap_or_default(),
        meta["webpage_url"].as_str().unwrap_or(url),
        s(&meta["upload_date"]),
    );
    let no_track = |why: String| -> Reach {
        if require_transcript {
            let mut r = Reach::fail(
                "youtube",
                "yt-dlp",
                Some(WebError::with_hint(
                    ErrorCode::NotFound,
                    format!(
                        "No subtitles for this request. Available: {}. Try another lang, or say that no transcript exists.",
                        available_tracks(&meta)
                    ),
                )),
                why,
            );
            r.data = format!("{header}\n\n{}", r.data);
            r
        } else {
            Reach::ok(
                "youtube",
                "yt-dlp",
                format!("YouTube video \"{title}\" (metadata only: {why})"),
                format!(
                    "{header}\n\nTranscript: not available ({why}). Available subtitles: {}",
                    available_tracks(&meta)
                ),
            )
        }
    };
    let Some((key, manual)) = choose_track(&meta, lang) else {
        return no_track(format!(
            "no subtitles or auto-captions in {}",
            lang.unwrap_or("English or the original language")
        ));
    };
    let info_path = tmp.path().join("info.json");
    if let Err(e) = std::fs::write(&info_path, &info) {
        return Reach::fail("youtube", "yt-dlp", None, e.to_string());
    }
    let output = tmp.path().join("sub.%(ext)s");
    let mut args = base(&[
        if manual {
            "--write-subs"
        } else {
            "--write-auto-subs"
        },
        "--sub-langs",
        &key,
        "--sub-format",
        "vtt/srt/best",
        "--load-info-json",
    ]);
    args.push(info_path.to_string_lossy().into_owned());
    args.push("-o".into());
    args.push(output.to_string_lossy().into_owned());
    match run_ytdlp(ytdlp, &args, stop, timeout) {
        Err(e) => return process_error(e),
        Ok(run) if !run.ok => {
            return failure(&run.stderr, "yt-dlp could not download subtitles");
        }
        Ok(_) => {}
    }
    let file = std::fs::read_dir(tmp.path())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("sub.") && !n.ends_with(".json"))
        });
    let Some(file) = file else {
        return no_track(format!("the {key} subtitle track could not be downloaded"));
    };
    let raw = std::fs::read(&file).unwrap_or_default();
    let text = subtitles_to_text(&String::from_utf8_lossy(&raw));
    if text.trim().is_empty() {
        return no_track(format!("the {key} subtitle track was empty"));
    }
    let total = text.chars().count();
    let (body, cut) = clip(&text, MAX_TEXT.saturating_sub(header.len() + 120));
    let kind = if manual { "manual" } else { "auto-generated" };
    let note = if cut {
        format!(
            "\n[transcript truncated: {} of {total} characters shown]",
            body.chars().count()
        )
    } else {
        String::new()
    };
    Reach::ok(
        "youtube",
        "yt-dlp",
        format!("Transcript of \"{title}\" ({key}, {kind}, {total} characters) via yt-dlp"),
        format!(
            "{header}\nTranscript ({key}, {kind}{}):\n{body}{note}",
            if cut { ", truncated" } else { "" }
        ),
    )
}

// ------------------------------------------------- evidence-gate helpers

/// Web sources in this goal window that the host classified as blocked.
/// NETWORK counts after two failures of the same source.
pub fn blocked_sources(window: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut network: Vec<(String, usize)> = Vec::new();
    for e in window {
        if let Some(source) = e["source"].as_str()
            && e["ok"] == true
            && e["error_code"].is_null()
        {
            // A later clean read of the same source (e.g. a challenge that
            // resolved in the browser) unblocks it.
            out.retain(|b| b["source"] != source);
            network.retain(|(s, _)| s != source);
            continue;
        }
        let (Some(source), Some(code)) = (
            e["source"].as_str().filter(|s| !s.is_empty()),
            e["error_code"].as_str().and_then(ErrorCode::parse),
        ) else {
            continue;
        };
        let blocked = if code.final_for_source() || e["repeat_blocked"] == true {
            true
        } else {
            let count = match network.iter_mut().find(|(s, _)| s == source) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    network.push((source.to_owned(), 1));
                    1
                }
            };
            count >= 2
        };
        if blocked && !out.iter().any(|b| b["source"] == source) {
            out.push(json!({"source":source,"error_code":code.as_str(),"hint":e["hint"]}));
        }
    }
    out
}

/// Short-circuit a repeat of a blocked source (no network, no retry loop).
pub fn repeat_of_blocked(window: &[Value], action: &Value, agent: &str) -> Option<Value> {
    let tool = action["tool"].as_str()?;
    let (target, lang) = match tool {
        "fetch_url" => (action["url"].as_str()?.to_owned(), None),
        "video_transcript" => (
            video_url(
                action["url_or_id"]
                    .as_str()
                    .or(action["url"].as_str())
                    .or(action["id"].as_str())?,
            ),
            action["lang"]
                .as_str()
                .map(str::trim)
                .filter(|l| !l.is_empty()),
        ),
        _ => return None,
    };
    let source = source_key(tool, &target, lang);
    let blocked = blocked_sources(window)
        .into_iter()
        .find(|b| b["source"] == source)?;
    let code = blocked["error_code"].as_str().unwrap_or("PLATFORM_BLOCKED");
    let hint = blocked["hint"].as_str().unwrap_or("");
    let action_key = if tool == "fetch_url" {
        format!("fetch:{target}")
    } else {
        format!("video_transcript:{target}")
    };
    Some(json!({
        "agent":agent,"action":action_key,"ok":false,
        "summary":format!("{code}: this source was already blocked earlier in this task; not retried. Use other sources, finish with what you have and say this source was blocked, or report_blocker."),
        "data":format!("{code}: source already blocked; not retried.\nHint: {hint}"),
        "error_code":code,"hint":hint,"source":source,"backend":"host","route":"blocked_source","repeat_blocked":true
    }))
}

/// Anti-bot challenge in the isolated browser's title and visible text.
pub fn browser_challenge(title: &str, text: &str) -> bool {
    let clean = |t: &str| t.replace(['<', '>'], " ");
    is_challenge(
        &format!(
            "<html><title>{}</title><body>{}</body></html>",
            clean(title.trim()),
            clean(text)
        ),
        "",
    )
}

/// True when the request needs a real page interaction (clicks, forms,
/// logins, screenshots or an explicit browser), so read-only routing and
/// host-level blocks must not stand in for the browser.
pub fn needs_interaction(text: &str) -> bool {
    let words: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let t = format!(
        " {} ",
        words.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    // Whole words or phrases only ("form" must not match "information").
    const PHRASES: &[&str] = &[
        "click",
        "clicks",
        "clicking",
        "press",
        "tap",
        "fill",
        "fill in",
        "fill out",
        "form",
        "forms",
        "type into",
        "log in",
        "login",
        "log into",
        "sign in",
        "signin",
        "sign up",
        "signup",
        "submit",
        "checkout",
        "check out",
        "add to cart",
        "scroll",
        "screenshot",
        "screenshots",
        "browser",
        "navigate",
        "interact",
        "hover",
        "drag",
        "dropdown",
        "upload",
        "book a",
        "reserve",
        "button",
        "buttons",
        "captcha",
    ];
    PHRASES.iter().any(|p| t.contains(&format!(" {p} ")))
}

fn host_of(source: &str) -> Option<String> {
    let (_, rest) = source.split_once(':')?;
    let url = reqwest::Url::parse(rest).ok()?;
    url.host_str()
        .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
}

/// URL of the isolated browser's current page, from the latest browser
/// entry that recorded one.
pub fn browser_page(window: &[Value]) -> Option<String> {
    window
        .iter()
        .rev()
        .find_map(|e| e["page_url"].as_str().filter(|u| !u.is_empty()))
        .map(str::to_owned)
}

/// Was this URL already fetched with fetch_url in this window (any result)?
pub fn fetched_before(window: &[Value], url: &str) -> bool {
    let key = source_key("fetch_url", url, None);
    window.iter().any(|e| {
        e["source"] == key.as_str()
            || e["action"]
                .as_str()
                .and_then(|a| a.strip_prefix("fetch:"))
                .is_some_and(|u| source_key("fetch_url", u, None) == key)
    })
}

/// Answer a browser action on a source already blocked, without driving
/// the browser. browser_open: the same URL blocked by fetch_url or the
/// browser, or the same host blocked as PLATFORM_BLOCKED / RATE_LIMITED /
/// NEED_LOGIN; an interactive task may still try the browser unless the
/// browser itself already hit a challenge there. Reads and inputs on a
/// page still showing a challenge are not repeated (Klyne does not solve
/// challenges): input at once, reads after two challenge observations.
pub fn browser_blocked(
    window: &[Value],
    action: &Value,
    interactive: bool,
    agent: &str,
) -> Option<Value> {
    let tool = action["tool"].as_str()?;
    let blocked = blocked_sources(window);
    let found = match tool {
        "browser_open" => {
            let url = action["url"].as_str()?;
            if !url.starts_with("http") {
                return None;
            }
            let keys = [
                source_key("fetch_url", url, None),
                source_key("browser", url, None),
            ];
            let host = host_of(&keys[1]);
            let hit = |b: &&Value, browser_only: bool| {
                let source = b["source"].as_str().unwrap_or("");
                if browser_only && !source.starts_with("browser:") {
                    return false;
                }
                keys.iter().any(|k| k == source)
                    || (host.is_some()
                        && host_of(source) == host
                        && matches!(
                            b["error_code"].as_str(),
                            Some("PLATFORM_BLOCKED" | "RATE_LIMITED" | "NEED_LOGIN")
                        ))
            };
            blocked.iter().find(|b| hit(b, interactive)).cloned()?
        }
        "browser_read" | "browser_click" | "browser_fill" => {
            let page = browser_page(window)?;
            let key = source_key("browser", &page, None);
            let b = blocked
                .iter()
                .find(|b| b["source"] == key.as_str())?
                .clone();
            let seen = window
                .iter()
                .filter(|e| e["source"] == key.as_str() && e["error_code"] == "PLATFORM_BLOCKED")
                .count();
            (tool != "browser_read" || seen >= 2).then_some(b)?
        }
        _ => return None,
    };
    let code = found["error_code"].as_str().unwrap_or("PLATFORM_BLOCKED");
    let hint = found["hint"].as_str().unwrap_or("");
    let source = found["source"].as_str().unwrap_or("");
    Some(json!({
        "agent":agent,"action":tool,"ok":false,
        "summary":format!("{code}: {source} is already blocked in this task; the browser was not used again. Klyne does not solve anti-bot challenges. Use other sources, finish with what you have and say this source was blocked, or report_blocker."),
        "data":format!("{code}: source already blocked; browser not used.\nHint: {hint}"),
        "error_code":code,"hint":hint,"source":source,"backend":"host","route":"blocked_source","repeat_blocked":true,
        "page_url":action["url"].as_str().map(str::to_owned).or_else(|| browser_page(window))
    }))
}

/// Classify a browser observation: adds page_url and source, and turns an
/// anti-bot challenge into a PLATFORM_BLOCKED failure the evidence gate
/// never accepts.
pub fn annotate_browser(
    entry: &mut Value,
    value: &Value,
    window: &[Value],
    requested: Option<&str>,
) {
    let tool = entry["action"].as_str().unwrap_or("").to_owned();
    if !matches!(
        tool.as_str(),
        "browser_open" | "browser_read" | "browser_click" | "browser_fill"
    ) {
        return;
    }
    let page = value["url"]
        .as_str()
        .filter(|u| !u.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            (tool == "browser_open")
                .then(|| requested.map(str::to_owned))
                .flatten()
        })
        .or_else(|| browser_page(window));
    let Some(page) = page else {
        return;
    };
    let source = source_key("browser", &page, None);
    entry["page_url"] = json!(page);
    entry["source"] = json!(source);
    entry["backend"] = json!("browser");
    if value["ok"] == false {
        return;
    }
    let title = value["title"].as_str().unwrap_or("");
    let text = value["text"].as_str().unwrap_or("");
    // Interstitials that render inside an iframe (e.g. DataDome) leave the
    // top page blank with the bare domain as its title.
    let bare_host = reqwest::Url::parse(&page)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
        })
        .is_some_and(|h| {
            title
                .trim()
                .trim_start_matches("www.")
                .eq_ignore_ascii_case(&h)
        })
        && text.trim().chars().count() < 40;
    if bare_host || browser_challenge(title, text) {
        let code = ErrorCode::PlatformBlocked;
        entry["ok"] = json!(false);
        entry["error_code"] = json!(code.as_str());
        entry["hint"] = json!(code.hint());
        entry["summary"] = json!(format!(
            "PLATFORM_BLOCKED: {page} shows an anti-bot challenge in the isolated browser (title {title:?}); not usable as evidence"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    type Route = fn(&str) -> (u16, &'static str, String);

    /// Tiny HTTP fixture: `route(path) -> (status, extra headers, body)`.
    fn serve(route: Route) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, headers, body) = route(&path);
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
                    body.len()
                );
            }
        });
        format!("http://{addr}")
    }

    fn opts(base: &str, jina: bool) -> Options {
        let mut o = Options::new(jina, None, None);
        o.jina_base = format!("{base}/jina");
        o.github_api = format!("{base}/gh");
        o.github_raw = format!("{base}/raw");
        o.timeout = Duration::from_secs(5);
        o
    }

    const CF: &str = "<!DOCTYPE html><html><head><title>Just a moment...</title></head><body><script src=\"/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1\"></script></body></html>";

    #[test]
    fn challenge_pages_are_detected_and_articles_are_not() {
        assert!(is_challenge(CF, ""));
        assert!(is_challenge("<html>ok</html>", "challenge"));
        assert!(is_challenge(
            "Title: Just a moment...\nWarning: Target URL returned error 403: Forbidden\nWarning: This page maybe requiring CAPTCHA",
            ""
        ));
        let article = format!(
            "<html><body><h1>How bot checks work</h1><p>Sites ask: are you a robot?</p><p>{}</p></body></html>",
            "Long article text. ".repeat(400)
        );
        assert!(!is_challenge(&article, ""));
        assert!(near_empty(
            "<html><body><div id=root></div><script src=app.js></script></body></html>"
        ));
        assert!(!near_empty(&article));
        assert!(visible_text("<p>A&amp;B</p><script>var x='<p>';</script>") == "a&b");
    }

    #[test]
    fn statuses_map_to_typed_errors_with_hints() {
        assert_eq!(classify_status(404, "").unwrap().code, ErrorCode::NotFound);
        assert_eq!(classify_status(401, "").unwrap().code, ErrorCode::NeedLogin);
        assert_eq!(
            classify_status(403, "").unwrap().code,
            ErrorCode::PlatformBlocked
        );
        let limited = classify_status(429, "120").unwrap();
        assert_eq!(limited.code, ErrorCode::RateLimited);
        assert!(limited.hint.contains("120"));
        assert_eq!(classify_status(503, "").unwrap().code, ErrorCode::Network);
        assert!(classify_status(400, "").is_none());
        for code in [
            "NEED_LOGIN",
            "RATE_LIMITED",
            "PLATFORM_BLOCKED",
            "NOT_FOUND",
            "NETWORK",
        ] {
            let parsed = ErrorCode::parse(code).unwrap();
            assert_eq!(parsed.as_str(), code);
            assert!(!parsed.hint().is_empty() && !parsed.hint().contains('\n'));
        }
        let yt = classify_ytdlp("ERROR: [youtube] abc: Sign in to confirm you're not a bot.");
        assert_eq!(yt.unwrap().code, ErrorCode::PlatformBlocked);
        let gone = classify_ytdlp("ERROR: [youtube] abc: Video unavailable");
        assert_eq!(gone.unwrap().code, ErrorCode::NotFound);
        let private = classify_ytdlp("ERROR: Private video. Sign in if you've been granted access");
        assert_eq!(private.unwrap().code, ErrorCode::NeedLogin);
    }

    fn web_route(path: &str) -> (u16, &'static str, String) {
        let jina = path.starts_with("/jina/");
        match path.rsplit('/').next().unwrap_or("") {
            "cf" if jina => (
                200,
                "",
                "Title: Real page\nURL Source: x\n\nMarkdown Content:\nThe real article text is here and long enough.".into(),
            ),
            "cf" => (403, "Server: cloudflare\r\n", CF.into()),
            "spa" if jina => (
                200,
                "",
                "Title: App\n\nMarkdown Content:\nRendered app content with plenty of words.".into(),
            ),
            "spa" => (
                200,
                "",
                "<html><body><div id=app></div><script src=a.js></script></body></html>".into(),
            ),
            "both" => (if jina { 200 } else { 403 }, "", CF.into()),
            "gone" => (404, "", "nope".into()),
            "login" => (302, "Location: https://accounts.example.test/login\r\n", String::new()),
            "ok" => (
                200,
                "",
                format!("<html><body><p>{}</p></body></html>", "Plain content. ".repeat(30)),
            ),
            _ => (500, "", String::new()),
        }
    }

    #[test]
    fn challenge_falls_back_to_jina_and_records_the_backend() {
        let base = serve(web_route);
        let stop = AtomicBool::new(false);
        let r = fetch(&format!("{base}/cf"), &opts(&base, true), &stop);
        assert!(r.ok, "{r:?}");
        assert_eq!(r.backend, "jina");
        assert!(r.fallback_reason.as_deref().unwrap().contains("challenge"));
        assert_eq!(r.meta()["backend"], "jina");
        let spa = fetch(&format!("{base}/spa"), &opts(&base, true), &stop);
        assert_eq!((spa.ok, spa.backend), (true, "jina"));
        // A challenge on both backends is a typed block, never evidence.
        let both = fetch(&format!("{base}/both"), &opts(&base, true), &stop);
        assert!(!both.ok);
        assert_eq!(both.error.unwrap().code, ErrorCode::PlatformBlocked);
        // Fallback off: the challenge is reported directly.
        let off = fetch(&format!("{base}/cf"), &opts(&base, false), &stop);
        assert_eq!((off.ok, off.backend), (false, "direct"));
        assert_eq!(off.meta()["error_code"], "PLATFORM_BLOCKED");
        // 404 is final: no fallback.
        let gone = fetch(&format!("{base}/gone"), &opts(&base, true), &stop);
        assert_eq!(gone.meta()["error_code"], "NOT_FOUND");
        assert_eq!(gone.backend, "direct");
        assert!(gone.data.contains("Hint:"));
        assert_eq!(gone.meta()["source"], format!("fetch_url:{base}/gone"));
        let login = fetch(&format!("{base}/login"), &opts(&base, true), &stop);
        assert_eq!(login.meta()["error_code"], "NEED_LOGIN");
        let ok = fetch(&format!("{base}/ok"), &opts(&base, true), &stop);
        assert_eq!((ok.ok, ok.backend), (true, "direct"));
        assert!(ok.fallback_reason.is_none());
        let mut entry = json!({"action":"fetch:x","ok":true});
        ok.annotate(&mut entry);
        assert_eq!(entry["backend"], "direct");
        assert!(entry.get("error_code").is_none());
    }

    fn github_fixture(path: &str) -> (u16, &'static str, String) {
        match path {
            "/gh/repos/o/r/issues/12" => (
                200,
                "",
                json!({"title":"Crash on start","state":"open","user":{"login":"ann"},"labels":[{"name":"bug"}],"created_at":"2026-10-01","comments":1,"html_url":"https://github.com/o/r/issues/12","body":"It crashes."}).to_string(),
            ),
            "/gh/repos/o/r/issues/12/comments?per_page=30" => (
                200,
                "",
                json!([{"user":{"login":"bob"},"created_at":"2026-10-02","body":"Same here."}]).to_string(),
            ),
            "/gh/repos/o/r" => (
                200,
                "",
                json!({"full_name":"o/r","description":"Demo","stargazers_count":5,"topics":["a","b"],"html_url":"https://github.com/o/r"}).to_string(),
            ),
            "/gh/repos/o/r/readme" => (200, "", "# Demo readme".into()),
            "/gh/repos/o/r/issues/13" => (404, "", "{}".into()),
            "/gh/repos/o/r/issues/14" => (
                403,
                "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: 99\r\n",
                "{}".into(),
            ),
            _ => (500, "", String::new()),
        }
    }

    #[test]
    fn github_links_route_to_the_api() {
        let issue = |n| GitHub::Issue {
            owner: "o".into(),
            repo: "r".into(),
            number: n,
        };
        assert_eq!(
            github_route("https://github.com/o/r/issues/12#issuecomment-1"),
            Some(issue(12))
        );
        assert_eq!(
            github_route("https://github.com/o/r.git"),
            Some(GitHub::Repo {
                owner: "o".into(),
                repo: "r".into()
            })
        );
        assert_eq!(
            github_route("https://github.com/o/r/pull/7/files"),
            Some(GitHub::Pull {
                owner: "o".into(),
                repo: "r".into(),
                number: 7
            })
        );
        assert_eq!(
            github_route("https://github.com/o/r/blob/main/src/a.rs"),
            Some(GitHub::Blob {
                owner: "o".into(),
                repo: "r".into(),
                rest: "main/src/a.rs".into()
            })
        );
        assert_eq!(github_route("https://github.com/settings/profile"), None);
        assert_eq!(github_route("https://github.com/o/r/actions"), None);
        assert_eq!(github_route("https://gitlab.com/o/r"), None);

        let base = serve(github_fixture);
        let o = opts(&base, false);
        let found = github(&issue(12), &o).unwrap();
        assert!(found.ok);
        assert_eq!(found.backend, "github_api");
        assert!(found.data.contains("Crash on start") && found.data.contains("bug"));
        assert!(found.data.contains("Same here.") && found.data.contains("[bob"));
        let repo = github(&github_route("https://github.com/o/r").unwrap(), &o).unwrap();
        assert!(repo.data.contains("# Demo readme") && repo.data.contains("Topics: a, b"));
        let missing = github(&issue(13), &o).unwrap();
        assert_eq!(missing.error.unwrap().code, ErrorCode::NotFound);
        let limited = github(&issue(14), &o).unwrap();
        assert_eq!(limited.error.as_ref().unwrap().code, ErrorCode::RateLimited);
        assert!(limited.error.unwrap().hint.contains("GITHUB_TOKEN"));
        // Server errors mean "API unavailable": the caller falls back.
        assert!(github(&issue(15), &o).is_err());
    }

    #[test]
    fn youtube_ids_tracks_and_subtitles() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://m.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
        ] {
            assert_eq!(youtube_id(url).as_deref(), Some("dQw4w9WgXcQ"), "{url}");
        }
        assert_eq!(youtube_id("https://www.youtube.com/@channel"), None);
        assert_eq!(
            source_key("fetch_url", "https://youtu.be/dQw4w9WgXcQ/", None),
            "fetch_url:youtube:dQw4w9WgXcQ"
        );
        let info = json!({"subtitles":{"de":[{}],"live_chat":[{}]},"automatic_captions":{"en":[{}],"en-orig":[{}],"fr":[{}]}});
        assert_eq!(choose_track(&info, None), Some(("en-orig".into(), false)));
        assert_eq!(choose_track(&info, Some("de")), Some(("de".into(), true)));
        assert_eq!(choose_track(&info, Some("ja")), None);
        let manual = json!({"subtitles":{"en-US":[{}]},"automatic_captions":{"en":[{}]}});
        assert_eq!(choose_track(&manual, None), Some(("en-US".into(), true)));
        let foreign = json!({"subtitles":{},"automatic_captions":{"es-orig":[{}],"de":[{}]}});
        assert_eq!(
            choose_track(&foreign, None),
            Some(("es-orig".into(), false))
        );
        assert_eq!(choose_track(&foreign, Some("ko")), None);
        assert_eq!(available_tracks(&info), "de, en (auto)");

        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\nNOTE generated\nmore note\n\n00:00:00.000 --> 00:00:02.000 align:start\nhello <00:00:00.500><c>there</c>\n\n00:00:02.000 --> 00:00:02.010\nhello there\n\n00:00:02.010 --> 00:00:04.000\nhello there\nhow are you &amp; me\n\n00:00:04.000 --> 00:00:04.010\nhow are you &amp; me\n";
        assert_eq!(subtitles_to_text(vtt), "hello there\nhow are you & me");
        let srt = "1\n00:00:01,000 --> 00:00:02,000\n<i>First line</i>\n\n2\n00:00:02,000 --> 00:00:03,000\nSecond line\n";
        assert_eq!(subtitles_to_text(srt), "First line\nSecond line");
        assert_eq!(duration_text(3725.0), "1:02:05");
        assert_eq!(duration_text(65.0), "1:05");
        let stop = AtomicBool::new(false);
        let none = Options::new(true, None, None);
        let missing = video_transcript("dQw4w9WgXcQ", Some("en"), &none, &stop);
        assert!(!missing.ok && missing.summary.contains("yt-dlp is not installed"));
        assert_eq!(missing.route, "video_transcript");
        assert_eq!(missing.source, "video_transcript:youtube:dQw4w9WgXcQ:en");
        assert!(!video_transcript("not a url", None, &none, &stop).ok);
        let bad_lang = video_transcript("dQw4w9WgXcQ", Some("en;rm"), &none, &stop);
        assert!(bad_lang.summary.contains("lang"));
    }

    #[test]
    fn blocked_sources_short_circuit_repeats() {
        let blocked = json!({"action":"fetch:https://x.test/a","ok":false,"error_code":"PLATFORM_BLOCKED","hint":"h","source":"fetch_url:https://x.test/a"});
        let net = json!({"action":"fetch:https://y.test/","ok":false,"error_code":"NETWORK","hint":"n","source":"fetch_url:https://y.test"});
        let window = vec![blocked, net.clone()];
        let fetch_action = |url: &str| json!({"tool":"fetch_url","url":url});
        let skip =
            repeat_of_blocked(&window, &fetch_action("https://x.test/a/#top"), "worker").unwrap();
        assert_eq!(skip["ok"], false);
        assert_eq!(skip["repeat_blocked"], true);
        assert_eq!(skip["error_code"], "PLATFORM_BLOCKED");
        assert_eq!(skip["action"], "fetch:https://x.test/a/#top");
        // One network failure still allows a retry; two do not.
        assert!(repeat_of_blocked(&window, &fetch_action("https://y.test"), "w").is_none());
        let twice = vec![net.clone(), net];
        assert!(repeat_of_blocked(&twice, &fetch_action("https://y.test"), "w").is_some());
        assert!(repeat_of_blocked(&window, &fetch_action("https://z.test"), "w").is_none());
        let read = json!({"tool":"read_file","path":"a"});
        assert!(repeat_of_blocked(&window, &read, "w").is_none());
        let yt =
            json!({"error_code":"NOT_FOUND","source":"video_transcript:youtube:dQw4w9WgXcQ:ja"});
        let ask =
            |lang: &str| json!({"tool":"video_transcript","url_or_id":"dQw4w9WgXcQ","lang":lang});
        assert!(repeat_of_blocked(std::slice::from_ref(&yt), &ask("ja"), "w").is_some());
        assert!(repeat_of_blocked(&[yt], &ask("en"), "w").is_none());
        assert_eq!(blocked_sources(&window).len(), 1);
    }

    /// Live network check, run by hand: `KLYNE_LIVE_WEB=1 cargo test
    /// live_web_reach -- --ignored --nocapture`. Needs yt-dlp in tools/.
    #[test]
    #[ignore]
    fn live_web_reach() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../workspace/studio");
        let o = Options::new(true, None, find_ytdlp(&root));
        let stop = AtomicBool::new(false);
        for url in std::env::var("KLYNE_LIVE_URLS")
            .unwrap_or_default()
            .split_whitespace()
        {
            let r = fetch(url, &o, &stop);
            println!(
                "== {url}\n{}\n{}\n{}",
                r.summary,
                r.meta(),
                clip(&r.data, 600).0
            );
        }
        if let Ok(id) = std::env::var("KLYNE_LIVE_VIDEO") {
            let r = video_transcript(&id, None, &o, &stop);
            println!(
                "== video {id}\n{}\n{}\n{}",
                r.summary,
                r.meta(),
                clip(&r.data, 900).0
            );
        }
    }

    #[test]
    fn browser_challenges_interaction_and_blocked_hosts() {
        assert!(browser_challenge(
            "Just a moment...",
            "Verifying you are human. This may take a few seconds."
        ));
        assert!(browser_challenge(
            "www.g2.com",
            "Performing security verification\nThis website uses a security service"
        ));
        assert!(browser_challenge("Attention Required! | Cloudflare", ""));
        assert!(!browser_challenge(
            "Rust issue #100",
            &"A long article about verifying you are human. ".repeat(100)
        ));
        assert!(!browser_challenge(
            "Example Domain",
            "This domain is for use in documentation examples."
        ));

        assert!(!needs_interaction(
            "Summarize https://github.com/rust-lang/rust/issues/100 with more information and the platform format"
        ));
        assert!(!needs_interaction("What does https://example.com say?"));
        for t in [
            "Click the Next button on https://x.test",
            "fill in the form",
            "Log in to the site",
            "Open it in the browser",
            "take a screenshot of x.test",
        ] {
            assert!(needs_interaction(t), "{t}");
        }

        let open = |url: &str| json!({"tool":"browser_open","url":url});
        let fetch_block = json!({"action":"fetch:https://www.g2.com/","ok":false,"error_code":"PLATFORM_BLOCKED","hint":"h","source":"fetch_url:https://www.g2.com"});
        let window = vec![fetch_block.clone()];
        // Read-only task: same URL and same host are answered as blocked.
        let skip = browser_blocked(&window, &open("https://www.g2.com/"), false, "w").unwrap();
        assert_eq!(skip["repeat_blocked"], true);
        assert_eq!(skip["error_code"], "PLATFORM_BLOCKED");
        assert!(browser_blocked(&window, &open("https://g2.com/products"), false, "w").is_some());
        assert!(browser_blocked(&window, &open("https://other.test/"), false, "w").is_none());
        // An interactive task may still try the browser after a fetch block.
        assert!(browser_blocked(&window, &open("https://www.g2.com/"), true, "w").is_none());
        // NOT_FOUND blocks the URL, not the host.
        let nf = vec![
            json!({"ok":false,"error_code":"NOT_FOUND","source":"fetch_url:https://x.test/a"}),
        ];
        assert!(browser_blocked(&nf, &open("https://x.test/a/"), false, "w").is_some());
        assert!(browser_blocked(&nf, &open("https://x.test/b"), false, "w").is_none());

        // The browser's own challenge observation, then reads and inputs.
        let mut first = json!({"action":"browser_open","ok":true});
        let page = json!({"url":"https://www.g2.com/","title":"Just a moment...","text":"Verifying you are human"});
        annotate_browser(&mut first, &page, &[], Some("https://www.g2.com/"));
        assert_eq!(first["ok"], false);
        assert_eq!(first["error_code"], "PLATFORM_BLOCKED");
        assert_eq!(first["source"], "browser:https://www.g2.com");
        let mut w = vec![first.clone()];
        assert!(browser_blocked(&w, &open("https://www.g2.com/"), true, "w").is_some());
        let read = json!({"tool":"browser_read"});
        assert!(
            browser_blocked(&w, &read, false, "w").is_none(),
            "one re-read is allowed"
        );
        assert!(
            browser_blocked(
                &w,
                &json!({"tool":"browser_click","selector":"#x"}),
                true,
                "w"
            )
            .is_some()
        );
        let mut again = json!({"action":"browser_read","ok":true});
        annotate_browser(
            &mut again,
            &json!({"url":null,"title":"Just a moment...","text":""}),
            &w,
            None,
        );
        assert_eq!(again["source"], "browser:https://www.g2.com");
        w.push(again);
        assert!(browser_blocked(&w, &read, false, "w").is_some());
        // A later clean read (challenge resolved) unblocks the page.
        let mut clean = json!({"action":"browser_read","ok":true});
        annotate_browser(
            &mut clean,
            &json!({"url":null,"title":"G2","text":"Business software reviews"}),
            &w,
            None,
        );
        assert_eq!(clean["ok"], true);
        w.push(clean);
        assert!(blocked_sources(&w).is_empty());
        assert!(browser_blocked(&w, &read, false, "w").is_none());

        let mut blank = json!({"action":"browser_open","ok":true});
        annotate_browser(
            &mut blank,
            &json!({"url":"https://www.g2.com/","title":"g2.com","text":""}),
            &[],
            None,
        );
        assert_eq!(blank["error_code"], "PLATFORM_BLOCKED");
        let mut fine = json!({"action":"browser_open","ok":true});
        annotate_browser(
            &mut fine,
            &json!({"url":"https://example.com/","title":"Example Domain","text":""}),
            &[],
            None,
        );
        assert_eq!(fine["ok"], true);
        assert!(fetched_before(&window, "https://www.g2.com"));
        assert!(fetched_before(
            &[json!({"action":"fetch:https://a.test/x","ok":true})],
            "https://a.test/x/"
        ));
        assert!(!fetched_before(&window, "https://a.test/"));
    }
}
