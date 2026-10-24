//! Podcasts: a directory to find shows in, the RSS feeds that carry their episodes, and the
//! downloads that keep episodes for the road.
//!
//! The directory is a trait ([`PodcastDirectory`]) so the app can look shows up in any
//! catalog that answers a search and a chart. [`Itunes`] uses Apple's public search and
//! lookup endpoints and its top-podcasts chart; [`FixtureDirectory`] reads the same JSON from
//! `resource/assets/podcast-fixture/` for scripted runs. A show can also be added by its feed
//! URL alone, which needs no directory at all.
//!
//! Feeds are RSS 2.0 with the iTunes namespace, which every podcast host publishes. The
//! parser here is the app's own: a small XML tokenizer and the dozen elements a podcast
//! player reads (title, author, image, enclosure, duration, publication date, summary).

use crate::library::{Episode, EpisodeFields, Podcast, PodcastFields, episode_id, podcast_id};
use day::prelude::*;
use day_part_http::Request;
use serde::Deserialize;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// The launch-time override, for scripts: `TUNES_PODCAST_DIRECTORY=asset:podcast-fixture`.
pub const DIRECTORY_ENV: &str = "TUNES_PODCAST_DIRECTORY";

type Fut<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// A show as a directory lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DirectoryShow {
    /// The directory's own id.
    pub id: String,
    pub title: String,
    pub author: String,
    pub feed: String,
    pub image: String,
    pub genre: String,
    pub episodes: i64,
    pub link: String,
}

/// A catalog of podcasts.
pub trait PodcastDirectory {
    fn name(&self) -> String;
    /// Shows matching free text.
    fn search(&self, term: &str) -> Fut<Vec<DirectoryShow>>;
    /// The chart: the most popular shows right now.
    fn top(&self, limit: usize) -> Fut<Vec<DirectoryShow>>;
}

// --- Apple's directory --------------------------------------------------------------------------

/// The iTunes Search API and the Apple podcasts chart.
pub struct Itunes {
    /// A two-letter storefront, `us` by default.
    country: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct LookupResponse {
    results: Vec<LookupResult>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct LookupResult {
    #[serde(rename = "collectionId")]
    collection_id: i64,
    #[serde(rename = "collectionName")]
    collection_name: String,
    #[serde(rename = "artistName")]
    artist_name: String,
    #[serde(rename = "feedUrl")]
    feed_url: String,
    #[serde(rename = "artworkUrl600")]
    artwork_600: String,
    #[serde(rename = "artworkUrl100")]
    artwork_100: String,
    #[serde(rename = "primaryGenreName")]
    genre: String,
    #[serde(rename = "trackCount")]
    track_count: i64,
    #[serde(rename = "collectionViewUrl")]
    view_url: String,
    kind: String,
}

impl LookupResult {
    fn into_show(self) -> Option<DirectoryShow> {
        if self.feed_url.is_empty() || self.collection_name.is_empty() {
            return None;
        }
        Some(DirectoryShow {
            id: self.collection_id.to_string(),
            title: self.collection_name,
            author: self.artist_name,
            feed: self.feed_url,
            image: if self.artwork_600.is_empty() {
                self.artwork_100
            } else {
                self.artwork_600
            },
            genre: self.genre,
            episodes: self.track_count,
            link: self.view_url,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ChartResponse {
    feed: ChartFeed,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ChartFeed {
    results: Vec<ChartEntry>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct ChartEntry {
    id: String,
}

/// Parse a search or lookup response into shows.
pub fn parse_lookup(text: &str) -> Result<Vec<DirectoryShow>, String> {
    let r: LookupResponse = serde_json::from_str(text)
        .map_err(|e| format!("the directory answer is not readable: {e}"))?;
    Ok(r.results
        .into_iter()
        .filter(|r| r.kind.is_empty() || r.kind == "podcast")
        .filter_map(LookupResult::into_show)
        .collect())
}

impl Itunes {
    pub fn new(country: &str) -> Itunes {
        let c = country.trim().to_ascii_lowercase();
        Itunes {
            country: if c.len() == 2 { c } else { "us".into() },
        }
    }
}

impl PodcastDirectory for Itunes {
    fn name(&self) -> String {
        "Apple Podcasts".to_string()
    }

    fn search(&self, term: &str) -> Fut<Vec<DirectoryShow>> {
        let url = format!(
            "https://itunes.apple.com/search?media=podcast&entity=podcast&limit=50&country={}&term={}",
            self.country,
            crate::radio::encode(term.trim())
        );
        Box::pin(async move {
            let text = get_text(&url).await?;
            parse_lookup(&text)
        })
    }

    fn top(&self, limit: usize) -> Fut<Vec<DirectoryShow>> {
        let limit = limit.clamp(1, 100);
        let country = self.country.clone();
        let chart = format!(
            "https://rss.marketingtools.apple.com/api/v2/{country}/podcasts/top/{limit}/podcasts.json"
        );
        Box::pin(async move {
            let text = get_text(&chart).await?;
            let r: ChartResponse = serde_json::from_str(&text)
                .map_err(|e| format!("the chart is not readable: {e}"))?;
            let ids: Vec<String> = r
                .feed
                .results
                .into_iter()
                .map(|e| e.id)
                .filter(|i| !i.is_empty())
                .collect();
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            // The chart has no feed URLs; one lookup of every id fills them in, in chart order.
            let lookup = format!(
                "https://itunes.apple.com/lookup?entity=podcast&country={country}&id={}",
                ids.join(",")
            );
            let text = get_text(&lookup).await?;
            let mut shows = parse_lookup(&text)?;
            shows.sort_by_key(|s| ids.iter().position(|i| *i == s.id).unwrap_or(usize::MAX));
            Ok(shows)
        })
    }
}

/// GET a small document through the platform's stack.
async fn get_text(url: &str) -> Result<String, String> {
    let req = Request::get(url)
        .header(
            "Accept",
            "application/json, application/rss+xml, application/xml, text/xml, */*",
        )
        .header("User-Agent", crate::radio::USER_AGENT)
        .timeout(std::time::Duration::from_secs(30));
    let response = day_part_http::fetch_future(req)
        .await
        .map_err(|e| format!("{e:?}"))?;
    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }
    Ok(response.text().into_owned())
}

// --- the fixture directory -------------------------------------------------------------------

/// Apple's lookup JSON, read from `resource/assets/<dir>/top.json` and `search.json` (filtered
/// by title here). Feeds in the fixture are `asset:<dir>/feed-<n>.xml`, which [`fetch_feed`]
/// reads from the bundle.
pub struct FixtureDirectory {
    dir: String,
}

impl FixtureDirectory {
    pub fn new(dir: &str) -> FixtureDirectory {
        FixtureDirectory {
            dir: dir.trim_matches('/').to_string(),
        }
    }
}

/// A bundled asset's text.
pub async fn asset_text(path: &str) -> Result<String, String> {
    #[cfg(target_arch = "wasm32")]
    {
        let url = format!("assets/data/{path}");
        let response = day_part_http::fetch_future(Request::get(url))
            .await
            .map_err(|e| format!("{path}: {e:?}"))?;
        if response.status != 200 {
            return Err(format!("{path}: HTTP {}", response.status));
        }
        Ok(response.text().into_owned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let res = day::resource(day::AssetName::dynamic(path.to_string()))
            .ok_or_else(|| format!("no bundled asset {path}"))?;
        Ok(String::from_utf8_lossy(res.as_slice()).into_owned())
    }
}

impl PodcastDirectory for FixtureDirectory {
    fn name(&self) -> String {
        format!("asset:{}", self.dir)
    }

    fn search(&self, term: &str) -> Fut<Vec<DirectoryShow>> {
        let path = format!("{}/search.json", self.dir);
        let q = term.trim().to_lowercase();
        Box::pin(async move {
            let text = asset_text(&path).await?;
            let mut shows = parse_lookup(&text)?;
            if !q.is_empty() {
                shows.retain(|s| {
                    s.title.to_lowercase().contains(&q) || s.author.to_lowercase().contains(&q)
                });
            }
            Ok(shows)
        })
    }

    fn top(&self, limit: usize) -> Fut<Vec<DirectoryShow>> {
        let path = format!("{}/top.json", self.dir);
        Box::pin(async move {
            let text = asset_text(&path).await?;
            Ok(parse_lookup(&text)?.into_iter().take(limit).collect())
        })
    }
}

// --- feeds ------------------------------------------------------------------------------------

/// A parsed feed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feed {
    pub title: String,
    pub author: String,
    pub summary: String,
    pub image: String,
    pub link: String,
    pub episodes: Vec<FeedEpisode>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeedEpisode {
    pub guid: String,
    pub title: String,
    pub summary: String,
    pub url: String,
    pub mime: String,
    pub bytes: i64,
    pub duration: f64,
    pub published: i64,
    pub image: String,
    pub link: String,
}

impl Feed {
    /// The subscription row for a feed fetched from `feed_url`.
    pub fn podcast(&self, feed_url: &str) -> Podcast {
        Podcast {
            id: podcast_id(feed_url),
            feed: feed_url.trim().to_string(),
            title: self.title.clone(),
            author: self.author.clone(),
            summary: self.summary.clone(),
            image: self.image.clone(),
            link: self.link.clone(),
            added: crate::library::now_secs(),
            refreshed: 0,
            etag: String::new(),
            modified: String::new(),
            episodes: self.episodes.len() as i64,
        }
    }

    /// The episode rows of a subscription.
    pub fn episode_rows(&self, feed_url: &str) -> Vec<Episode> {
        let podcast = podcast_id(feed_url);
        let now = crate::library::now_secs();
        self.episodes
            .iter()
            .filter(|e| !e.url.is_empty())
            .map(|e| Episode {
                id: episode_id(feed_url, &e.guid),
                podcast,
                guid: e.guid.clone(),
                title: e.title.clone(),
                summary: e.summary.clone(),
                url: e.url.clone(),
                mime: e.mime.clone(),
                bytes: e.bytes,
                duration: e.duration,
                published: e.published,
                image: e.image.clone(),
                position: 0.0,
                played: false,
                local: String::new(),
                download: crate::library::DOWNLOAD_NONE,
                added: now,
            })
            .collect()
    }
}

/// What a fetch answered: the feed, with the validators to send next time, or "unchanged".
pub enum FeedFetch {
    Changed {
        feed: Feed,
        etag: String,
        modified: String,
    },
    Unchanged,
}

/// Fetch and parse a feed, conditionally when validators are known. An `asset:` URL reads
/// the bundle, for the fixture.
pub async fn fetch_feed(url: &str, etag: &str, modified: &str) -> Result<FeedFetch, String> {
    if let Some(path) = url.strip_prefix("asset:") {
        let text = asset_text(path).await?;
        return Ok(FeedFetch::Changed {
            feed: parse_feed(&text)?,
            etag: String::new(),
            modified: String::new(),
        });
    }
    let mut req = Request::get(url)
        .header(
            "Accept",
            "application/rss+xml, application/xml, text/xml, */*",
        )
        .header("User-Agent", crate::radio::USER_AGENT)
        .timeout(std::time::Duration::from_secs(45));
    if !etag.is_empty() {
        req = req.header("If-None-Match", etag);
    }
    if !modified.is_empty() {
        req = req.header("If-Modified-Since", modified);
    }
    let response = day_part_http::fetch_future(req)
        .await
        .map_err(|e| format!("{e:?}"))?;
    if response.status == 304 {
        return Ok(FeedFetch::Unchanged);
    }
    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }
    let feed = parse_feed(&response.text())?;
    Ok(FeedFetch::Changed {
        feed,
        etag: response.header("etag").unwrap_or("").to_string(),
        modified: response.header("last-modified").unwrap_or("").to_string(),
    })
}

/// Parse an RSS 2.0 podcast feed.
pub fn parse_feed(xml: &str) -> Result<Feed, String> {
    let mut feed = Feed::default();
    let mut path: Vec<String> = Vec::new();
    let mut episode: Option<FeedEpisode> = None;
    let mut saw_channel = false;
    let mut text = String::new();
    // The channel's `<image><url>` is two levels deep; `<itunes:image href>` is an attribute.
    for event in Tokenizer::new(xml) {
        match event {
            Event::Start { name, attrs, empty } => {
                let local = local_name(&name).to_string();
                let in_item = episode.is_some();
                match (local.as_str(), in_item) {
                    ("item", false) => {
                        episode = Some(FeedEpisode::default());
                    }
                    ("channel", _) => saw_channel = true,
                    ("enclosure", true) => {
                        if let Some(ep) = episode.as_mut()
                            && ep.url.is_empty()
                        {
                            ep.url = attr(&attrs, "url");
                            ep.mime = attr(&attrs, "type");
                            ep.bytes = attr(&attrs, "length").trim().parse().unwrap_or(0);
                        }
                    }
                    ("image", _) if name.starts_with("itunes:") || name.contains(':') => {
                        let href = attr(&attrs, "href");
                        if !href.is_empty() {
                            match episode.as_mut() {
                                Some(ep) if ep.image.is_empty() => ep.image = href,
                                Some(_) => {}
                                None if feed.image.is_empty() => feed.image = href,
                                None => {}
                            }
                        }
                    }
                    _ => {}
                }
                text.clear();
                if !empty {
                    path.push(name);
                } else if local == "item" {
                    // `<item/>` carries nothing.
                    episode = None;
                }
            }
            Event::Text(t) => text.push_str(&t),
            Event::End(name) => {
                let local = local_name(&name).to_string();
                let value = clean_text(&text);
                let parent = path
                    .len()
                    .checked_sub(2)
                    .and_then(|i| path.get(i))
                    .map(|p| local_name(p).to_string())
                    .unwrap_or_default();
                match episode.as_mut() {
                    Some(ep) => match (name.as_str(), local.as_str()) {
                        (_, "item") => {
                            let done = episode.take();
                            if let Some(mut e) = done {
                                if e.guid.is_empty() {
                                    e.guid = if !e.url.is_empty() {
                                        e.url.clone()
                                    } else {
                                        format!("{}|{}", e.title, e.published)
                                    };
                                }
                                if !e.url.is_empty() || !e.title.is_empty() {
                                    feed.episodes.push(e);
                                }
                            }
                        }
                        ("title", _) if parent == "item" => ep.title = value,
                        ("guid", _) => ep.guid = value,
                        ("link", _) if parent == "item" => ep.link = value,
                        ("pubDate", _) => ep.published = parse_date(&value).unwrap_or(0),
                        ("itunes:duration", _) => ep.duration = parse_duration(&value),
                        ("itunes:summary", _) if ep.summary.is_empty() => {
                            ep.summary = strip_html(&value)
                        }
                        ("description", _) if parent == "item" && ep.summary.is_empty() => {
                            ep.summary = strip_html(&value)
                        }
                        ("content:encoded", _) if ep.summary.is_empty() => {
                            ep.summary = strip_html(&value)
                        }
                        ("itunes:subtitle", _) if ep.summary.is_empty() => {
                            ep.summary = strip_html(&value)
                        }
                        _ => {}
                    },
                    None => match (name.as_str(), local.as_str()) {
                        ("title", _) if parent == "channel" => feed.title = value,
                        ("itunes:author", _) if parent == "channel" => feed.author = value,
                        ("link", _) if parent == "channel" && !value.is_empty() => {
                            feed.link = value
                        }
                        ("description", _) if parent == "channel" && feed.summary.is_empty() => {
                            feed.summary = strip_html(&value)
                        }
                        ("itunes:summary", _) if parent == "channel" => {
                            feed.summary = strip_html(&value)
                        }
                        ("url", _) if parent == "image" && feed.image.is_empty() => {
                            feed.image = value
                        }
                        _ => {}
                    },
                }
                text.clear();
                if path.last() == Some(&name) {
                    path.pop();
                }
            }
        }
    }
    if !saw_channel && feed.episodes.is_empty() {
        return Err("not a podcast feed".into());
    }
    if feed.title.is_empty() {
        feed.title = feed.link.clone();
    }
    Ok(feed)
}

fn attr(attrs: &[(String, String)], name: &str) -> String {
    attrs
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default()
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn clean_text(s: &str) -> String {
    s.trim().to_string()
}

/// Text without its markup: tags dropped, entities decoded, blank runs collapsed, paragraph
/// breaks kept as one newline.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for c in s.chars() {
        if in_tag {
            if c == '>' {
                in_tag = false;
                let t = tag.trim_matches('/').to_ascii_lowercase();
                let t = t.split_whitespace().next().unwrap_or("");
                if matches!(
                    t,
                    "p" | "br" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "ul" | "ol"
                ) {
                    out.push('\n');
                }
                tag.clear();
            } else {
                tag.push(c);
            }
        } else if c == '<' {
            in_tag = true;
        } else {
            out.push(c);
        }
    }
    let decoded = decode_entities(&out);
    // Collapse runs of spaces and runs of line breaks: one break between blocks.
    let mut result = String::with_capacity(decoded.len());
    let mut newlines = 0;
    let mut space = false;
    for c in decoded.chars() {
        if c == '\n' {
            newlines += 1;
            space = false;
            if newlines == 1 && !result.is_empty() {
                result.push('\n');
            }
        } else if c.is_whitespace() {
            if !space && !result.ends_with('\n') {
                result.push(' ');
            }
            space = true;
        } else {
            newlines = 0;
            space = false;
            result.push(c);
        }
    }
    result.trim().to_string()
}

/// `&amp;`, `&#39;`, `&#x27;` and the handful of named entities feeds use.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "copy" => Some('©'),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| {
                    if let Some(h) = n.strip_prefix(['x', 'X']) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        n.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `1:02:03`, `62:03`, or `3723` seconds.
pub fn parse_duration(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    let parts: Vec<&str> = s.split(':').collect();
    let mut total = 0.0;
    for p in &parts {
        let v: f64 = p.trim().parse().unwrap_or(0.0);
        total = total * 60.0 + v;
    }
    if parts.len() == 1 && total > 0.0 && total < 100.0 && s.contains('.') {
        // "12.5" is seconds too.
        return total;
    }
    total.max(0.0)
}

/// RFC 2822 (`Tue, 10 Jun 2025 08:00:00 GMT`, `+0200`, `EST`) and the ISO 8601 a few feeds
/// use, to seconds since the epoch.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(t) = parse_iso8601(s) {
        return Some(t);
    }
    let mut parts: Vec<&str> = s.split_whitespace().collect();
    // Drop the weekday.
    if parts
        .first()
        .is_some_and(|p| p.ends_with(',') || p.len() == 3 && parts.len() > 4)
    {
        parts.remove(0);
    }
    if parts.len() < 4 {
        return None;
    }
    let day: i64 = parts[0].trim_end_matches(',').parse().ok()?;
    let month = month_of(parts[1])?;
    let mut year: i64 = parts[2].parse().ok()?;
    if year < 100 {
        year += if year < 70 { 2000 } else { 1900 };
    }
    let (h, m, sec) = parse_hms(parts[3])?;
    let offset = parts.get(4).map(|z| zone_offset(z)).unwrap_or(0);
    Some(days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + sec - offset)
}

fn parse_iso8601(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let month: i64 = s[5..7].parse().ok()?;
    let day: i64 = s[8..10].parse().ok()?;
    let mut secs = days_from_civil(year, month, day) * 86_400;
    if b.len() >= 16 && (b[10] == b'T' || b[10] == b' ') {
        let (h, m, sec) = parse_hms(&s[11..])?;
        secs += h * 3600 + m * 60 + sec;
        let tail = &s[11..];
        let zone_at = tail.find(['Z', '+', '-']).map(|i| &tail[i..]);
        if let Some(z) = zone_at {
            secs -= zone_offset(z);
        }
    }
    Some(secs)
}

fn parse_hms(s: &str) -> Option<(i64, i64, i64)> {
    let core: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ':')
        .collect();
    let mut it = core.split(':');
    let h: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next().unwrap_or("0").parse().ok()?;
    let sec: i64 = it.next().unwrap_or("0").parse().ok()?;
    Some((h, m, sec))
}

fn month_of(s: &str) -> Option<i64> {
    let m = s.get(..3)?.to_ascii_lowercase();
    Some(match m.as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    })
}

/// `+0200` → 7200, `-05:00` → -18000, `GMT`/`Z`/`UT` → 0, the US zone names.
fn zone_offset(z: &str) -> i64 {
    let z = z.trim();
    match z.to_ascii_uppercase().as_str() {
        "Z" | "GMT" | "UT" | "UTC" => return 0,
        "EST" => return -5 * 3600,
        "EDT" => return -4 * 3600,
        "CST" => return -6 * 3600,
        "CDT" => return -5 * 3600,
        "MST" => return -7 * 3600,
        "MDT" => return -6 * 3600,
        "PST" => return -8 * 3600,
        "PDT" => return -7 * 3600,
        _ => {}
    }
    let (sign, digits) = match z.as_bytes().first() {
        Some(b'+') => (1, &z[1..]),
        Some(b'-') => (-1, &z[1..]),
        _ => return 0,
    };
    let digits: String = digits.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return 0;
    }
    let h: i64 = digits[0..2].parse().unwrap_or(0);
    let m: i64 = digits[2..4].parse().unwrap_or(0);
    sign * (h * 3600 + m * 60)
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// --- the XML tokenizer -------------------------------------------------------------------------

enum Event {
    Start {
        name: String,
        attrs: Vec<(String, String)>,
        empty: bool,
    },
    End(String),
    Text(String),
}

/// Enough XML for a feed: elements, attributes, text, CDATA, entities; comments, processing
/// instructions and doctypes skipped. Malformed input ends the stream rather than erroring.
struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(src: &'a str) -> Tokenizer<'a> {
        Tokenizer { src, pos: 0 }
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = Event;
    fn next(&mut self) -> Option<Event> {
        loop {
            let rest = &self.src[self.pos..];
            if rest.is_empty() {
                return None;
            }
            if let Some(after) = rest.strip_prefix('<') {
                if let Some(cdata) = after.strip_prefix("![CDATA[") {
                    let end = cdata.find("]]>")?;
                    self.pos += 1 + 8 + end + 3;
                    return Some(Event::Text(cdata[..end].to_string()));
                }
                if after.starts_with("!--") {
                    let end = after.find("-->")?;
                    self.pos += 1 + end + 3;
                    continue;
                }
                if after.starts_with('?') || after.starts_with('!') {
                    let end = after.find('>')?;
                    self.pos += 1 + end + 1;
                    continue;
                }
                if let Some(close) = after.strip_prefix('/') {
                    let end = close.find('>')?;
                    let name = close[..end].trim().to_string();
                    self.pos += 2 + end + 1;
                    return Some(Event::End(name));
                }
                // A start tag; attribute values may hold '>'.
                let mut i = 0;
                let bytes = after.as_bytes();
                let mut quote: u8 = 0;
                while i < bytes.len() {
                    let b = bytes[i];
                    if quote != 0 {
                        if b == quote {
                            quote = 0;
                        }
                    } else if b == b'"' || b == b'\'' {
                        quote = b;
                    } else if b == b'>' {
                        break;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return None;
                }
                let inner = &after[..i];
                self.pos += 1 + i + 1;
                let (inner, empty) = match inner.strip_suffix('/') {
                    Some(s) => (s, true),
                    None => (inner, false),
                };
                let mut parts = inner.splitn(2, |c: char| c.is_whitespace());
                let name = parts.next().unwrap_or("").to_string();
                let attrs = parse_attrs(parts.next().unwrap_or(""));
                return Some(Event::Start { name, attrs, empty });
            }
            let end = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..end];
            self.pos += end;
            if text.trim().is_empty() {
                continue;
            }
            return Some(Event::Text(decode_entities(text)));
        }
    }
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    while let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next() else { break };
        if q != '"' && q != '\'' {
            break;
        }
        let Some(end) = after[1..].find(q) else { break };
        let value = decode_entities(&after[1..1 + end]);
        if !name.is_empty() {
            out.push((name, value));
        }
        rest = after[1 + end + 1..].trim_start();
    }
    out
}

// --- the app-wide podcasts ---------------------------------------------------------------------

/// What a download is doing.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: u64,
}

/// The app's podcasts: the directory in use, feed refreshes, and downloads in flight.
pub struct Podcasts {
    directory: std::cell::RefCell<Rc<dyn PodcastDirectory>>,
    pub directory_name: Signal<String>,
    /// The chart and the last search, cached.
    pub top: Signal<crate::radio::Fetched<Vec<DirectoryShow>>>,
    pub search: Signal<crate::radio::Fetched<Vec<DirectoryShow>>>,
    pub search_term: Signal<String>,
    /// Feeds previewed from the directory, by URL.
    previews:
        std::cell::RefCell<std::collections::HashMap<String, Signal<crate::radio::Fetched<Feed>>>>,
    /// How many refreshes are in flight.
    pub refreshing: Signal<usize>,
    /// Downloads in flight, by episode id.
    pub downloads: Signal<std::collections::HashMap<u64, DownloadProgress>>,
    /// The last refresh's failures, for Settings.
    pub last_error: Signal<Option<String>>,
    /// Directory results seen this session, by feed, so a route can name one by its feed.
    shows: std::cell::RefCell<std::collections::HashMap<String, DirectoryShow>>,
}

impl Podcasts {
    pub fn new(country: &str) -> Podcasts {
        let directory: Rc<dyn PodcastDirectory> =
            match day::env(DIRECTORY_ENV).filter(|s| !s.is_empty()) {
                Some(base) if base.starts_with("asset:") => {
                    Rc::new(FixtureDirectory::new(base.trim_start_matches("asset:")))
                }
                _ => Rc::new(Itunes::new(country)),
            };
        let name = directory.name();
        Podcasts {
            directory: std::cell::RefCell::new(directory),
            directory_name: Signal::new(name),
            top: Signal::new(crate::radio::Fetched::Loading),
            search: Signal::new(crate::radio::Fetched::Ready(Vec::new())),
            search_term: Signal::new(String::new()),
            previews: std::cell::RefCell::new(std::collections::HashMap::new()),
            refreshing: Signal::new(0),
            downloads: Signal::new(std::collections::HashMap::new()),
            last_error: Signal::new(None),
            shows: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    /// Keep a directory result so a pushed page can find it again by feed.
    pub fn remember_show(&self, show: &DirectoryShow) {
        self.shows
            .borrow_mut()
            .insert(show.feed.clone(), show.clone());
    }

    pub fn show_for(&self, feed: &str) -> Option<DirectoryShow> {
        self.shows.borrow().get(feed).cloned()
    }

    pub fn set_country(&'static self, country: &str) {
        if day::env(DIRECTORY_ENV).is_some_and(|s| !s.is_empty()) {
            return;
        }
        *self.directory.borrow_mut() = Rc::new(Itunes::new(country));
        self.top.set(crate::radio::Fetched::Loading);
        self.load_top();
    }

    /// Fetch the chart (once; `refresh_top` asks again).
    pub fn load_top(&'static self) {
        let fut = self.directory.borrow().top(50);
        let top = self.top;
        day::task(async move {
            match fut.await {
                Ok(rows) => top.set(crate::radio::Fetched::Ready(rows)),
                Err(e) => {
                    error!("podcast chart: {e}");
                    top.set(crate::radio::Fetched::Failed(e));
                }
            }
        });
    }

    pub fn refresh_top(&'static self) {
        self.top.set(crate::radio::Fetched::Loading);
        self.load_top();
    }

    /// Run a search; an empty term clears the results.
    pub fn run_search(&'static self, term: &str) {
        let term = term.trim().to_string();
        self.search_term.set(term.clone());
        if term.is_empty() {
            self.search.set(crate::radio::Fetched::Ready(Vec::new()));
            return;
        }
        self.search.set(crate::radio::Fetched::Loading);
        let fut = self.directory.borrow().search(&term);
        let search = self.search;
        let current = self.search_term;
        day::task(async move {
            let result = fut.await;
            if current.get_untracked() != term {
                return;
            }
            match result {
                Ok(rows) => search.set(crate::radio::Fetched::Ready(rows)),
                Err(e) => search.set(crate::radio::Fetched::Failed(e)),
            }
        });
    }

    /// A feed fetched for preview (the directory's show page before subscribing).
    pub fn preview(&'static self, feed_url: &str) -> Signal<crate::radio::Fetched<Feed>> {
        if let Some(s) = self.previews.borrow().get(feed_url) {
            return *s;
        }
        let signal = Signal::global(crate::radio::Fetched::Loading);
        self.previews
            .borrow_mut()
            .insert(feed_url.to_string(), signal);
        let url = feed_url.to_string();
        day::task(async move {
            match fetch_feed(&url, "", "").await {
                Ok(FeedFetch::Changed { feed, .. }) => {
                    signal.set(crate::radio::Fetched::Ready(feed))
                }
                Ok(FeedFetch::Unchanged) => {
                    signal.set(crate::radio::Fetched::Failed("unchanged".into()))
                }
                Err(e) => signal.set(crate::radio::Fetched::Failed(e)),
            }
        });
        signal
    }

    /// Subscribe to a feed: fetch it, make the rows. `done` receives the subscription id.
    pub fn subscribe(&'static self, feed_url: &str, done: Signal<Option<Result<u64, String>>>) {
        let url = feed_url.trim().to_string();
        let lib = crate::App::app().library;
        if lib.is_subscribed(&url) {
            done.set(Some(Ok(podcast_id(&url))));
            return;
        }
        self.refreshing.update(|n| *n += 1);
        day::task(async move {
            let result = fetch_feed(&url, "", "").await;
            self.refreshing.update(|n| *n = n.saturating_sub(1));
            match result {
                Ok(FeedFetch::Changed {
                    feed,
                    etag,
                    modified,
                }) => {
                    let mut p = feed.podcast(&url);
                    p.etag = etag;
                    p.modified = modified;
                    let id = lib.subscribe(p);
                    lib.merge_episodes(id, feed.episode_rows(&url));
                    done.set(Some(Ok(id)));
                }
                Ok(FeedFetch::Unchanged) => done.set(Some(Err("the feed sent nothing".into()))),
                Err(e) => done.set(Some(Err(e))),
            }
        });
    }

    /// Refresh one subscription.
    pub fn refresh(&'static self, podcast: u64) {
        let lib = crate::App::app().library;
        let Some(p) = lib.podcast(podcast) else {
            return;
        };
        self.refreshing.update(|n| *n += 1);
        day::task(async move {
            let result = fetch_feed(&p.feed, &p.etag, &p.modified).await;
            self.refreshing.update(|n| *n = n.saturating_sub(1));
            match result {
                Ok(FeedFetch::Changed {
                    feed,
                    etag,
                    modified,
                }) => {
                    if let Some(row) = lib.container.get::<Podcast>(podcast) {
                        if !feed.title.is_empty() {
                            row.title().write(feed.title.clone());
                        }
                        row.author().write(feed.author.clone());
                        if !feed.summary.is_empty() {
                            row.summary().write(feed.summary.clone());
                        }
                        if !feed.image.is_empty() {
                            row.image().write(feed.image.clone());
                        }
                        row.etag().write(etag);
                        row.modified().write(modified);
                    }
                    let added = lib.merge_episodes(podcast, feed.episode_rows(&p.feed));
                    if added > 0 {
                        info!("{}: {added} new episode(s)", p.title);
                    }
                }
                Ok(FeedFetch::Unchanged) => {
                    if let Some(row) = lib.container.get::<Podcast>(podcast) {
                        row.refreshed().write(crate::library::now_secs());
                    }
                }
                Err(e) => {
                    warn!("{}: {e}", p.title);
                    self.last_error.set(Some(format!("{}: {e}", p.title)));
                }
            }
        });
    }

    /// Refresh every subscription.
    pub fn refresh_all(&'static self) {
        let lib = crate::App::app().library;
        self.last_error.set(None);
        for id in crate::library::Library::ids_of(&lib.podcasts()) {
            self.refresh(id);
        }
    }

    /// Download an episode for offline listening, on a worker thread, with progress.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn download(&'static self, episode: u64) {
        use day_part_http::{HttpError, StreamSink, fetch_streamed};
        use std::io::Write;
        let lib = crate::App::app().library;
        let Some(ep) = lib.episode(episode) else {
            return;
        };
        if ep.download == crate::library::DOWNLOAD_RUNNING
            || self.downloads.with_untracked(|d| d.contains_key(&episode))
        {
            return;
        }
        let Some(dest) = lib.episode_download_path(&ep) else {
            return;
        };
        if let Some(row) = lib.container.get::<Episode>(episode) {
            row.download().write(crate::library::DOWNLOAD_RUNNING);
        }
        self.downloads.update(|d| {
            d.insert(
                episode,
                DownloadProgress {
                    received: 0,
                    total: ep.bytes.max(0) as u64,
                },
            );
        });
        let url = ep.url.clone();
        let part = dest.with_extension("part");
        std::thread::spawn(move || {
            struct Sink {
                file: std::fs::File,
                received: u64,
                total: u64,
                episode: u64,
                last_report: std::time::Instant,
            }
            impl StreamSink for Sink {
                fn head(&mut self, status: u16, headers: &[(String, String)]) -> bool {
                    if status != 200 {
                        return false;
                    }
                    if let Some(len) = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| v.trim().parse::<u64>().ok())
                    {
                        self.total = len;
                    }
                    true
                }
                fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
                    self.file
                        .write_all(data)
                        .map_err(|e| HttpError::Io(e.to_string()))?;
                    self.received += data.len() as u64;
                    if self.last_report.elapsed().as_millis() >= 200 {
                        self.last_report = std::time::Instant::now();
                        let (episode, received, total) = (self.episode, self.received, self.total);
                        day::reactive::on_main(move || {
                            crate::App::app().podcasts.downloads.update(|d| {
                                d.insert(episode, DownloadProgress { received, total });
                            });
                        });
                    }
                    Ok(())
                }
            }
            let outcome = (|| -> Result<(), String> {
                let file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
                let mut sink = Sink {
                    file,
                    received: 0,
                    total: 0,
                    episode,
                    last_report: std::time::Instant::now(),
                };
                let req = Request::get(&url)
                    .header("User-Agent", crate::radio::USER_AGENT)
                    .timeout(std::time::Duration::from_secs(120))
                    .allow_expensive(true);
                let result = fetch_streamed(&req, &mut sink).map_err(|e| format!("{e:?}"))?;
                drop(sink);
                if result.status != 200 {
                    return Err(format!("HTTP {}", result.status));
                }
                let _ = std::fs::remove_file(&dest);
                std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
                Ok(())
            })();
            if outcome.is_err() {
                let _ = std::fs::remove_file(&part);
            }
            let local = dest.to_string_lossy().into_owned();
            day::reactive::on_main(move || {
                let app = crate::App::app();
                app.podcasts.downloads.update(|d| {
                    d.remove(&episode);
                });
                if let Some(row) = app.library.container.get::<Episode>(episode) {
                    match outcome {
                        Ok(()) => {
                            row.local().write(local);
                            row.download().write(crate::library::DOWNLOAD_DONE);
                        }
                        Err(e) => {
                            error!("download failed: {e}");
                            row.download().write(crate::library::DOWNLOAD_FAILED);
                        }
                    }
                }
            });
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub fn download(&'static self, _episode: u64) {}

    /// Delete a downloaded copy; the episode streams again.
    pub fn remove_download(&'static self, episode: u64) {
        let lib = crate::App::app().library;
        if let Some(row) = lib.container.get::<Episode>(episode) {
            let local = row.local().read();
            if !local.is_empty() {
                let _ = std::fs::remove_file(local);
            }
            row.local().write(String::new());
            row.download().write(crate::library::DOWNLOAD_NONE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd" xmlns:content="http://purl.org/rss/1.0/modules/content/" version="2.0">
<channel>
  <title>Morning Signal</title>
  <link>https://example.org/signal</link>
  <description><![CDATA[<p>A show about <b>mornings</b> &amp; coffee.</p>]]></description>
  <itunes:author>Ada &amp; Co</itunes:author>
  <itunes:image href="https://example.org/cover.jpg"/>
  <image><url>https://example.org/old.jpg</url><title>x</title></image>
  <item>
    <title>Episode 2: Steam</title>
    <guid isPermaLink="false">ep-2</guid>
    <pubDate>Tue, 10 Jun 2025 08:00:00 +0200</pubDate>
    <enclosure url="https://cdn.example.org/ep2.mp3?x=1" length="12345678" type="audio/mpeg"/>
    <itunes:duration>1:02:03</itunes:duration>
    <itunes:summary>Second &#x27;episode&#x27;.</itunes:summary>
    <itunes:image href="https://example.org/ep2.jpg"/>
  </item>
  <item>
    <title>Episode 1</title>
    <description>&lt;p&gt;First.&lt;/p&gt;</description>
    <pubDate>Mon, 02 Jun 2025 08:00:00 GMT</pubDate>
    <enclosure url="https://cdn.example.org/ep1.m4a" type="audio/x-m4a" />
    <itunes:duration>1830</itunes:duration>
  </item>
  <item><title>No audio</title></item>
</channel>
</rss>"#;

    #[test]
    fn a_feed_parses() {
        let feed = parse_feed(FEED).expect("parses");
        assert_eq!(feed.title, "Morning Signal");
        assert_eq!(feed.author, "Ada & Co");
        assert_eq!(feed.image, "https://example.org/cover.jpg");
        assert_eq!(feed.summary, "A show about mornings & coffee.");
        assert_eq!(feed.link, "https://example.org/signal");
        assert_eq!(feed.episodes.len(), 3);
        let e = &feed.episodes[0];
        assert_eq!(e.title, "Episode 2: Steam");
        assert_eq!(e.guid, "ep-2");
        assert_eq!(e.url, "https://cdn.example.org/ep2.mp3?x=1");
        assert_eq!(e.mime, "audio/mpeg");
        assert_eq!(e.bytes, 12_345_678);
        assert_eq!(e.duration, 3723.0);
        assert_eq!(e.summary, "Second 'episode'.");
        assert_eq!(e.image, "https://example.org/ep2.jpg");
        assert_eq!(e.published, 1_749_535_200);
        let e1 = &feed.episodes[1];
        assert_eq!(e1.guid, "https://cdn.example.org/ep1.m4a");
        assert_eq!(e1.summary, "First.");
        assert_eq!(e1.duration, 1830.0);
        assert_eq!(e1.published, 1_748_851_200);
        let rows = feed.episode_rows("https://example.org/feed.xml");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].podcast, podcast_id("https://example.org/feed.xml"));
    }

    #[test]
    fn dates_and_durations() {
        assert_eq!(parse_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_date("01 Jan 1970 01:00:00 +0100"), Some(0));
        assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_date("2000-03-01"), Some(951_868_800));
        assert_eq!(
            parse_date("Wed, 1 Jan 2025 12:00:00 EST"),
            Some(1_735_750_800)
        );
        assert_eq!(parse_date("junk"), None);
        assert_eq!(parse_duration("45"), 45.0);
        assert_eq!(parse_duration("02:30"), 150.0);
        assert_eq!(parse_duration(""), 0.0);
    }

    #[test]
    fn html_is_stripped_and_entities_decoded() {
        assert_eq!(
            strip_html("<p>Hi<br/>there &amp; you</p>  <p>Two</p>"),
            "Hi\nthere & you\nTwo"
        );
        assert_eq!(
            decode_entities("a &lt; b &#65; &#x42; &bogus; &"),
            "a < b A B &bogus; &"
        );
    }

    #[test]
    fn lookup_json_parses() {
        let text = r#"{"resultCount":2,"results":[
          {"kind":"podcast","collectionId":1,"collectionName":"Show","artistName":"Host","feedUrl":"https://f/1","artworkUrl600":"https://a/600.jpg","primaryGenreName":"Tech","trackCount":12,"collectionViewUrl":"https://v/1"},
          {"kind":"podcast","collectionId":2,"collectionName":"No feed"}]}"#;
        let shows = parse_lookup(text).expect("parses");
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].id, "1");
        assert_eq!(shows[0].image, "https://a/600.jpg");
    }

    #[test]
    fn malformed_xml_does_not_panic() {
        assert!(parse_feed("<rss><channel><title>x").is_ok());
        assert!(parse_feed("nothing here").is_err());
        assert!(parse_feed("<a b=\"unterminated").is_err());
    }
}
