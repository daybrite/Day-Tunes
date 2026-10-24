//! Internet radio: an index of stations the listener browses and searches, behind one trait
//! so the app can take its stations from any directory that answers the same questions.
//!
//! [`RadioBrowser`] speaks the Community Radio Browser API (https://www.radio-browser.info),
//! the public index most players draw from; its JSON is also what the bundled fixture speaks,
//! so [`FixtureIndex`] reads the same shape from `resource/assets/radio-fixture/` for scripted
//! runs with no network. The index to use comes from the launch environment
//! (`TUNES_RADIO_INDEX=asset:radio-fixture`) or Settings (a server of the listener's choice);
//! left alone, the app asks the project's server list and picks one.
//!
//! Results are cached per browse for the life of the process ([`Radio::stations`]), so going
//! back to a country or a genre is instant and a flaky network is asked once.

use crate::library::{Station, station_id};
use day::prelude::*;
use day_part_http::Request;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// The app's name on the wire: the Radio Browser project asks every client to say who it is.
pub const USER_AGENT: &str = "DayTunes/0.2 (+https://daybrite.dev)";
/// The launch-time override, for scripts.
pub const INDEX_ENV: &str = "TUNES_RADIO_INDEX";
/// The server list the public index publishes; one of its entries answers the API.
const SERVER_LIST: &str = "https://all.api.radio-browser.info/json/servers";
/// Where to turn when the list cannot be read.
const FALLBACK_SERVERS: [&str; 3] = [
    "de2.api.radio-browser.info",
    "fi1.api.radio-browser.info",
    "de1.api.radio-browser.info",
];
/// How many stations one browse page asks for.
pub const PAGE: usize = 200;

type Fut<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// One cut of the index: what a station list shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Browse {
    /// The most voted stations.
    Top,
    /// The most played stations lately.
    Trending,
    Country(String),
    Tag(String),
    Language(String),
    Search(String),
}

/// The three lists a listener browses by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FacetKind {
    Countries,
    Tags,
    Languages,
}

/// One entry of a Browse list: a country, tag, or language and its station count.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Facet {
    /// The key a [`Browse`] is built from: a country code, a tag, a language name.
    pub key: String,
    /// What the row shows.
    pub name: String,
    pub stations: i64,
}

/// A directory of internet radio stations.
pub trait RadioIndex {
    /// The directory's name, for Settings and About.
    fn name(&self) -> String;
    /// Up to `limit` stations of one browse, from `offset`.
    fn stations(&self, browse: &Browse, limit: usize, offset: usize) -> Fut<Vec<Station>>;
    /// The countries, tags, or languages the index knows, with counts, most stations first.
    fn facets(&self, kind: FacetKind) -> Fut<Vec<Facet>>;
    /// Tell the index a station was played (the Radio Browser project counts clicks).
    fn played(&self, uuid: &str) -> Fut<()>;
    /// Vote a station up.
    fn vote(&self, uuid: &str) -> Fut<()>;
}

// --- the JSON shape --------------------------------------------------------------------------

/// A station as the Radio Browser API spells it (and as the fixture files do).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct IndexStation {
    pub stationuuid: String,
    pub name: String,
    pub url: String,
    pub url_resolved: String,
    pub homepage: String,
    pub favicon: String,
    pub tags: String,
    pub country: String,
    pub countrycode: String,
    pub state: String,
    pub language: String,
    pub votes: i64,
    pub codec: String,
    pub bitrate: i64,
    #[serde(deserialize_with = "int_or_bool")]
    pub hls: i64,
    #[serde(deserialize_with = "int_or_bool")]
    pub lastcheckok: i64,
    pub clickcount: i64,
}

/// `hls` and `lastcheckok` are `0`/`1` on the wire; a fixture written by hand may say `true`.
fn int_or_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum V {
        I(i64),
        B(bool),
        S(String),
    }
    Ok(match V::deserialize(d)? {
        V::I(i) => i,
        V::B(b) => b as i64,
        V::S(s) => s.parse().unwrap_or(0),
    })
}

impl IndexStation {
    /// The app's own row for this station; the listener's columns start empty.
    pub fn into_station(self) -> Station {
        Station {
            id: station_id(&self.stationuuid),
            uuid: self.stationuuid,
            name: self.name.trim().to_string(),
            url: self.url,
            url_resolved: self.url_resolved,
            homepage: self.homepage,
            favicon: self.favicon,
            tags: self.tags,
            country: self.country,
            countrycode: self.countrycode.to_ascii_uppercase(),
            state: self.state,
            language: self.language,
            codec: self.codec.to_ascii_uppercase(),
            bitrate: self.bitrate,
            hls: self.hls != 0,
            votes: self.votes,
            favorite: false,
            added: 0,
            last_played: 0,
            play_count: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct IndexCountry {
    name: String,
    iso_3166_1: String,
    stationcount: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct IndexTag {
    name: String,
    stationcount: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct IndexServer {
    name: String,
}

/// Parse a station list, dropping rows with no stream and duplicates by uuid.
pub fn parse_stations(text: &str) -> Result<Vec<Station>, String> {
    let rows: Vec<IndexStation> =
        serde_json::from_str(text).map_err(|e| format!("the station list is not readable: {e}"))?;
    let mut seen = std::collections::HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|s| !s.url.trim().is_empty() && !s.stationuuid.is_empty())
        .filter(|s| seen.insert(s.stationuuid.clone()))
        .map(IndexStation::into_station)
        .collect())
}

fn parse_facets(kind: FacetKind, text: &str) -> Result<Vec<Facet>, String> {
    let err = |e: serde_json::Error| format!("the list is not readable: {e}");
    let mut out: Vec<Facet> = match kind {
        FacetKind::Countries => serde_json::from_str::<Vec<IndexCountry>>(text)
            .map_err(err)?
            .into_iter()
            .filter(|c| c.iso_3166_1.len() == 2 && !c.name.is_empty())
            .map(|c| Facet {
                key: c.iso_3166_1.to_ascii_uppercase(),
                name: c.name,
                stations: c.stationcount,
            })
            .collect(),
        FacetKind::Tags | FacetKind::Languages => serde_json::from_str::<Vec<IndexTag>>(text)
            .map_err(err)?
            .into_iter()
            .filter(|t| !t.name.trim().is_empty())
            .map(|t| Facet {
                key: t.name.clone(),
                name: t.name,
                stations: t.stationcount,
            })
            .collect(),
    };
    // One row per key: the index lists a country under several spellings now and then.
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.key.to_lowercase()));
    out.sort_by(|a, b| {
        b.stations
            .cmp(&a.stations)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

// --- the live index ----------------------------------------------------------------------------

/// The Radio Browser API, on one of its mirrors.
#[derive(Clone)]
pub struct RadioBrowser {
    /// `https://<host>`; empty until a server is picked. Shared with every future in flight.
    server: Rc<RefCell<String>>,
    /// The listener's own choice of server, or empty for the project's list.
    preferred: String,
}

impl RadioBrowser {
    pub fn new(preferred: &str) -> RadioBrowser {
        let preferred = preferred.trim().trim_end_matches('/');
        let preferred = if preferred.is_empty() {
            String::new()
        } else if preferred.contains("://") {
            preferred.to_string()
        } else {
            format!("https://{preferred}")
        };
        RadioBrowser {
            server: Rc::new(RefCell::new(preferred.clone())),
            preferred,
        }
    }

    /// The server to talk to: the preferred one, the one picked earlier, or a fresh pick.
    async fn server(&self) -> Result<String, String> {
        if !self.preferred.is_empty() {
            return Ok(self.preferred.clone());
        }
        let known = self.server.borrow().clone();
        if !known.is_empty() {
            return Ok(known);
        }
        let picked = match get_text(SERVER_LIST).await {
            Ok(text) => {
                let servers: Vec<IndexServer> = serde_json::from_str(&text).unwrap_or_default();
                let names: Vec<String> = servers
                    .into_iter()
                    .map(|s| s.name)
                    .filter(|n| !n.is_empty())
                    .collect();
                if names.is_empty() {
                    None
                } else {
                    let pick = (crate::library::now_secs() as usize) % names.len();
                    Some(names[pick].clone())
                }
            }
            Err(e) => {
                warn!("radio server list unavailable: {e}");
                None
            }
        };
        let host = picked.unwrap_or_else(|| {
            let pick = (crate::library::now_secs() as usize) % FALLBACK_SERVERS.len();
            FALLBACK_SERVERS[pick].to_string()
        });
        let server = format!("https://{host}");
        info!("radio index: {server}");
        *self.server.borrow_mut() = server.clone();
        Ok(server)
    }

    /// Forget the picked server, so the next call picks again (after a failure).
    fn reset(&self) {
        if self.preferred.is_empty() {
            self.server.borrow_mut().clear();
        }
    }

    fn path_for(browse: &Browse, limit: usize, offset: usize) -> String {
        let page = format!("hidebroken=true&limit={limit}&offset={offset}");
        match browse {
            Browse::Top => format!("/json/stations/topvote?{page}"),
            Browse::Trending => format!("/json/stations/topclick?{page}"),
            Browse::Country(code) => format!(
                "/json/stations/bycountrycodeexact/{}?order=votes&reverse=true&{page}",
                encode(code)
            ),
            Browse::Tag(tag) => format!(
                "/json/stations/bytagexact/{}?order=votes&reverse=true&{page}",
                encode(tag)
            ),
            Browse::Language(lang) => format!(
                "/json/stations/bylanguageexact/{}?order=votes&reverse=true&{page}",
                encode(lang)
            ),
            Browse::Search(q) => format!(
                "/json/stations/search?name={}&order=votes&reverse=true&{page}",
                encode(q.trim())
            ),
        }
    }
}

impl RadioIndex for RadioBrowser {
    fn name(&self) -> String {
        let s = self.server.borrow();
        if s.is_empty() {
            "radio-browser.info".to_string()
        } else {
            s.trim_start_matches("https://").to_string()
        }
    }

    fn stations(&self, browse: &Browse, limit: usize, offset: usize) -> Fut<Vec<Station>> {
        let path = Self::path_for(browse, limit, offset);
        let this = self.clone();
        Box::pin(async move {
            let server = this.server().await?;
            match get_text(&format!("{server}{path}")).await {
                Ok(text) => parse_stations(&text),
                Err(e) => {
                    this.reset();
                    Err(e)
                }
            }
        })
    }

    fn facets(&self, kind: FacetKind) -> Fut<Vec<Facet>> {
        let path = match kind {
            FacetKind::Countries => {
                "/json/countries?order=stationcount&reverse=true&hidebroken=true"
            }
            FacetKind::Tags => {
                "/json/tags?order=stationcount&reverse=true&hidebroken=true&limit=400"
            }
            FacetKind::Languages => {
                "/json/languages?order=stationcount&reverse=true&hidebroken=true&limit=200"
            }
        };
        let this = self.clone();
        Box::pin(async move {
            let server = this.server().await?;
            match get_text(&format!("{server}{path}")).await {
                Ok(text) => parse_facets(kind, &text),
                Err(e) => {
                    this.reset();
                    Err(e)
                }
            }
        })
    }

    fn played(&self, uuid: &str) -> Fut<()> {
        let path = format!("/json/url/{}", encode(uuid));
        let this = self.clone();
        Box::pin(async move {
            let server = this.server().await?;
            get_text(&format!("{server}{path}")).await.map(|_| ())
        })
    }

    fn vote(&self, uuid: &str) -> Fut<()> {
        let path = format!("/json/vote/{}", encode(uuid));
        let this = self.clone();
        Box::pin(async move {
            let server = this.server().await?;
            get_text(&format!("{server}{path}")).await.map(|_| ())
        })
    }
}

/// GET a small JSON document through the platform's stack.
async fn get_text(url: &str) -> Result<String, String> {
    let req = Request::get(url)
        .header("Accept", "application/json")
        .header("User-Agent", USER_AGENT)
        .timeout(std::time::Duration::from_secs(30));
    let response = day_part_http::fetch_future(req)
        .await
        .map_err(|e| format!("{e:?}"))?;
    if response.status != 200 {
        return Err(format!("HTTP {}", response.status));
    }
    Ok(response.text().into_owned())
}

/// Percent-encode one path segment or query value.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// --- the fixture index --------------------------------------------------------------------------

/// The same JSON, from files bundled under `resource/assets/<dir>/`: `top.json`,
/// `trending.json`, `country-<CODE>.json`, `tag-<tag>.json`, `language-<name>.json`,
/// `search.json` (every station, filtered by name here), `countries.json`, `tags.json`,
/// `languages.json`. The walkthrough runs on it.
#[derive(Clone)]
pub struct FixtureIndex {
    dir: String,
}

impl FixtureIndex {
    pub fn new(dir: &str) -> FixtureIndex {
        FixtureIndex {
            dir: dir.trim_matches('/').to_string(),
        }
    }

    async fn asset(&self, name: &str) -> Result<String, String> {
        let path = format!("{}/{name}", self.dir);
        #[cfg(target_arch = "wasm32")]
        {
            let url = format!("assets/data/{path}");
            let response = day_part_http::fetch_future(Request::get(url))
                .await
                .map_err(|e| format!("{name}: {e:?}"))?;
            if response.status != 200 {
                return Err(format!("{name}: HTTP {}", response.status));
            }
            Ok(response.text().into_owned())
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let res = day::resource(day::AssetName::dynamic(path.clone()))
                .ok_or_else(|| format!("no bundled asset {path}"))?;
            Ok(String::from_utf8_lossy(res.as_slice()).into_owned())
        }
    }
}

impl RadioIndex for FixtureIndex {
    fn name(&self) -> String {
        format!("asset:{}", self.dir)
    }

    fn stations(&self, browse: &Browse, limit: usize, offset: usize) -> Fut<Vec<Station>> {
        let (file, filter) = match browse {
            Browse::Top => ("top.json".to_string(), None),
            Browse::Trending => ("trending.json".to_string(), None),
            Browse::Country(c) => (format!("country-{}.json", c.to_ascii_uppercase()), None),
            Browse::Tag(t) => (format!("tag-{}.json", slug(t)), None),
            Browse::Language(l) => (format!("language-{}.json", slug(l)), None),
            Browse::Search(q) => ("search.json".to_string(), Some(q.trim().to_lowercase())),
        };
        let this = self.clone();
        Box::pin(async move {
            let text = this.asset(&file).await?;
            let mut rows = parse_stations(&text)?;
            if let Some(q) = filter {
                rows.retain(|s| {
                    s.name.to_lowercase().contains(&q) || s.tags.to_lowercase().contains(&q)
                });
            }
            Ok(rows.into_iter().skip(offset).take(limit).collect())
        })
    }

    fn facets(&self, kind: FacetKind) -> Fut<Vec<Facet>> {
        let file = match kind {
            FacetKind::Countries => "countries.json",
            FacetKind::Tags => "tags.json",
            FacetKind::Languages => "languages.json",
        };
        let this = self.clone();
        Box::pin(async move {
            let text = this.asset(file).await?;
            parse_facets(kind, &text)
        })
    }

    fn played(&self, _uuid: &str) -> Fut<()> {
        Box::pin(async { Ok(()) })
    }

    fn vote(&self, _uuid: &str) -> Fut<()> {
        Box::pin(async { Ok(()) })
    }
}

/// A tag or language as a fixture file name.
fn slug(s: &str) -> String {
    s.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

// --- the app-wide radio -----------------------------------------------------------------------

/// What a cached fetch holds.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Fetched<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Fetched<T> {
    pub fn is_loading(&self) -> bool {
        matches!(self, Fetched::Loading)
    }
}

/// The app's radio: the index in use and everything fetched from it so far.
pub struct Radio {
    index: RefCell<Rc<dyn RadioIndex>>,
    /// The name of the index in use, for Settings.
    pub index_name: Signal<String>,
    stations: RefCell<HashMap<Browse, Signal<Fetched<Vec<Station>>>>>,
    facets: RefCell<HashMap<FacetKind, Signal<Fetched<Vec<Facet>>>>>,
}

impl Radio {
    /// The index named by the environment, else Settings, else the public one.
    pub fn new(preferred_server: &str) -> Radio {
        let index: Rc<dyn RadioIndex> = match day::env(INDEX_ENV).filter(|s| !s.is_empty()) {
            Some(base) if base.starts_with("asset:") => {
                Rc::new(FixtureIndex::new(base.trim_start_matches("asset:")))
            }
            Some(base) => Rc::new(RadioBrowser::new(&base)),
            None => Rc::new(RadioBrowser::new(preferred_server)),
        };
        let name = index.name();
        Radio {
            index: RefCell::new(index),
            index_name: Signal::new(name),
            stations: RefCell::new(HashMap::new()),
            facets: RefCell::new(HashMap::new()),
        }
    }

    /// Point at another server (Settings) and forget everything fetched.
    pub fn set_server(&'static self, server: &str) {
        if day::env(INDEX_ENV).is_some_and(|s| !s.is_empty()) {
            return;
        }
        let index: Rc<dyn RadioIndex> = Rc::new(RadioBrowser::new(server));
        self.index_name.set(index.name());
        *self.index.borrow_mut() = index;
        self.stations.borrow_mut().clear();
        self.facets.borrow_mut().clear();
    }

    /// The stations of one browse, fetched on first ask and kept.
    pub fn stations(&'static self, browse: &Browse) -> Signal<Fetched<Vec<Station>>> {
        if let Some(s) = self.stations.borrow().get(browse) {
            return *s;
        }
        let signal = Signal::global(Fetched::Loading);
        self.stations.borrow_mut().insert(browse.clone(), signal);
        let index = self.index.borrow().clone();
        let fut = index.stations(browse, PAGE, 0);
        day::task(async move {
            match fut.await {
                Ok(rows) => signal.set(Fetched::Ready(rows)),
                Err(e) => {
                    error!("radio: {e}");
                    signal.set(Fetched::Failed(e));
                }
            }
        });
        signal
    }

    /// Ask again for a browse that failed (or that the listener wants fresh).
    pub fn refresh(&'static self, browse: &Browse) {
        self.stations.borrow_mut().remove(browse);
        let _ = self.stations(browse);
    }

    pub fn facets(&'static self, kind: FacetKind) -> Signal<Fetched<Vec<Facet>>> {
        if let Some(s) = self.facets.borrow().get(&kind) {
            return *s;
        }
        let signal = Signal::global(Fetched::Loading);
        self.facets.borrow_mut().insert(kind, signal);
        let index = self.index.borrow().clone();
        let fut = index.facets(kind);
        day::task(async move {
            match fut.await {
                Ok(rows) => signal.set(Fetched::Ready(rows)),
                Err(e) => {
                    error!("radio: {e}");
                    signal.set(Fetched::Failed(e));
                }
            }
        });
        signal
    }

    pub fn refresh_facets(&'static self, kind: FacetKind) {
        self.facets.borrow_mut().remove(&kind);
        let _ = self.facets(kind);
    }

    /// The name of a country code, from the countries list when it is loaded.
    pub fn country_name(&self, code: &str) -> Option<String> {
        let facets = self.facets.borrow();
        let signal = facets.get(&FacetKind::Countries)?;
        signal.with_untracked(|f| match f {
            Fetched::Ready(rows) => rows
                .iter()
                .find(|r| r.key.eq_ignore_ascii_case(code))
                .map(|r| r.name.clone()),
            _ => None,
        })
    }

    /// Count a play with the index; failures are nobody's problem.
    pub fn played(&self, uuid: &str) {
        let fut = self.index.borrow().played(uuid);
        day::task(async move {
            if let Err(e) = fut.await {
                debug!("radio click not counted: {e}");
            }
        });
    }

    pub fn vote(&self, uuid: &str, done: Signal<Option<Result<(), String>>>) {
        let fut = self.index.borrow().vote(uuid);
        day::task(async move {
            done.set(Some(fut.await));
        });
    }
}

/// A [`Browse`] as a route segment (https://daybrite.dev/docs/how-to/navigation): `top`,
/// `country-FR`, `tag-jazz`, `language-english`, `search-<text>`.
impl Route for Browse {
    fn key(&self) -> String {
        match self {
            Browse::Top => "top".into(),
            Browse::Trending => "trending".into(),
            Browse::Country(c) => format!("country-{c}"),
            Browse::Tag(t) => format!("tag-{t}"),
            Browse::Language(l) => format!("language-{l}"),
            Browse::Search(q) => format!("search-{q}"),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "top" => return Some(Browse::Top),
            "trending" => return Some(Browse::Trending),
            _ => {}
        }
        if let Some(c) = key.strip_prefix("country-") {
            return Some(Browse::Country(c.to_string()));
        }
        if let Some(t) = key.strip_prefix("tag-") {
            return Some(Browse::Tag(t.to_string()));
        }
        if let Some(l) = key.strip_prefix("language-") {
            return Some(Browse::Language(l.to_string()));
        }
        key.strip_prefix("search-")
            .map(|q| Browse::Search(q.to_string()))
    }
    fn title(&self) -> String {
        crate::pages::browse_title(self)
    }
}

/// The stream a station is played from: the resolved endpoint when the index found one.
pub fn stream_url(s: &Station) -> String {
    if s.url_resolved.trim().is_empty() {
        s.url.trim().to_string()
    } else {
        s.url_resolved.trim().to_string()
    }
}

/// The regional-indicator flag for an ISO country code, or nothing for an odd one.
pub fn flag(code: &str) -> String {
    let code = code.trim().to_ascii_uppercase();
    if code.len() != 2 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
        return String::new();
    }
    code.bytes()
        .map(|b| char::from_u32(0x1F1E6 + (b - b'A') as u32).unwrap_or(' '))
        .collect()
}

/// A station's tags as a short list.
pub fn tag_list(tags: &str) -> Vec<String> {
    tags.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn station_json_parses_and_dedups() {
        let text = r#"[
            {"stationuuid":"a","name":" One ","url":"http://x/1","hls":1,"lastcheckok":1,"votes":5,"countrycode":"fr","codec":"mp3"},
            {"stationuuid":"a","name":"One again","url":"http://x/1b"},
            {"stationuuid":"b","name":"No stream","url":""},
            {"stationuuid":"c","name":"Two","url":"http://x/2","hls":true}
        ]"#;
        let rows = parse_stations(text).expect("parses");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "One");
        assert!(rows[0].hls);
        assert_eq!(rows[0].countrycode, "FR");
        assert_eq!(rows[0].codec, "MP3");
        assert_eq!(rows[0].id, station_id("a"));
        assert!(rows[1].hls);
    }

    #[test]
    fn facets_parse_sorted_and_deduped() {
        let text = r#"[{"name":"France","iso_3166_1":"FR","stationcount":5},
            {"name":"Germany","iso_3166_1":"DE","stationcount":9},{"name":"Francia","iso_3166_1":"fr","stationcount":1},
            {"name":"","iso_3166_1":"XX","stationcount":1}]"#;
        let f = parse_facets(FacetKind::Countries, text).expect("parses");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].key, "DE");
        assert_eq!(f[1].name, "France");
    }

    #[test]
    fn browse_routes_round_trip() {
        for b in [
            Browse::Top,
            Browse::Trending,
            Browse::Country("FR".into()),
            Browse::Tag("classic rock".into()),
            Browse::Language("english".into()),
            Browse::Search("smooth jazz".into()),
        ] {
            assert_eq!(Browse::from_key(&b.key()), Some(b.clone()));
        }
        assert_eq!(
            RadioBrowser::path_for(&Browse::Tag("drum & bass".into()), 10, 0),
            "/json/stations/bytagexact/drum%20%26%20bass?order=votes&reverse=true&hidebroken=true&limit=10&offset=0"
        );
    }

    #[test]
    fn flags_and_tags() {
        assert_eq!(flag("de"), "\u{1F1E9}\u{1F1EA}");
        assert_eq!(flag(""), "");
        assert_eq!(tag_list("jazz, blues ,,"), vec!["jazz", "blues"]);
        assert_eq!(slug("Classic Rock"), "classic-rock");
    }
}
