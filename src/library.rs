//! The listener's store (https://daybrite.dev/docs/reference/persistence): every row the app
//! keeps lives in one SQLite file. Music tracks and the folders they were scanned from,
//! playlists and their entries, radio stations the listener kept or played, podcast
//! subscriptions and their episodes, the play history, and the scrobbles waiting to be sent.
//!
//! Everything is keyed by a stable 63-bit FNV hash of the row's natural identity (a track's
//! path, a station's uuid, an episode's feed + guid), so a re-scan or a re-fetch finds the
//! row it already has rather than making a second one.
//!
//! The scanner walks library folders on a worker thread, reads each file's tags through
//! `day_piece_media::tags`, and hands the rows back to the UI thread in batches. Album and
//! artist lists are GROUP BY queries over the tracks table, recomputed when the library's
//! `generation` ticks.

use day::prelude::*;
use day_persistence::prelude::*;
use day_persistence::{DbError, Fetch, ModelContainer, Query, Sqlite, Value};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;

// --- models ---------------------------------------------------------------------------------

/// One audio file in the library.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "tracks", fts("title", "artist", "album"))]
pub struct Track {
    /// `hash(path)`.
    #[model(id)]
    pub id: u64,
    /// The absolute path of the file (a scanned folder's file, or one imported under the
    /// app's data directory).
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The album's artist: the tag when the file carries one, else the track artist.
    pub album_artist: String,
    pub genre: String,
    pub year: i64,
    pub track_no: i64,
    pub disc_no: i64,
    /// Seconds; `0.0` when the file does not say.
    pub duration: f64,
    /// `MP3`, `FLAC`, …
    pub format: String,
    pub bitrate: i64,
    /// The artwork cache key (`art/<hash>.<ext>` under the app's files), empty for none.
    pub artwork: String,
    pub added: i64,
    pub play_count: i64,
    pub last_played: i64,
    /// The folder this file was scanned from; `0` for a file imported on its own.
    #[model(index)]
    pub folder: u64,
    /// `hash(lower(album_artist) + lower(album))`: what the Albums list groups by.
    #[model(index)]
    pub album_key: u64,
    /// `hash(lower(artist))`.
    #[model(index)]
    pub artist_key: u64,
}

/// A folder the library watches.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "folders")]
pub struct Folder {
    /// `hash(path)`.
    #[model(id)]
    pub id: u64,
    pub path: String,
    pub added: i64,
    pub scanned: i64,
    pub tracks: i64,
}

/// A playlist the listener made.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "playlists")]
pub struct Playlist {
    #[model(id)]
    pub id: u64,
    pub name: String,
    pub created: i64,
    /// Sidebar order.
    pub position: f64,
}

/// One track in one playlist, at a fractional position.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "playlist_entries", index("playlist", "position"))]
pub struct PlaylistEntry {
    #[model(id)]
    pub id: u64,
    #[model(index)]
    pub playlist: u64,
    pub track: u64,
    pub position: f64,
}

/// A radio station the listener kept or played: the index's row, copied, so favorites work
/// with no network and survive the index changing its mind.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "stations", fts("name", "tags"))]
pub struct Station {
    /// `hash(uuid)`.
    #[model(id)]
    pub id: u64,
    pub uuid: String,
    pub name: String,
    pub url: String,
    pub url_resolved: String,
    pub homepage: String,
    pub favicon: String,
    /// Comma-separated, as the index spells them.
    pub tags: String,
    pub country: String,
    pub countrycode: String,
    pub state: String,
    pub language: String,
    pub codec: String,
    pub bitrate: i64,
    pub hls: bool,
    pub votes: i64,
    #[model(index)]
    pub favorite: bool,
    pub added: i64,
    #[model(index)]
    pub last_played: i64,
    pub play_count: i64,
}

/// A podcast subscription.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "podcasts", fts("title", "author"))]
pub struct Podcast {
    /// `hash(feed)`.
    #[model(id)]
    pub id: u64,
    pub feed: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub image: String,
    pub link: String,
    pub added: i64,
    pub refreshed: i64,
    /// Conditional-request validators from the last fetch.
    pub etag: String,
    pub modified: String,
    pub episodes: i64,
}

/// One episode of a subscription.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "episodes", fts("title"), index("podcast", "published"))]
pub struct Episode {
    /// `hash(feed + guid)`.
    #[model(id)]
    pub id: u64,
    #[model(index)]
    pub podcast: u64,
    pub guid: String,
    pub title: String,
    pub summary: String,
    /// The enclosure.
    pub url: String,
    pub mime: String,
    pub bytes: i64,
    pub duration: f64,
    #[model(index)]
    pub published: i64,
    pub image: String,
    /// Where playback left off, in seconds.
    pub position: f64,
    #[model(index)]
    pub played: bool,
    /// A downloaded copy, under the app's files; empty while streaming.
    pub local: String,
    /// `DOWNLOAD_*`.
    pub download: i64,
    pub added: i64,
}

pub const DOWNLOAD_NONE: i64 = 0;
pub const DOWNLOAD_RUNNING: i64 = 1;
pub const DOWNLOAD_DONE: i64 = 2;
pub const DOWNLOAD_FAILED: i64 = 3;

/// Something that was played: the kind and id of the item, with the words to show if the item
/// is gone.
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "history")]
pub struct History {
    /// `hash(kind, item)`: one row per item, moved to the top on every play.
    #[model(id)]
    pub id: u64,
    pub kind: i64,
    pub item: u64,
    pub title: String,
    pub subtitle: String,
    #[model(index)]
    pub played: i64,
}

pub const KIND_TRACK: i64 = 1;
pub const KIND_STATION: i64 = 2;
pub const KIND_EPISODE: i64 = 3;

/// A listen waiting to be scrobbled (src/scrobble.rs).
#[derive(Model, Clone, Default, PartialEq, Debug)]
#[model(table = "scrobbles")]
pub struct Scrobble {
    #[model(id)]
    pub id: u64,
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration: f64,
    #[model(index)]
    pub started: i64,
    pub attempts: i64,
}

// --- ids and hashing -----------------------------------------------------------------------

/// FNV-1a over `bytes`, folded to a positive 63-bit integer: SQLite's INTEGER is signed, and
/// a `STRICT` table refuses anything past `i64::MAX`.
pub fn hash(bytes: &[u8]) -> u64 {
    fnv(bytes, 0xcbf2_9ce4_8422_2325)
}

/// A hash over several parts, separated so `("a", "bc")` and `("ab", "c")` differ.
pub fn hash_parts(parts: &[&str]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for p in parts {
        h = fnv(p.as_bytes(), h);
        h = fnv(&[0], h);
    }
    h & 0x7fff_ffff_ffff_ffff
}

fn fnv(bytes: &[u8], seed: u64) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h & 0x7fff_ffff_ffff_ffff
}

pub fn track_id(path: &str) -> u64 {
    hash(path.as_bytes())
}

pub fn station_id(uuid: &str) -> u64 {
    hash(uuid.as_bytes())
}

pub fn podcast_id(feed: &str) -> u64 {
    hash(feed.trim().as_bytes())
}

pub fn episode_id(feed: &str, guid: &str) -> u64 {
    hash_parts(&[feed.trim(), guid.trim()])
}

pub fn album_key(album_artist: &str, album: &str) -> u64 {
    hash_parts(&[&album_artist.to_lowercase(), &album.to_lowercase()])
}

pub fn artist_key(artist: &str) -> u64 {
    hash(artist.to_lowercase().as_bytes())
}

fn history_id(kind: i64, item: u64) -> u64 {
    hash_parts(&[&kind.to_string(), &item.to_string()])
}

/// Seconds since the epoch, or a persisted counter on the web, where the clock is a browser
/// API rather than std; the order is all the lists ask of these times.
pub fn now_secs() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
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

// --- grouped views ----------------------------------------------------------------------------

/// One row of the Albums, Artists, or Genres list: a group of tracks and a few facts about it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Group {
    /// `album_key` / `artist_key` / the genre text.
    pub key: u64,
    pub name: String,
    /// The album's artist, an artist's album count, a genre's track count.
    pub subtitle: String,
    pub tracks: i64,
    pub year: i64,
    /// An artwork key from one of the group's tracks.
    pub artwork: String,
    pub duration: f64,
}

/// How many recently played rows the history keeps.
const HISTORY_CAP: usize = 200;

/// The one container, and the live summaries the sidebar and lists read.
pub struct Library {
    pub container: ModelContainer,
    /// Bumped after every scan, import or removal; the grouped views recompute on it.
    pub generation: Signal<u64>,
    pub track_count: Signal<i64>,
    pub albums: Signal<Vec<Group>>,
    pub artists: Signal<Vec<Group>>,
    pub genres: Signal<Vec<Group>>,
    /// What the scanner is doing (`None` when idle).
    pub scanning: Signal<Option<ScanProgress>>,
    /// A standing query over the playlists, sidebar order.
    pub playlists: Query<Playlist>,
    /// A standing query over favorite stations: the hearts read through it.
    favorites: Query<Station>,
    /// Where imported media, artwork and downloads live (`None` on the web).
    pub files: Option<PathBuf>,
}

/// Where a scan is.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScanProgress {
    pub folder: String,
    pub seen: u64,
    pub added: u64,
}

impl Library {
    pub fn open() -> Result<Library, DbError> {
        #[cfg(not(target_arch = "wasm32"))]
        let driver = Sqlite::app_data("tunes.db")?;
        #[cfg(target_arch = "wasm32")]
        let driver = Sqlite::at("tunes.db");
        Self::open_with(driver)
    }

    /// The fallback for a machine whose data directory cannot be written: everything works,
    /// nothing outlives the process.
    pub fn in_memory() -> Library {
        match Self::open_with(Sqlite::memory()) {
            Ok(c) => c,
            // An in-memory SQLite that cannot open is a broken build, not a runtime condition.
            Err(e) => panic!("the in-memory store could not open: {e}"),
        }
    }

    fn open_with(driver: Sqlite) -> Result<Library, DbError> {
        let container = ModelContainer::open(
            driver,
            schema![
                Track,
                Folder,
                Playlist,
                PlaylistEntry,
                Station,
                Podcast,
                Episode,
                History,
                Scrobble
            ],
        )?;
        let playlists = container
            .query::<Playlist>()
            .sort(Playlist::position().asc())
            .sort(Playlist::name().asc())
            .live();
        let favorites = container
            .query::<Station>()
            .filter(Station::favorite().eq(true))
            .live();
        let lib = Library {
            container,
            generation: Signal::new(0),
            track_count: Signal::new(0),
            albums: Signal::new(Vec::new()),
            artists: Signal::new(Vec::new()),
            genres: Signal::new(Vec::new()),
            scanning: Signal::new(None),
            playlists,
            favorites,
            files: files_root(),
        };
        lib.refresh_groups();
        Ok(lib)
    }

    /// Recompute the grouped views and the count: three GROUP BYs, run after every change to
    /// the tracks table.
    pub fn refresh_groups(&self) {
        let result = self
            .container
            .with_connection(|conn| -> Result<_, DbError> {
                let mut count = 0i64;
                conn.query("SELECT COUNT(*) FROM tracks", &[], &mut |row| {
                    count = row.get(0).as_int().unwrap_or(0)
                })?;
                let mut albums = Vec::new();
                conn.query(
                    "SELECT album_key, MIN(album), MIN(album_artist), COUNT(*), MIN(year), \
                     MAX(artwork), SUM(duration) FROM tracks GROUP BY album_key \
                     ORDER BY LOWER(MIN(album_artist)), MIN(year), LOWER(MIN(album))",
                    &[],
                    &mut |row| {
                        albums.push(Group {
                            key: row.get(0).as_int().unwrap_or(0) as u64,
                            name: text(&row.get(1)),
                            subtitle: text(&row.get(2)),
                            tracks: row.get(3).as_int().unwrap_or(0),
                            year: row.get(4).as_int().unwrap_or(0),
                            artwork: text(&row.get(5)),
                            duration: row.get(6).as_real().unwrap_or(0.0),
                        })
                    },
                )?;
                let mut artists = Vec::new();
                conn.query(
                    "SELECT artist_key, MIN(artist), COUNT(*), COUNT(DISTINCT album_key), \
                     MAX(artwork), SUM(duration) FROM tracks GROUP BY artist_key \
                     ORDER BY LOWER(MIN(artist))",
                    &[],
                    &mut |row| {
                        artists.push(Group {
                            key: row.get(0).as_int().unwrap_or(0) as u64,
                            name: text(&row.get(1)),
                            subtitle: row.get(3).as_int().unwrap_or(0).to_string(),
                            tracks: row.get(2).as_int().unwrap_or(0),
                            year: 0,
                            artwork: text(&row.get(4)),
                            duration: row.get(5).as_real().unwrap_or(0.0),
                        })
                    },
                )?;
                let mut genres = Vec::new();
                conn.query(
                    "SELECT genre, COUNT(*), MAX(artwork), SUM(duration) FROM tracks \
                     WHERE genre <> '' GROUP BY LOWER(genre) ORDER BY LOWER(genre)",
                    &[],
                    &mut |row| {
                        let name = text(&row.get(0));
                        genres.push(Group {
                            key: hash(name.to_lowercase().as_bytes()),
                            name,
                            subtitle: String::new(),
                            tracks: row.get(1).as_int().unwrap_or(0),
                            year: 0,
                            artwork: text(&row.get(2)),
                            duration: row.get(3).as_real().unwrap_or(0.0),
                        })
                    },
                )?;
                Ok((count, albums, artists, genres))
            });
        match result {
            Ok((count, albums, artists, genres)) => {
                self.track_count.set(count);
                self.albums.set(albums);
                self.artists.set(artists);
                self.genres.set(genres);
                self.generation.update(|g| *g += 1);
            }
            Err(e) => error!("library summary failed: {e}"),
        }
    }

    // --- tracks -----------------------------------------------------------------------------

    pub fn track(&self, id: u64) -> Option<Track> {
        self.container.get::<Track>(id)?;
        self.container
            .cache::<Track>()
            .with_untracked(|k| k.get(id).cloned())
    }

    /// Every track, sorted by the listener's choice; `search` narrows by title, artist, or
    /// album as typed.
    pub fn tracks(
        &self,
        sort: impl Fn() -> TrackSort + 'static,
        search: impl Fn() -> String + 'static,
    ) -> Query<Track> {
        self.container.query_fn::<Track>(move || {
            let mut f = Fetch::new();
            let q = search();
            if let Some(expr) = fts_expression(&q) {
                f = f.filter(Track::fts().matches(expr));
            }
            match sort() {
                TrackSort::Title => f.sort(Track::title().asc()),
                TrackSort::Artist => f
                    .sort(Track::artist().asc())
                    .sort(Track::album().asc())
                    .sort(Track::disc_no().asc())
                    .sort(Track::track_no().asc()),
                TrackSort::Album => f
                    .sort(Track::album().asc())
                    .sort(Track::disc_no().asc())
                    .sort(Track::track_no().asc()),
                TrackSort::Added => f.sort(Track::added().desc()),
                TrackSort::Played => f.sort(Track::last_played().desc()),
            }
        })
    }

    /// The tracks of one album, in disc and track order.
    pub fn album_tracks(&self, key: u64) -> Query<Track> {
        self.container
            .query::<Track>()
            .filter(Track::album_key().eq(key))
            .sort(Track::disc_no().asc())
            .sort(Track::track_no().asc())
            .sort(Track::title().asc())
            .live()
    }

    /// The tracks of one artist, grouped by album.
    pub fn artist_tracks(&self, key: u64) -> Query<Track> {
        self.container
            .query::<Track>()
            .filter(Track::artist_key().eq(key))
            .sort(Track::year().asc())
            .sort(Track::album().asc())
            .sort(Track::disc_no().asc())
            .sort(Track::track_no().asc())
            .live()
    }

    /// The albums an artist appears on.
    pub fn artist_albums(&self, key: u64) -> Vec<Group> {
        let mut out = Vec::new();
        let _ = self.container.with_connection(|conn| {
            conn.query(
                "SELECT album_key, MIN(album), MIN(album_artist), COUNT(*), MIN(year), MAX(artwork), \
                 SUM(duration) FROM tracks WHERE artist_key = ? GROUP BY album_key \
                 ORDER BY MIN(year), LOWER(MIN(album))",
                &[Value::Int(key as i64)],
                &mut |row| {
                    out.push(Group {
                        key: row.get(0).as_int().unwrap_or(0) as u64,
                        name: text(&row.get(1)),
                        subtitle: text(&row.get(2)),
                        tracks: row.get(3).as_int().unwrap_or(0),
                        year: row.get(4).as_int().unwrap_or(0),
                        artwork: text(&row.get(5)),
                        duration: row.get(6).as_real().unwrap_or(0.0),
                    })
                },
            )
        });
        out
    }

    /// The tracks of one genre.
    pub fn genre_tracks(&self, genre: String) -> Query<Track> {
        self.container
            .query::<Track>()
            .filter(Track::genre().eq(genre))
            .sort(Track::artist().asc())
            .sort(Track::album().asc())
            .sort(Track::track_no().asc())
            .live()
    }

    /// The newest files, for Recently Added.
    pub fn recent_tracks(&self) -> Query<Track> {
        self.container
            .query::<Track>()
            .sort(Track::added().desc())
            .limit(200)
            .live()
    }

    /// The album a group key names, from the current summary.
    pub fn album(&self, key: u64) -> Option<Group> {
        self.albums
            .with_untracked(|a| a.iter().find(|g| g.key == key).cloned())
    }

    pub fn artist(&self, key: u64) -> Option<Group> {
        self.artists
            .with_untracked(|a| a.iter().find(|g| g.key == key).cloned())
    }

    pub fn genre(&self, key: u64) -> Option<Group> {
        self.genres
            .with_untracked(|a| a.iter().find(|g| g.key == key).cloned())
    }

    /// The ids a query holds right now, as plain keys.
    pub fn ids_of<M: day_persistence::Model>(q: &Query<M>) -> Vec<u64> {
        q.ids_untracked().iter().map(|id| id.handle()).collect()
    }

    /// Note a play: the counts on the row, and the history.
    pub fn touch_track(&self, id: u64) {
        if let Some(t) = self.container.get::<Track>(id) {
            t.play_count().write(t.play_count().read() + 1);
            t.last_played().write(now_secs());
            let (title, artist) = (t.title().read(), t.artist().read());
            self.touch_history(KIND_TRACK, id, title, artist);
        }
    }

    /// Remove a folder and every track scanned from it.
    pub fn remove_folder(&self, id: u64) {
        let ids = Self::ids_of(
            &self
                .container
                .query::<Track>()
                .filter(Track::folder().eq(id))
                .live(),
        );
        for t in ids {
            self.remove_track(t);
        }
        if let Err(e) = self.container.delete::<Folder>(id) {
            error!("could not remove the folder: {e}");
        }
        self.refresh_groups();
    }

    /// Remove one track and its playlist entries. The file itself stays where it is, unless it
    /// was imported under the app's own files, which the import made a copy.
    pub fn remove_track(&self, id: u64) {
        let entries = Self::ids_of(
            &self
                .container
                .query::<PlaylistEntry>()
                .filter(PlaylistEntry::track().eq(id))
                .live(),
        );
        for e in entries {
            let _ = self.container.delete::<PlaylistEntry>(e);
        }
        if let Some(t) = self.container.get::<Track>(id)
            && t.folder().read() == 0
            && let Some(path) = self.file_path(&TrackFields::path(t).read())
        {
            let _ = std::fs::remove_file(&path);
        }
        if let Err(e) = self.container.delete::<Track>(id) {
            error!("could not remove the track: {e}");
        }
    }

    // --- folders and scanning -----------------------------------------------------------------

    pub fn folders(&self) -> Query<Folder> {
        self.container
            .query::<Folder>()
            .sort(Folder::path().asc())
            .live()
    }

    /// Start (or re-run) a scan of `path`, on a worker thread; rows land in batches. The
    /// folder row is made first so the sidebar shows it at once.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn scan_folder(&'static self, path: PathBuf) {
        let folder = hash(path.to_string_lossy().as_bytes());
        if self.container.get::<Folder>(folder).is_none() {
            self.container.insert(Folder {
                id: folder,
                path: path.to_string_lossy().into_owned(),
                added: now_secs(),
                scanned: 0,
                tracks: 0,
            });
        }
        let known: std::collections::HashSet<u64> = Self::ids_of(
            &self
                .container
                .query::<Track>()
                .filter(Track::folder().eq(folder))
                .live(),
        )
        .into_iter()
        .collect();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        self.scanning.set(Some(ScanProgress {
            folder: name.clone(),
            seen: 0,
            added: 0,
        }));
        let art_root = self.files.as_ref().map(|f| f.join("art"));
        std::thread::spawn(move || {
            let mut files = Vec::new();
            walk(&path, &mut files, 0);
            files.sort();
            let total_seen = files.len() as u64;
            let mut batch = Vec::new();
            let mut added = 0u64;
            let mut found = std::collections::HashSet::new();
            for (i, file) in files.iter().enumerate() {
                let id = track_id(&file.to_string_lossy());
                found.insert(id);
                if !known.contains(&id)
                    && let Some(track) = read_track(file, folder, art_root.as_deref())
                {
                    batch.push(track);
                    added += 1;
                }
                if batch.len() >= 64 || i + 1 == files.len() {
                    let rows = std::mem::take(&mut batch);
                    let seen = (i + 1) as u64;
                    let name = name.clone();
                    day::reactive::on_main(move || {
                        let lib = crate::App::app().library;
                        for row in rows {
                            lib.container.insert(row);
                        }
                        lib.scanning.set(Some(ScanProgress {
                            folder: name,
                            seen,
                            added,
                        }));
                    });
                }
            }
            // Files that left the folder leave the library.
            let gone: Vec<u64> = known.difference(&found).copied().collect();
            day::reactive::on_main(move || {
                let lib = crate::App::app().library;
                for id in gone {
                    lib.remove_track(id);
                }
                if let Some(f) = lib.container.get::<Folder>(folder) {
                    f.scanned().write(now_secs());
                    f.tracks().write(total_seen as i64);
                }
                lib.scanning.set(None);
                lib.refresh_groups();
                info!("scanned {total_seen} files, {added} new");
            });
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub fn scan_folder(&'static self, _path: PathBuf) {}

    /// Copy picked files into the app's own media directory and add them as tracks the way
    /// a scan would: how a phone, which has no folders to point at, builds a library.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_files(&'static self, files: Vec<PathBuf>) {
        let Some(root) = self.files.clone() else {
            return;
        };
        let media = root.join("media");
        let art = root.join("art");
        self.scanning.set(Some(ScanProgress {
            folder: String::new(),
            seen: 0,
            added: 0,
        }));
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&media);
            let mut rows = Vec::new();
            for (i, src) in files.iter().enumerate() {
                let ext = src
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if !day_piece_media::tags::is_audio_extension(&ext) {
                    continue;
                }
                // Named by content, so importing the same file twice yields one track.
                let Ok(bytes) = std::fs::read(src) else {
                    continue;
                };
                let name = format!("{:016x}.{ext}", hash(&bytes));
                let dest = media.join(&name);
                if !dest.exists() && std::fs::write(&dest, &bytes).is_err() {
                    continue;
                }
                if let Some(mut track) = read_track(&dest, 0, Some(&art)) {
                    // Kept relative to the app's files: the container moves between installs
                    // on the phones, and a stable path also gives the row a stable id.
                    track.path = format!("media/{name}");
                    track.id = track_id(&track.path);
                    if track.title.is_empty() {
                        track.title = src
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default();
                    }
                    rows.push(track);
                }
                let seen = (i + 1) as u64;
                let added = rows.len() as u64;
                day::reactive::on_main(move || {
                    crate::App::app().library.scanning.set(Some(ScanProgress {
                        folder: String::new(),
                        seen,
                        added,
                    }));
                });
            }
            day::reactive::on_main(move || {
                let lib = crate::App::app().library;
                let n = rows.len();
                for row in rows {
                    lib.container.insert(row);
                }
                lib.scanning.set(None);
                lib.refresh_groups();
                info!("imported {n} files");
            });
        });
    }

    #[cfg(target_arch = "wasm32")]
    pub fn import_files(&'static self, _files: Vec<PathBuf>) {}

    /// Import the audio files a bundled asset directory lists in its `manifest.json`
    /// (`{"files": [...]}`): the walkthrough's library, through the same path as Import Files….
    #[cfg(not(target_arch = "wasm32"))]
    pub fn import_asset_dir(&'static self, dir: &str) {
        let dir = dir.trim_matches('/').to_string();
        let Some(root) = self.files.clone() else {
            return;
        };
        let manifest = day::resource(day::AssetName::dynamic(format!("{dir}/manifest.json")));
        let Some(manifest) = manifest else {
            warn!("no bundled asset {dir}/manifest.json");
            return;
        };
        let names: Vec<String> = serde_json::from_slice::<serde_json::Value>(manifest.as_slice())
            .ok()
            .and_then(|v| v.get("files").cloned())
            .and_then(|f| serde_json::from_value(f).ok())
            .unwrap_or_default();
        let staging = root.join("import");
        let _ = std::fs::create_dir_all(&staging);
        let mut paths = Vec::new();
        for name in names {
            let Some(res) = day::resource(day::AssetName::dynamic(format!("{dir}/{name}"))) else {
                continue;
            };
            let dest = staging.join(&name);
            if std::fs::write(&dest, res.as_slice()).is_ok() {
                paths.push(dest);
            }
        }
        self.import_files(paths);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn import_asset_dir(&'static self, _dir: &str) {}

    // --- playlists --------------------------------------------------------------------------

    pub fn create_playlist(&self, name: &str) -> u64 {
        let name = name.trim();
        let id = hash_parts(&[name, &now_secs().to_string(), &rand_salt().to_string()]);
        let position = self
            .playlists
            .ids_untracked()
            .iter()
            .filter_map(|p| self.container.get::<Playlist>(p.handle()))
            .map(|p| p.position().read())
            .fold(0.0f64, f64::max)
            + 1.0;
        self.container.insert(Playlist {
            id,
            name: if name.is_empty() {
                String::new()
            } else {
                name.to_string()
            },
            created: now_secs(),
            position,
        });
        id
    }

    pub fn rename_playlist(&self, id: u64, name: &str) {
        if let Some(p) = self.container.get::<Playlist>(id) {
            p.name().write(name.trim().to_string());
        }
    }

    pub fn delete_playlist(&self, id: u64) {
        for e in Self::ids_of(&self.playlist_entries(id)) {
            let _ = self.container.delete::<PlaylistEntry>(e);
        }
        if let Err(e) = self.container.delete::<Playlist>(id) {
            error!("could not delete the playlist: {e}");
        }
    }

    pub fn playlist(&self, id: u64) -> Option<Playlist> {
        self.container.get::<Playlist>(id)?;
        self.container
            .cache::<Playlist>()
            .with_untracked(|k| k.get(id).cloned())
    }

    pub fn playlist_entries(&self, id: u64) -> Query<PlaylistEntry> {
        self.container
            .query::<PlaylistEntry>()
            .filter(PlaylistEntry::playlist().eq(id))
            .sort(PlaylistEntry::position().asc())
            .live()
    }

    /// The track ids of a playlist, in order.
    pub fn playlist_tracks(&self, id: u64) -> Vec<u64> {
        Self::ids_of(&self.playlist_entries(id))
            .into_iter()
            .filter_map(|e| self.container.get::<PlaylistEntry>(e))
            .map(|e| e.track().read())
            .collect()
    }

    pub fn entry_track(&self, entry: u64) -> Option<u64> {
        self.container
            .get::<PlaylistEntry>(entry)
            .map(|e| e.track().read())
    }

    /// Append tracks to a playlist (a track may appear more than once, as in every player).
    pub fn add_to_playlist(&self, playlist: u64, tracks: &[u64]) {
        let mut position = Self::ids_of(&self.playlist_entries(playlist))
            .iter()
            .filter_map(|e| self.container.get::<PlaylistEntry>(*e))
            .map(|e| e.position().read())
            .fold(0.0f64, f64::max);
        for t in tracks {
            position += 1.0;
            let id = hash_parts(&[
                &playlist.to_string(),
                &t.to_string(),
                &position.to_string(),
                &rand_salt().to_string(),
            ]);
            self.container.insert(PlaylistEntry {
                id,
                playlist,
                track: *t,
                position,
            });
        }
    }

    pub fn remove_entry(&self, entry: u64) {
        let _ = self.container.delete::<PlaylistEntry>(entry);
    }

    /// Move an entry from `from` to `to` within its playlist: one fractional write.
    pub fn move_entry(&self, playlist: u64, from: usize, to: usize) {
        let ids = Self::ids_of(&self.playlist_entries(playlist));
        if from >= ids.len() || to >= ids.len() || from == to {
            return;
        }
        let pos = |i: usize| -> f64 {
            self.container
                .get::<PlaylistEntry>(ids[i])
                .map(|e| e.position().read())
                .unwrap_or(0.0)
        };
        let target = if to == 0 {
            pos(0) - 1.0
        } else if to == ids.len() - 1 {
            pos(ids.len() - 1) + 1.0
        } else if from < to {
            (pos(to) + pos(to + 1)) / 2.0
        } else {
            (pos(to - 1) + pos(to)) / 2.0
        };
        if let Some(e) = self.container.get::<PlaylistEntry>(ids[from]) {
            e.position().write(target);
        }
    }

    // --- stations -----------------------------------------------------------------------------

    pub fn station(&self, id: u64) -> Option<Station> {
        self.container.get::<Station>(id)?;
        self.container
            .cache::<Station>()
            .with_untracked(|k| k.get(id).cloned())
    }

    /// Keep (or refresh) a station row from the index's data, keeping the listener's own
    /// columns.
    pub fn remember_station(&self, s: &Station) -> u64 {
        if let Some(row) = self.container.get::<Station>(s.id) {
            row.name().write(s.name.clone());
            row.url().write(s.url.clone());
            row.url_resolved().write(s.url_resolved.clone());
            row.homepage().write(s.homepage.clone());
            row.favicon().write(s.favicon.clone());
            row.tags().write(s.tags.clone());
            row.country().write(s.country.clone());
            row.countrycode().write(s.countrycode.clone());
            row.codec().write(s.codec.clone());
            row.bitrate().write(s.bitrate);
            row.hls().write(s.hls);
            row.votes().write(s.votes);
        } else {
            let mut row = s.clone();
            row.added = now_secs();
            self.container.insert(row);
        }
        s.id
    }

    pub fn is_favorite_station(&self, id: u64) -> bool {
        self.favorites.contains(id)
    }

    pub fn toggle_favorite_station(&self, s: &Station) {
        let id = self.remember_station(s);
        if let Some(row) = self.container.get::<Station>(id) {
            let now = !row.favorite().read();
            row.favorite().write(now);
            if now {
                row.added().write(now_secs());
            }
        }
    }

    pub fn favorite_stations(&self) -> Query<Station> {
        self.container
            .query::<Station>()
            .filter(Station::favorite().eq(true))
            .sort(Station::added().desc())
            .live()
    }

    pub fn recent_stations(&self) -> Query<Station> {
        self.container
            .query::<Station>()
            .filter(Station::last_played().gt(0i64))
            .sort(Station::last_played().desc())
            .limit(50)
            .live()
    }

    pub fn touch_station(&self, s: &Station) {
        let id = self.remember_station(s);
        if let Some(row) = self.container.get::<Station>(id) {
            row.play_count().write(row.play_count().read() + 1);
            row.last_played().write(now_secs());
        }
        self.touch_history(KIND_STATION, id, s.name.clone(), s.country.clone());
    }

    /// Forget a station that is neither a favorite nor recently played.
    pub fn forget_station(&self, id: u64) {
        let _ = self.container.delete::<Station>(id);
    }

    // --- podcasts -----------------------------------------------------------------------------

    pub fn podcasts(&self) -> Query<Podcast> {
        self.container
            .query::<Podcast>()
            .sort(Podcast::title().asc())
            .live()
    }

    pub fn podcast(&self, id: u64) -> Option<Podcast> {
        self.container.get::<Podcast>(id)?;
        self.container
            .cache::<Podcast>()
            .with_untracked(|k| k.get(id).cloned())
    }

    pub fn is_subscribed(&self, feed: &str) -> bool {
        self.container.get::<Podcast>(podcast_id(feed)).is_some()
    }

    pub fn subscribe(&self, p: Podcast) -> u64 {
        let id = p.id;
        if self.container.get::<Podcast>(id).is_none() {
            self.container.insert(p);
        }
        id
    }

    pub fn unsubscribe(&self, id: u64) {
        for e in Self::ids_of(&self.podcast_episodes(id)) {
            if let Some(ep) = self.container.get::<Episode>(e) {
                let local = ep.local().read();
                if !local.is_empty() {
                    let _ = std::fs::remove_file(local);
                }
            }
            let _ = self.container.delete::<Episode>(e);
        }
        if let Err(e) = self.container.delete::<Podcast>(id) {
            error!("could not unsubscribe: {e}");
        }
    }

    pub fn episode(&self, id: u64) -> Option<Episode> {
        self.container.get::<Episode>(id)?;
        self.container
            .cache::<Episode>()
            .with_untracked(|k| k.get(id).cloned())
    }

    pub fn podcast_episodes(&self, podcast: u64) -> Query<Episode> {
        self.container
            .query::<Episode>()
            .filter(Episode::podcast().eq(podcast))
            .sort(Episode::published().desc())
            .live()
    }

    /// The newest episodes across every subscription, unplayed first.
    pub fn latest_episodes(&self) -> Query<Episode> {
        self.container
            .query::<Episode>()
            .sort(Episode::published().desc())
            .limit(300)
            .live()
    }

    pub fn unplayed_count(&self) -> day_persistence::CountQuery<Episode> {
        self.container
            .query::<Episode>()
            .filter(Episode::played().eq(false))
            .live_count()
    }

    pub fn downloaded_episodes(&self) -> Query<Episode> {
        self.container
            .query::<Episode>()
            .filter(Episode::download().ne(DOWNLOAD_NONE))
            .sort(Episode::published().desc())
            .live()
    }

    /// Merge fetched episodes into a subscription: new rows are added, known rows keep the
    /// listener's position and played flag. Returns how many were new.
    pub fn merge_episodes(&self, podcast: u64, rows: Vec<Episode>) -> usize {
        let mut added = 0;
        for row in rows {
            if let Some(known) = self.container.get::<Episode>(row.id) {
                known.title().write(row.title);
                known.summary().write(row.summary);
                known.url().write(row.url);
                if row.duration > 0.0 {
                    known.duration().write(row.duration);
                }
                known.image().write(row.image);
            } else {
                self.container.insert(row);
                added += 1;
            }
        }
        if let Some(p) = self.container.get::<Podcast>(podcast) {
            p.refreshed().write(now_secs());
            let n = self.podcast_episodes(podcast).ids_untracked().len() as i64;
            p.episodes().write(n);
        }
        added
    }

    pub fn set_episode_position(&self, id: u64, position: f64, played: Option<bool>) {
        if let Some(ep) = self.container.get::<Episode>(id) {
            if (ep.position().read() - position).abs() >= 1.0 {
                ep.position().write(position);
            }
            if let Some(p) = played
                && ep.played().read() != p
            {
                ep.played().write(p);
            }
        }
    }

    pub fn touch_episode(&self, id: u64) {
        if let Some(ep) = self.container.get::<Episode>(id) {
            let show = self
                .podcast(ep.podcast().read())
                .map(|p| p.title)
                .unwrap_or_default();
            self.touch_history(KIND_EPISODE, id, ep.title().read(), show);
        }
    }

    // --- history and scrobbles ----------------------------------------------------------------

    pub fn history(&self) -> Query<History> {
        self.container
            .query::<History>()
            .sort(History::played().desc())
            .live()
    }

    pub fn history_row(&self, id: u64) -> Option<History> {
        self.container.get::<History>(id)?;
        self.container
            .cache::<History>()
            .with_untracked(|k| k.get(id).cloned())
    }

    fn touch_history(&self, kind: i64, item: u64, title: String, subtitle: String) {
        let id = history_id(kind, item);
        if let Some(h) = self.container.get::<History>(id) {
            h.played().write(now_secs());
            h.title().write(title);
            h.subtitle().write(subtitle);
        } else {
            self.container.insert(History {
                id,
                kind,
                item,
                title,
                subtitle,
                played: now_secs(),
            });
        }
        let ids = Self::ids_of(&self.history());
        for old in ids.iter().skip(HISTORY_CAP) {
            let _ = self.container.delete::<History>(*old);
        }
    }

    pub fn clear_history(&self) {
        for id in Self::ids_of(&self.history()) {
            let _ = self.container.delete::<History>(id);
        }
    }

    pub fn queue_scrobble(
        &self,
        artist: &str,
        title: &str,
        album: &str,
        duration: f64,
        started: i64,
    ) {
        let id = hash_parts(&[artist, title, &started.to_string()]);
        if self.container.get::<Scrobble>(id).is_some() {
            return;
        }
        self.container.insert(Scrobble {
            id,
            artist: artist.to_string(),
            title: title.to_string(),
            album: album.to_string(),
            duration,
            started,
            attempts: 0,
        });
    }

    pub fn pending_scrobbles(&self) -> Query<Scrobble> {
        self.container
            .query::<Scrobble>()
            .sort(Scrobble::started().asc())
            .live()
    }

    pub fn scrobble_row(&self, id: u64) -> Option<Scrobble> {
        self.container.get::<Scrobble>(id)?;
        self.container
            .cache::<Scrobble>()
            .with_untracked(|k| k.get(id).cloned())
    }

    pub fn scrobbled(&self, ids: &[u64]) {
        for id in ids {
            let _ = self.container.delete::<Scrobble>(*id);
        }
    }

    pub fn scrobble_failed(&self, ids: &[u64]) {
        for id in ids {
            if let Some(s) = self.container.get::<Scrobble>(*id) {
                let n = s.attempts().read() + 1;
                if n > 20 {
                    let _ = self.container.delete::<Scrobble>(*id);
                } else {
                    s.attempts().write(n);
                }
            }
        }
    }

    // --- files --------------------------------------------------------------------------------

    /// Where a track's audio is: the scanned file's own path, or an imported copy resolved
    /// under the app's files of this install.
    pub fn track_url(&self, track: &Track) -> String {
        if track.folder == 0 {
            self.file_path(&track.path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| track.path.clone())
        } else {
            track.path.clone()
        }
    }

    /// The absolute path of a file under the app's own files (`art/x.jpg`, `media/y.mp3`).
    pub fn file_path(&self, relative: &str) -> Option<PathBuf> {
        if relative.is_empty() {
            return None;
        }
        self.files.as_ref().map(|r| r.join(relative))
    }

    /// Where a downloaded episode goes.
    pub fn episode_download_path(&self, episode: &Episode) -> Option<PathBuf> {
        let ext = extension_for(&episode.mime, &episode.url);
        let root = self.files.as_ref()?.join("podcasts");
        let _ = std::fs::create_dir_all(&root);
        Some(root.join(format!("{:016x}.{ext}", episode.id)))
    }
}

/// The directory the app keeps its own files in: imported media, artwork, downloads.
fn files_root() -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let root = day_part_filesystem::data_dir()
            .ok()?
            .join("day-fs")
            .join("tunes");
        for sub in ["art", "media", "podcasts"] {
            let _ = std::fs::create_dir_all(root.join(sub));
        }
        Some(root)
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// A file extension for an episode's enclosure, from its MIME type first.
pub fn extension_for(mime: &str, url: &str) -> String {
    let m = mime.to_ascii_lowercase();
    let by_mime = match m.as_str() {
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/mp4" | "audio/x-m4a" | "audio/m4a" | "audio/aac" => Some("m4a"),
        "audio/ogg" | "audio/vorbis" => Some("ogg"),
        "audio/opus" => Some("opus"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "video/mp4" => Some("mp4"),
        _ => None,
    };
    if let Some(e) = by_mime {
        return e.to_string();
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path.rsplit('.').next().unwrap_or("");
    if (1..=4).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext.to_ascii_lowercase()
    } else {
        "mp3".to_string()
    }
}

/// A few bits of entropy for ids that have no natural identity (a new playlist).
fn rand_salt() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    let n = NEXT.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    n ^ (n >> 29)
}

/// A row cell as text; `NULL` and non-text read empty.
fn text(v: &Value) -> String {
    v.as_text().unwrap_or_default().to_string()
}

/// How the Songs list is ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TrackSort {
    #[default]
    Title,
    Artist,
    Album,
    Added,
    Played,
}

/// Free text → an FTS5 expression: every word quoted and prefix-matched, so typing feels live
/// and a stray quote or dash is searched for rather than parsed as syntax.
pub fn fts_expression(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(words.join(" "))
}

// --- the scanner ----------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 16 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk(&path, out, depth + 1);
        } else if kind.is_file() || kind.is_symlink() {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if day_piece_media::tags::is_audio_extension(&ext) {
                out.push(path);
            }
        }
    }
}

/// Read one file into a track row; `None` for a file that cannot be opened. Embedded artwork
/// lands in the art cache, named by its bytes, so an album's ten files share one picture.
#[cfg(not(target_arch = "wasm32"))]
fn read_track(path: &Path, folder: u64, art_root: Option<&Path>) -> Option<Track> {
    use day_piece_media::tags;
    let tags = match tags::read_file(path) {
        Ok(t) => t,
        Err(e) => {
            warn!("could not read {}: {e}", path.display());
            return None;
        }
    };
    let path_text = path.to_string_lossy().into_owned();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let title = tags.title.filter(|t| !t.trim().is_empty()).unwrap_or(stem);
    let artist = tags.artist.unwrap_or_default().trim().to_string();
    let album = tags.album.unwrap_or_default().trim().to_string();
    let album_artist = tags
        .album_artist
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| artist.clone());
    let artwork = match (tags.picture, art_root) {
        (Some(pic), Some(root)) => store_artwork(root, &pic.mime, &pic.data),
        _ => String::new(),
    };
    Some(Track {
        id: track_id(&path_text),
        path: path_text,
        title: title.trim().to_string(),
        album_key: album_key(&album_artist, &album),
        artist_key: artist_key(&artist),
        artist,
        album,
        album_artist,
        genre: tags.genre.unwrap_or_default().trim().to_string(),
        year: tags.year.map(|y| y as i64).unwrap_or(0),
        track_no: tags.track.map(|t| t as i64).unwrap_or(0),
        disc_no: tags.disc.map(|d| d as i64).unwrap_or(0),
        duration: tags.duration_secs.unwrap_or(0.0),
        format: format_name(tags.format).to_string(),
        bitrate: tags.bitrate_kbps.map(|b| b as i64).unwrap_or(0),
        artwork,
        added: now_secs(),
        play_count: 0,
        last_played: 0,
        folder,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn format_name(f: day_piece_media::tags::Format) -> &'static str {
    use day_piece_media::tags::Format;
    match f {
        Format::Mp3 => "MP3",
        Format::Mp4 => "AAC",
        Format::Flac => "FLAC",
        Format::OggVorbis => "Vorbis",
        Format::Opus => "Opus",
        Format::Wav => "WAV",
        Format::Aiff => "AIFF",
        Format::Unknown => "",
    }
}

/// Write a picture into the art cache and return its key (`art/<hash>.<ext>`).
#[cfg(not(target_arch = "wasm32"))]
fn store_artwork(root: &Path, mime: &str, data: &[u8]) -> String {
    if data.is_empty() || data.len() > 16 << 20 {
        return String::new();
    }
    let ext = if mime.contains("png") || data.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else {
        "jpg"
    };
    let name = format!("{:016x}.{ext}", hash(data));
    let dest = root.join(&name);
    if !dest.exists() {
        let _ = std::fs::create_dir_all(root);
        if std::fs::write(&dest, data).is_err() {
            return String::new();
        }
    }
    format!("art/{name}")
}

/// Store downloaded bytes (a station logo, a podcast cover) in the art cache, for the
/// platforms that draw file-backed images: returns the key.
pub fn cache_artwork_bytes(lib: &Library, data: &[u8]) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        match lib.files.as_ref() {
            Some(root) => store_artwork(&root.join("art"), "", data),
            None => String::new(),
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (lib, data);
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_positive() {
        assert_eq!(track_id("/a/b.mp3"), track_id("/a/b.mp3"));
        assert!(track_id("/a/b.mp3") <= i64::MAX as u64);
        assert_ne!(hash_parts(&["a", "bc"]), hash_parts(&["ab", "c"]));
        assert_ne!(episode_id("f", "1"), episode_id("f", "2"));
        assert_eq!(
            album_key("Miles Davis", "Kind of Blue"),
            album_key("miles davis", "KIND OF BLUE")
        );
    }

    /// The walkthrough names fixture rows by these ids (dayscript/walkthrough.yaml); a change
    /// to the hashing or the fixture shows up here first.
    #[test]
    fn fixture_ids_match_the_walkthrough() {
        assert_eq!(
            album_key("Aurora Fields", "Morning Light"),
            1_311_233_589_162_823_106
        );
        assert_eq!(artist_key("Aurora Fields"), 1_799_745_019_033_601_338);
        assert_eq!(station_id("f2"), 619_401_112_520_371_537);
        assert_eq!(
            episode_id("asset:podcast-fixture/feed-1.xml", "signal-3"),
            8_982_385_423_799_103_407
        );
        assert_eq!(
            podcast_id("asset:podcast-fixture/feed-1.xml"),
            1_608_913_888_395_334_559
        );
    }

    #[test]
    fn search_expressions_prefix_every_word_and_survive_quotes() {
        assert_eq!(
            fts_expression("so what").as_deref(),
            Some("\"so\"* \"what\"*")
        );
        assert_eq!(
            fts_expression("rock \"n\" roll").as_deref(),
            Some("\"rock\"* \"n\"* \"roll\"*")
        );
        assert_eq!(fts_expression("  -- "), None);
    }

    #[test]
    fn enclosure_extensions_prefer_the_mime_type() {
        assert_eq!(extension_for("audio/mpeg", "https://x/y.bin"), "mp3");
        assert_eq!(extension_for("", "https://x/show.m4a?sig=1"), "m4a");
        assert_eq!(extension_for("", "https://x/stream"), "mp3");
    }

    #[test]
    fn an_in_memory_library_round_trips_rows() {
        let lib = Library::in_memory();
        let id = lib.create_playlist("Morning");
        assert_eq!(lib.playlist(id).map(|p| p.name), Some("Morning".into()));
        lib.container.insert(Track {
            id: 7,
            title: "Seven".into(),
            ..Default::default()
        });
        lib.add_to_playlist(id, &[7, 7]);
        assert_eq!(lib.playlist_tracks(id), vec![7, 7]);
        lib.refresh_groups();
        assert_eq!(lib.track_count.get_untracked(), 1);
        lib.queue_scrobble("A", "T", "", 100.0, 5);
        assert_eq!(lib.pending_scrobbles().ids_untracked().len(), 1);
        lib.delete_playlist(id);
        assert!(lib.playlist(id).is_none());
    }
}
