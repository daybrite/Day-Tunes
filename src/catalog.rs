//! Two databases, one container (https://daybrite.dev/docs/persistence).
//!
//! The station catalog is the Tune Out project's own `stations.sqlite`, downloaded as
//! published (src/sync.rs) and ATTACHed read-only under the alias `catalog`. Its tables are
//! read exactly as the catalog built them — the `Station` model below maps the columns this
//! app shows, keyed by the file's implicit rowid, and searches through the FTS5 index the
//! catalog ships. Nothing here writes to that file; a new download replaces it whole.
//!
//! The listener's own data — favorites, recents, notes, tags — lives in the app's writable
//! store and names stations by the catalog's uuid, a value that survives every catalog
//! rebuild where a rowid would not. Those are `link(…)`s: relations by value, with no foreign
//! key, so a favorite whose station drops out of a later catalog keeps its row and shows an
//! empty link rather than vanishing.

use day::persistence::{DbError, Fetch, Linked, ModelContainer, Pred, Query, Sqlite, Value, rank};
use day::prelude::*;

/// The alias the catalog file is attached under; every external model's table is `catalog.…`.
pub const CATALOG_ALIAS: &str = "catalog";

/// One station of the catalog, as the file spells it. The columns this app does not show
/// (the research prose, the per-locale names) stay in the file and cost nothing here.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "stations", external = "catalog", fts("name"))]
pub struct Station {
    /// The file's implicit rowid — the model's key within ONE download. The FTS index is keyed
    /// by it too. Never stored by the app: user rows name a station by `uuid`.
    #[model(id, column = "rowid")]
    pub id: u64,
    /// The catalog's 16-byte uuid, the identity that survives rebuilds.
    pub uuid: Vec<u8>,
    pub name: String,
    pub url: String,
    /// The direct endpoint behind an aggregator URL, when the catalog resolved one; empty
    /// otherwise. Played in preference (the catalog's own note: no ad-injection hop).
    pub url_resolved: String,
    pub homepage: String,
    pub favicon: String,
    pub country: String,
    pub countrycode: String,
    pub state: String,
    /// Comma-separated language codes as upstream spelled them (`DE,EN`).
    pub languagecodes: String,
    pub votes: i64,
    pub codec: String,
    pub bitrate: i64,
    pub hls: bool,
    #[model(column = "lastcheckok")]
    pub online: bool,
    pub clickcount: i64,
    /// The catalog's editorial score, `-1.0..=1.0`; higher sorts first on the Top list.
    pub curation: f64,
    /// `community`, `public broadcaster`, `commercial`, … — the catalog's research field.
    #[model(column = "r_nature")]
    pub nature: String,
    /// The listener's rows about this station, by value.
    #[model(link(target = Favorite, local = "uuid", remote = "station"))]
    pub favorites: Linked<Favorite>,
    #[model(link(target = Tagging, local = "uuid", remote = "station"))]
    pub taggings: Linked<Tagging>,
}

/// A favorited station, by the catalog's uuid; `added` orders the list.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "favorites")]
pub struct Favorite {
    /// `station_id(uuid)`: one favorite per station, addressable without a lookup.
    #[model(id)]
    pub id: u64,
    #[model(index)]
    pub station: Vec<u8>,
    #[model(index)]
    pub added: i64,
    #[model(link(target = Station, local = "station", remote = "uuid"))]
    pub link: Linked<Station>,
    #[model(link(target = Tagging, local = "station", remote = "station"))]
    pub taggings: Linked<Tagging>,
}

/// A played station, by uuid; `played` is the last time.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "recents")]
pub struct Recent {
    #[model(id)]
    pub id: u64,
    #[model(index)]
    pub station: Vec<u8>,
    #[model(index)]
    pub played: i64,
    #[model(link(target = Station, local = "station", remote = "uuid"))]
    pub link: Linked<Station>,
    #[model(link(target = Tagging, local = "station", remote = "station"))]
    pub taggings: Linked<Tagging>,
}

/// The listener's free-text note on a station.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "notes")]
pub struct Note {
    #[model(id)]
    pub id: u64,
    #[model(index)]
    pub station: Vec<u8>,
    pub text: String,
    #[model(link(target = Station, local = "station", remote = "uuid"))]
    pub link: Linked<Station>,
}

/// One of the listener's own tags on a station: `station` × `tag`, keyed by both.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "taggings", index("station", "tag"))]
pub struct Tagging {
    #[model(id)]
    pub id: u64,
    #[model(index)]
    pub station: Vec<u8>,
    #[model(index)]
    pub tag: String,
    #[model(link(target = Station, local = "station", remote = "uuid"))]
    pub link: Linked<Station>,
}

/// How many recently played stations the library keeps.
const RECENTS_CAP: usize = 50;

/// One entry of a Browse list: a country, genre, or language and its station count.
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    /// The key a scope is built from: a country code, a tag slug, a language slug.
    pub key: String,
    /// What the row shows.
    pub name: String,
    pub stations: i64,
}

/// One playable variant of a station, from the catalog's `station_streams`.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamRow {
    pub url: String,
    pub url_resolved: String,
    pub codec: String,
    pub bitrate: i64,
    pub hls: bool,
    pub label: String,
}

/// The one container: the listener's store, with the catalog attached when there is one.
pub struct Catalog {
    pub container: ModelContainer,
    /// A standing query over the favorites. The container holds live queries weakly, so a
    /// query made inside a reactive closure dies with that run; this one lives as long as the
    /// catalog, and every heart reads through it.
    favorites: Query<Favorite>,
    /// Whether a catalog file is attached; the Browse and Search panes wait on it.
    pub attached: Signal<bool>,
    /// Bumped on every attach: a new file re-keys every station, and a page open on the old
    /// one closes (`window_shell`).
    pub generation: Signal<u64>,
    pub station_count: Signal<u64>,
    /// The Browse lists, recomputed on every attach — the catalog is static between them.
    pub countries: Signal<Vec<Facet>>,
    pub tags: Signal<Vec<Facet>>,
    pub languages: Signal<Vec<Facet>>,
}

/// FNV-1a over a station's uuid bytes, folded to a positive 63-bit integer: SQLite's INTEGER
/// is signed, and a `STRICT` table refuses anything past `i64::MAX`.
pub fn station_id(uuid: &[u8]) -> u64 {
    fnv(uuid, 0xcbf2_9ce4_8422_2325)
}

/// The id of one tagging: the station and the tag together.
fn tagging_id(uuid: &[u8], tag: &str) -> u64 {
    fnv(
        tag.as_bytes(),
        fnv(uuid, 0xcbf2_9ce4_8422_2325) ^ 0x9e37_79b9,
    )
}

fn fnv(bytes: &[u8], seed: u64) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h & 0x7fff_ffff_ffff_ffff
}

/// A time for ordering favorites and recents: seconds since the epoch, or a counter on the
/// web, where the clock is a browser API rather than std.
pub fn now_secs() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
    // The web has no std clock. A persisted counter keeps the ORDER, which is all the library
    // asks of these times.
    #[cfg(target_arch = "wasm32")]
    {
        let next = day::prefs::get("app.clock")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
            + 1;
        day::prefs::set("app.clock", &next.to_string());
        next
    }
}

/// `uuid BLOB` → the catalog's dashed text form, for logs and the station page.
pub fn uuid_text(uuid: &[u8]) -> String {
    let hex: String = uuid.iter().map(|b| format!("{b:02x}")).collect();
    if hex.len() != 32 {
        return hex;
    }
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

impl Catalog {
    /// Open the listener's store. The catalog attaches separately, once its file is on disk
    /// (`attach`), which on a first launch is after the download.
    pub fn open() -> Result<Catalog, DbError> {
        #[cfg(not(target_arch = "wasm32"))]
        let driver = Sqlite::app_data("tunes.db")?;
        #[cfg(target_arch = "wasm32")]
        let driver = Sqlite::at("tunes.db");
        Self::open_with(driver)
    }

    /// The fallback for a machine whose data directory cannot be written: everything works,
    /// nothing outlives the process.
    pub fn in_memory() -> Catalog {
        match Self::open_with(Sqlite::memory()) {
            Ok(c) => c,
            // An in-memory SQLite that cannot open is a broken build, not a runtime condition.
            Err(e) => panic!("the in-memory store could not open: {e}"),
        }
    }

    fn open_with(driver: Sqlite) -> Result<Catalog, DbError> {
        let container = ModelContainer::open(driver, schema![Favorite, Recent, Note, Tagging])?;
        let favorites = container
            .query::<Favorite>()
            .sort(Favorite::added().desc())
            .live();
        Ok(Catalog {
            container,
            favorites,
            attached: Signal::new(false),
            generation: Signal::new(0),
            station_count: Signal::new(0),
            countries: Signal::new(Vec::new()),
            tags: Signal::new(Vec::new()),
            languages: Signal::new(Vec::new()),
        })
    }

    /// Attach (or swap in) the catalog file at `path` and recompute the Browse lists. Every
    /// live query over the catalog re-runs, so a list on screen shows the new rows.
    pub fn attach(&self, path: &std::path::Path) -> Result<(), DbError> {
        let path = path.to_string_lossy().into_owned();
        self.container
            .attach_database(CATALOG_ALIAS, &path, schema![Station])?;
        self.refresh_facets()?;
        self.generation.update(|g| *g += 1);
        self.attached.set(true);
        info!(
            "catalog attached: {} stations",
            self.station_count.get_untracked()
        );
        Ok(())
    }

    /// The Browse lists and the count, straight from the file: three GROUP BYs over indexed
    /// columns, run once per attach. A `WITHOUT ROWID` junction table is not a model, so
    /// these read through the connection.
    fn refresh_facets(&self) -> Result<(), DbError> {
        let (count, countries, tags, languages) =
            self.container
                .with_connection(|conn| -> Result<_, DbError> {
                    let mut count = 0i64;
                    conn.query(
                        "SELECT COUNT(*) FROM catalog.stations WHERE url IS NOT NULL AND url <> ''",
                        &[],
                        &mut |row| count = row.get(0).as_int().unwrap_or(0),
                    )?;
                    let mut countries = Vec::new();
                    conn.query(
                        "SELECT countrycode, MIN(country), COUNT(*) AS n FROM catalog.stations \
                         WHERE countrycode <> '' AND lastcheckok = 1 GROUP BY countrycode \
                         ORDER BY n DESC",
                        &[],
                        &mut |row| {
                            countries.push(Facet {
                                key: text(&row.get(0)),
                                name: text(&row.get(1)),
                                stations: row.get(2).as_int().unwrap_or(0),
                            })
                        },
                    )?;
                    let mut tags = Vec::new();
                    conn.query(
                        "SELECT t.slug, COUNT(*) AS n FROM catalog.tags t \
                         JOIN catalog.station_tags st ON st.tag_id = t.id \
                         GROUP BY t.id ORDER BY n DESC",
                        &[],
                        &mut |row| {
                            let slug = text(&row.get(0));
                            tags.push(Facet {
                                name: slug.clone(),
                                key: slug,
                                stations: row.get(1).as_int().unwrap_or(0),
                            })
                        },
                    )?;
                    let mut languages = Vec::new();
                    conn.query(
                        "SELECT l.slug, COUNT(*) AS n FROM catalog.languages l \
                         JOIN catalog.station_languages sl ON sl.language_id = l.id \
                         GROUP BY l.id ORDER BY n DESC",
                        &[],
                        &mut |row| {
                            let slug = text(&row.get(0));
                            languages.push(Facet {
                                name: slug.clone(),
                                key: slug,
                                stations: row.get(1).as_int().unwrap_or(0),
                            })
                        },
                    )?;
                    Ok((count, countries, tags, languages))
                })?;
        self.station_count.set(count.max(0) as u64);
        self.countries.set(countries);
        self.tags.set(tags);
        self.languages.set(languages);
        Ok(())
    }

    // --- reads ---------------------------------------------------------------------------

    /// The whole row, faulted in if needed. `None` only for an id the catalog never held.
    pub fn station(&self, id: u64) -> Option<Station> {
        self.container.get::<Station>(id)?;
        self.container
            .cache::<Station>()
            .with_untracked(|k| k.get(id).cloned())
    }

    /// The station a user row names, if this catalog still carries it.
    pub fn station_by_uuid(&self, uuid: &[u8]) -> Option<Station> {
        if uuid.is_empty() || !self.attached.get_untracked() {
            return None;
        }
        let id = self
            .container
            .query::<Station>()
            .filter(Station::uuid().eq(uuid.to_vec()))
            .limit(1)
            .live()
            .ids_untracked()
            .first()?
            .handle();
        self.station(id)
    }

    /// The uuids of `ids`, in the same order — a list's worth of stations for the player's
    /// queue, in one query rather than a fault per row.
    pub fn uuids_for(&self, ids: &[u64]) -> Vec<Vec<u8>> {
        if ids.is_empty() || !self.attached.get_untracked() {
            return Vec::new();
        }
        let mut found: std::collections::HashMap<u64, Vec<u8>> = std::collections::HashMap::new();
        for chunk in ids.chunks(500) {
            let marks = vec!["?"; chunk.len()].join(",");
            let params: Vec<Value> = chunk.iter().map(|id| Value::Int(*id as i64)).collect();
            let _ = self.container.with_connection(|conn| {
                conn.query(
                    &format!("SELECT rowid, uuid FROM catalog.stations WHERE rowid IN ({marks})"),
                    &params,
                    &mut |row| {
                        let id = row.get(0).as_int().unwrap_or(0) as u64;
                        if let Ok(uuid) = row.get(1).as_blob() {
                            found.insert(id, uuid.to_vec());
                        }
                    },
                )
            });
        }
        ids.iter().filter_map(|id| found.get(id).cloned()).collect()
    }

    /// Every stream variant a station publishes, in the catalog's preference order; empty
    /// for a station with just the one (its `url` is the stream then).
    pub fn streams(&self, uuid: &[u8]) -> Vec<StreamRow> {
        if !self.attached.get_untracked() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let _ = self.container.with_connection(|conn| {
            conn.query(
                "SELECT url, url_resolved, codec, bitrate, hls, label FROM catalog.station_streams \
                 WHERE station_uuid = ? ORDER BY sort_order",
                &[Value::Blob(uuid.to_vec())],
                &mut |row| {
                    out.push(StreamRow {
                        url: text(&row.get(0)),
                        url_resolved: text(&row.get(1)),
                        codec: text(&row.get(2)).to_uppercase(),
                        bitrate: row.get(3).as_int().unwrap_or(0),
                        hls: row.get(4).as_int().unwrap_or(0) != 0,
                        label: text(&row.get(5)),
                    })
                },
            )
        });
        out
    }

    /// The catalog's genre slugs for a station, from its junction table.
    pub fn station_tags(&self, uuid: &[u8]) -> Vec<String> {
        if !self.attached.get_untracked() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let _ = self.container.with_connection(|conn| {
            conn.query(
                "SELECT t.slug FROM catalog.station_tags st JOIN catalog.tags t ON t.id = st.tag_id \
                 WHERE st.station_uuid = ? ORDER BY t.slug",
                &[Value::Blob(uuid.to_vec())],
                &mut |row| out.push(text(&row.get(0))),
            )
        });
        out
    }

    /// A live query over stations that re-fetches whenever `scope` changes.
    pub fn stations_for(&self, scope: impl Fn() -> Scope + 'static) -> Query<Station> {
        self.container.query_fn::<Station>(move || scope().fetch())
    }

    // --- the library ---------------------------------------------------------------------

    /// The favorites carrying the listener's tag `tag` (every favorite when `None`), live.
    pub fn favorites_tagged(&self, tag: impl Fn() -> Option<String> + 'static) -> Query<Favorite> {
        self.container.query_fn::<Favorite>(move || {
            let mut f = Fetch::new().sort(Favorite::added().desc());
            if let Some(tag) = tag() {
                f = f.filter(Favorite::taggings().any(Tagging::tag().eq(tag)));
            }
            f
        })
    }

    pub fn recents(&self) -> Query<Recent> {
        self.container
            .query::<Recent>()
            .sort(Recent::played().desc())
            .live()
    }

    /// The recents carrying the listener's tag `tag` (every recent when `None`), live.
    pub fn recents_tagged(&self, tag: impl Fn() -> Option<String> + 'static) -> Query<Recent> {
        self.container.query_fn::<Recent>(move || {
            let mut f = Fetch::new().sort(Recent::played().desc());
            if let Some(tag) = tag() {
                f = f.filter(Recent::taggings().any(Tagging::tag().eq(tag)));
            }
            f
        })
    }

    /// The uuid a favorite row names.
    pub fn favorite_uuid(&self, id: u64) -> Option<Vec<u8>> {
        self.container
            .get::<Favorite>(id)
            .map(|f| f.station().read())
    }

    /// The uuid a recent row names.
    pub fn recent_uuid(&self, id: u64) -> Option<Vec<u8>> {
        self.container.get::<Recent>(id).map(|r| r.station().read())
    }

    /// Whether a station is favorited — a tracked read, so a heart re-draws when it flips.
    pub fn is_favorite(&self, uuid: &[u8]) -> bool {
        self.favorites.contains(station_id(uuid))
    }

    pub fn toggle_favorite(&self, uuid: &[u8]) {
        let id = station_id(uuid);
        if self.container.get::<Favorite>(id).is_some() {
            if let Err(e) = self.container.delete::<Favorite>(id) {
                error!("could not remove the favorite: {e}");
            }
        } else {
            self.container.insert(Favorite {
                id,
                station: uuid.to_vec(),
                added: now_secs(),
                link: Linked::default(),
                taggings: Linked::default(),
            });
        }
    }

    /// Record a play: the newest recent, and the list trimmed to its cap.
    pub fn touch_recent(&self, uuid: &[u8]) {
        let id = station_id(uuid);
        if let Some(r) = self.container.get::<Recent>(id) {
            r.played().write(now_secs());
        } else {
            self.container.insert(Recent {
                id,
                station: uuid.to_vec(),
                played: now_secs(),
                link: Linked::default(),
                taggings: Linked::default(),
            });
        }
        let ids = self.recents().ids_untracked();
        for old in ids.iter().skip(RECENTS_CAP) {
            if let Err(e) = self.container.delete::<Recent>(old.handle()) {
                error!("could not trim the recents: {e}");
            }
        }
    }

    /// The note on a station — created empty on first read, so the editor binds to a row.
    pub fn note(&self, uuid: &[u8]) -> Elem<Note> {
        let id = station_id(uuid);
        if let Some(n) = self.container.get::<Note>(id) {
            return n;
        }
        self.container.insert(Note {
            id,
            station: uuid.to_vec(),
            text: String::new(),
            link: Linked::default(),
        });
        self.container.cache::<Note>().elem(id)
    }

    /// The listener's tags on a station, live: a query the page keeps.
    pub fn taggings(&self, uuid: &[u8]) -> Query<Tagging> {
        self.container
            .query::<Tagging>()
            .filter(Tagging::station().eq(uuid.to_vec()))
            .sort(Tagging::tag().asc())
            .live()
    }

    /// Every tagging the listener has made, live — the Library filter reads the distinct
    /// names off it.
    pub fn all_taggings(&self) -> Query<Tagging> {
        self.container
            .query::<Tagging>()
            .sort(Tagging::tag().asc())
            .live()
    }

    /// The tag a tagging row carries.
    pub fn tagging_name(&self, id: u64) -> Option<String> {
        self.container.get::<Tagging>(id).map(|t| t.tag().read())
    }

    /// Add `tag` to a station; a tag is a trimmed, lower-cased word or two, once per station.
    pub fn add_tag(&self, uuid: &[u8], tag: &str) {
        let tag = tag.trim().to_lowercase();
        if tag.is_empty() {
            return;
        }
        let id = tagging_id(uuid, &tag);
        if self.container.get::<Tagging>(id).is_some() {
            return;
        }
        self.container.insert(Tagging {
            id,
            station: uuid.to_vec(),
            tag,
            link: Linked::default(),
        });
    }

    pub fn remove_tag(&self, uuid: &[u8], tag: &str) {
        let id = tagging_id(uuid, tag);
        if let Err(e) = self.container.delete::<Tagging>(id) {
            error!("could not remove the tag: {e}");
        }
    }
}

/// A row cell as text; `NULL` and non-text read empty.
fn text(v: &Value) -> String {
    v.as_text().unwrap_or_default().to_string()
}

/// One cut of the catalog: what a station list shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Scope {
    /// The catalog's best, by editorial score then votes.
    Top,
    Country(String),
    Tag(String),
    Language(String),
    /// Free text over names and genres; empty text is `Top`.
    Search(String),
}

impl Scope {
    /// A page of results, not a whole table: 300 rows is more than any list is scrolled
    /// through, and it bounds the id set a live query keeps.
    const PAGE: usize = 300;

    pub fn fetch(&self) -> Fetch {
        let playable = Station::online().eq(true) & Station::url().ne(String::new());
        match self {
            Scope::Top => Fetch::new()
                .filter(playable)
                .sort(Station::curation().desc())
                .sort(Station::votes().desc())
                .limit(Self::PAGE),
            Scope::Country(code) => Fetch::new()
                .filter(playable & Station::countrycode().eq(code.clone()))
                .sort(Station::votes().desc())
                .limit(Self::PAGE),
            // The junction tables are `WITHOUT ROWID` and so not models; a raw membership
            // test over them is exact, and the catalog never changes underneath a query.
            Scope::Tag(slug) => Fetch::new()
                .filter(
                    playable
                        & Pred::Raw(
                            "catalog.stations.uuid IN (SELECT st.station_uuid FROM \
                             catalog.station_tags st JOIN catalog.tags t ON t.id = st.tag_id \
                             WHERE t.slug = ?)"
                                .into(),
                            vec![Value::Text(slug.clone())],
                        ),
                )
                .sort(Station::votes().desc())
                .limit(Self::PAGE),
            Scope::Language(slug) => Fetch::new()
                .filter(
                    playable
                        & Pred::Raw(
                            "catalog.stations.uuid IN (SELECT sl.station_uuid FROM \
                             catalog.station_languages sl JOIN catalog.languages l \
                             ON l.id = sl.language_id WHERE l.slug = ?)"
                                .into(),
                            vec![Value::Text(slug.clone())],
                        ),
                )
                .sort(Station::votes().desc())
                .limit(Self::PAGE),
            Scope::Search(text) => match fts_expression(text) {
                None => Scope::Top.fetch(),
                Some(expr) => Fetch::new()
                    .filter(Station::fts().matches(expr) & Station::url().ne(String::new()))
                    .sort(rank())
                    .sort(Station::votes().desc())
                    .limit(Self::PAGE),
            },
        }
    }
}

/// Free text → an FTS5 expression over the catalog index's name and genre columns: every
/// word quoted and prefix-matched, so typing feels live and a stray quote or dash is
/// searched for rather than parsed as syntax. `None` for text with no words in it.
pub fn fts_expression(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(format!("{{name tags_text}} : {}", words.join(" ")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_positive() {
        let u = b"9617a958060111e8ae97";
        assert_eq!(station_id(u), station_id(u));
        assert!(station_id(u) <= i64::MAX as u64);
        assert_ne!(station_id(u), station_id(b"962cc6df060111e8ae97"));
        assert_ne!(tagging_id(u, "jazz"), tagging_id(u, "rock"));
    }

    #[test]
    fn search_expressions_prefix_every_word_and_survive_quotes() {
        assert_eq!(
            fts_expression("smooth jazz").as_deref(),
            Some("{name tags_text} : \"smooth\"* \"jazz\"*")
        );
        assert_eq!(
            fts_expression("rock \"n\" roll").as_deref(),
            Some("{name tags_text} : \"rock\"* \"n\"* \"roll\"*")
        );
        assert_eq!(fts_expression("  -- "), None);
    }

    #[test]
    fn uuids_print_dashed() {
        let bytes: Vec<u8> = (0..16).collect();
        assert_eq!(uuid_text(&bytes), "00010203-0405-0607-0809-0a0b0c0d0e0f");
    }

    #[test]
    fn flags_come_from_country_codes() {
        assert_eq!(flag("de"), "\u{1F1E9}\u{1F1EA}");
        assert_eq!(flag(""), "");
        assert_eq!(flag("XKX"), "");
    }
}
