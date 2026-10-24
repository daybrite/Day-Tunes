//! Transport state: the app-owned audio session (https://daybrite.dev/docs/reference/media)
//! and the play queue around it. Lists hand the transport a `Vec<Item>` and a start index;
//! the transport walks it, keeps counts and resume positions in the store, and reports what
//! the native engine is doing through `snapshot`.

use crate::library::{self, Library};
use crate::res;
use crate::scrobble::Listen;
use day::prelude::*;
use day_piece_media::{
    AudioSession, BackgroundAudio, NowPlayingInfo, PlaybackSnapshot, PlaybackState, RemoteCommand,
};

/// A queue entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Track(u64),
    Station(u64),
    Episode(u64),
}

impl Item {
    pub fn kind(self) -> i64 {
        match self {
            Item::Track(_) => library::KIND_TRACK,
            Item::Station(_) => library::KIND_STATION,
            Item::Episode(_) => library::KIND_EPISODE,
        }
    }
    pub fn id(self) -> u64 {
        match self {
            Item::Track(id) | Item::Station(id) | Item::Episode(id) => id,
        }
    }
}

/// A picture source for the now-playing surfaces.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Art {
    #[default]
    None,
    /// An artwork cache key (`art/<hash>.jpg`), relative to the app's files.
    File(String),
    Url(String),
}

/// The loaded entry, described for display.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub item: Item,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art: Art,
    pub duration: f64,
    pub live: bool,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

const LISTEN_AFTER_SECS: f64 = 240.0;
const RADIO_SONG_MIN_SECS: i64 = 30;
pub const VOLUME_KEY: &str = "player.volume";
pub const RATE_KEY: &str = "player.rate";

/// All handles; `Copy`.
#[derive(Clone, Copy)]
pub struct Transport {
    session: &'static AudioSession,
    pub snapshot: Signal<PlaybackSnapshot>,
    pub loaded: Signal<Option<Loaded>>,
    pub queue: Signal<Vec<Item>>,
    /// Play order over `queue` (identity unless shuffled).
    pub order: Signal<Vec<usize>>,
    pub pos: Signal<Option<usize>>,
    pub shuffle: Signal<bool>,
    pub repeat: Signal<Repeat>,
    pub volume: Signal<f64>,
    /// Speed for episodes.
    pub rate: Signal<f64>,
    /// The seek slider's value while it is being dragged.
    pub scrub: Signal<Option<f64>>,
    /// When the loaded entry started, for listens.
    started: Signal<i64>,
    /// Whether the loaded entry's listen was queued already.
    counted: Signal<bool>,
    /// A seek to apply once the source can seek (resume).
    pending_seek: Signal<Option<f64>>,
    /// The stream song on air before the current one, and when it began.
    radio_song: Signal<Option<(String, String, i64)>>,
    /// The last episode position written to the store.
    saved_at: Signal<f64>,
}

impl Default for Transport {
    fn default() -> Self {
        Transport::new()
    }
}

impl Transport {
    pub fn new() -> Transport {
        let session = AudioSession::new();
        let volume = day::prefs::get(VOLUME_KEY)
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(0.85, |v| v.clamp(0.0, 1.0));
        let rate = day::prefs::get(RATE_KEY)
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(1.0, |v| v.clamp(0.5, 3.0));
        let _ = session.set_volume(volume);
        session.background(BackgroundAudio {
            title: res::str::app_title().format(),
            description: res::str::background_description().format(),
            channel_name: res::str::background_channel().format(),
        });
        let t = Transport {
            snapshot: session.snapshot(),
            session: Box::leak(Box::new(session)),
            loaded: Signal::new(None),
            queue: Signal::new(Vec::new()),
            order: Signal::new(Vec::new()),
            pos: Signal::new(None),
            shuffle: Signal::new(false),
            repeat: Signal::new(Repeat::Off),
            volume: Signal::new(volume),
            rate: Signal::new(rate),
            scrub: Signal::new(None),
            started: Signal::new(0),
            counted: Signal::new(false),
            pending_seek: Signal::new(None),
            radio_song: Signal::new(None),
            saved_at: Signal::new(0.0),
        };
        t.session.on_remote(move |cmd| t.remote(cmd));
        t
    }

    /// Wire the watchers that need the whole app: call once from `root()`.
    pub fn install(self) {
        let app = crate::App::app();
        watch(
            move || self.volume.get(),
            move |v, _| {
                let _ = self.session.set_volume(*v);
                day::prefs::set(VOLUME_KEY, &format!("{v:.3}"));
            },
        );
        watch(
            move || self.rate.get(),
            move |r, _| {
                day::prefs::set(RATE_KEY, &format!("{r:.2}"));
                if self.is_episode() {
                    let _ = self.session.set_rate(*r);
                }
            },
        );
        // Everything the engine reports: the end of a file, the resume seek, positions,
        // listens, and the system's display.
        watch(
            move || self.snapshot.get(),
            move |snap, _| self.on_snapshot(app, snap),
        );
    }

    fn on_snapshot(self, app: crate::App, snap: &PlaybackSnapshot) {
        let Some(loaded) = self.loaded.get_untracked() else {
            return;
        };
        let pos = snap.timeline.position;
        let duration = snap.timeline.duration.unwrap_or(loaded.duration);
        if duration > 0.0 && (loaded.duration - duration).abs() > 0.5 {
            self.loaded.update(|l| {
                if let Some(l) = l {
                    l.duration = duration;
                }
            });
        }
        if let Some(target) = self.pending_seek.get_untracked()
            && snap.timeline.capabilities.seek
        {
            self.pending_seek.set(None);
            let _ = self.session.seek(target);
        }
        match loaded.item {
            Item::Episode(id) => {
                if snap.state == PlaybackState::Playing
                    && (pos - self.saved_at.get_untracked()).abs() >= 5.0
                {
                    self.saved_at.set(pos);
                    app.library.set_episode_position(id, pos, None);
                }
                if snap.state == PlaybackState::Ended {
                    app.library.set_episode_position(id, 0.0, Some(true));
                }
            }
            Item::Station(_) => self.radio_listens(app, snap),
            Item::Track(_) => {}
        }
        // A file counts as listened past half its length or four minutes.
        if !loaded.live
            && !self.counted.get_untracked()
            && snap.state == PlaybackState::Playing
            && duration > 30.0
            && pos >= (duration / 2.0).min(LISTEN_AFTER_SECS)
        {
            self.counted.set(true);
            if app.scrobbling.wants(loaded.item.kind()) {
                app.scrobbling.listened(
                    app.library,
                    &Listen {
                        artist: loaded.artist.clone(),
                        title: loaded.title.clone(),
                        album: loaded.album.clone(),
                        duration,
                        started: self.started.get_untracked(),
                    },
                );
            }
        }
        if snap.state == PlaybackState::Ended {
            self.advance(true);
        }
        self.publish(snap);
    }

    /// Radio: a new stream title ends the song before it; a song that lasted long enough is a
    /// listen.
    fn radio_listens(self, app: crate::App, snap: &PlaybackSnapshot) {
        let now = library::now_secs();
        let current = snap
            .metadata
            .as_ref()
            .filter(|m| !m.title.is_empty())
            .map(|m| (m.artist.clone(), m.title.clone()));
        let previous = self.radio_song.get_untracked();
        match (previous, current) {
            (Some((a, t, since)), Some((na, nt))) if a == na && t == nt => {
                let _ = since;
            }
            (previous, current) => {
                if let Some((artist, title, since)) = previous
                    && !artist.is_empty()
                    && now - since >= RADIO_SONG_MIN_SECS
                    && app.scrobbling.wants(library::KIND_STATION)
                {
                    app.scrobbling.listened(
                        app.library,
                        &Listen {
                            artist,
                            title,
                            album: String::new(),
                            duration: 0.0,
                            started: since,
                        },
                    );
                }
                match current {
                    Some((artist, title)) => {
                        if !artist.is_empty() && app.scrobbling.wants(library::KIND_STATION) {
                            app.scrobbling.now_playing(Listen {
                                artist: artist.clone(),
                                title: title.clone(),
                                album: String::new(),
                                duration: 0.0,
                                started: now,
                            });
                        }
                        self.radio_song.set(Some((artist, title, now)));
                    }
                    None => self.radio_song.set(None),
                }
            }
        }
    }

    // --- what is on ----------------------------------------------------------------------

    pub fn state(self) -> PlaybackState {
        self.snapshot.with(|s| s.state.clone())
    }

    pub fn is_active(self) -> bool {
        self.snapshot.with(|s| s.state.is_active())
    }

    pub fn position(self) -> f64 {
        self.snapshot.with(|s| s.timeline.position)
    }

    pub fn can_seek(self) -> bool {
        self.snapshot.with(|s| s.timeline.capabilities.seek)
    }

    pub fn current(self) -> Option<Item> {
        self.loaded.with(|l| l.as_ref().map(|l| l.item))
    }

    pub fn is_current(self, item: Item) -> bool {
        self.current() == Some(item)
    }

    fn is_episode(self) -> bool {
        matches!(
            self.loaded.with_untracked(|l| l.as_ref().map(|l| l.item)),
            Some(Item::Episode(_))
        )
    }

    /// The stream's own song, for a station on air.
    pub fn stream_title(self) -> Option<day_piece_media::StreamMetadata> {
        self.snapshot.with(|s| s.metadata.clone())
    }

    pub fn has_next(self) -> bool {
        let len = self.order.with(|o| o.len());
        match self.pos.get() {
            Some(p) => p + 1 < len || (len > 0 && self.repeat.get() == Repeat::All),
            None => false,
        }
    }

    pub fn has_previous(self) -> bool {
        self.pos.get().is_some_and(|p| p > 0)
            || (self.pos.get().is_some() && self.repeat.get() == Repeat::All)
    }

    // --- commands -------------------------------------------------------------------------

    /// Play `items`, starting at `start`. The queue becomes this list.
    pub fn play_items(self, items: Vec<Item>, start: usize) {
        if items.is_empty() {
            return;
        }
        let start = start.min(items.len() - 1);
        let n = items.len();
        self.queue.set(items);
        let order = self.make_order(n, Some(start));
        let pos = order.iter().position(|i| *i == start).unwrap_or(0);
        self.order.set(order);
        self.pos.set(Some(pos));
        self.load_current();
    }

    pub fn play_item(self, item: Item) {
        self.play_items(vec![item], 0);
    }

    /// Put `items` right after the current one.
    pub fn play_next(self, items: Vec<Item>) {
        if self.queue.with_untracked(|q| q.is_empty()) {
            self.play_items(items, 0);
            return;
        }
        let base = self.queue.with_untracked(|q| q.len());
        let n = items.len();
        self.queue.update(|q| q.extend(items));
        let at = self.pos.get_untracked().map(|p| p + 1).unwrap_or(0);
        self.order.update(|o| {
            for (k, idx) in (base..base + n).enumerate() {
                o.insert((at + k).min(o.len()), idx);
            }
        });
    }

    /// Append `items` to the queue.
    pub fn add_to_queue(self, items: Vec<Item>) {
        if self.queue.with_untracked(|q| q.is_empty()) {
            self.play_items(items, 0);
            return;
        }
        let base = self.queue.with_untracked(|q| q.len());
        let n = items.len();
        self.queue.update(|q| q.extend(items));
        self.order.update(|o| o.extend(base..base + n));
    }

    /// Remove the entry at play-order position `at`.
    pub fn remove_at(self, at: usize) {
        let len = self.order.with_untracked(|o| o.len());
        if at >= len {
            return;
        }
        let current = self.pos.get_untracked();
        self.order.update(|o| {
            o.remove(at);
        });
        match current {
            Some(p) if p == at => {
                let len = len - 1;
                if len == 0 {
                    self.clear_queue();
                } else {
                    self.pos.set(Some(p.min(len - 1)));
                    self.load_current();
                }
            }
            Some(p) if p > at => self.pos.set(Some(p - 1)),
            _ => {}
        }
    }

    /// Move the entry at play-order position `from` to `to`.
    pub fn move_entry(self, from: usize, to: usize) {
        let len = self.order.with_untracked(|o| o.len());
        if from >= len || to >= len || from == to {
            return;
        }
        let current = self
            .pos
            .get_untracked()
            .and_then(|p| self.order.with_untracked(|o| o.get(p).copied()));
        self.order.update(|o| {
            let it = o.remove(from);
            o.insert(to, it);
        });
        if let Some(c) = current {
            let p = self
                .order
                .with_untracked(|o| o.iter().position(|i| *i == c));
            self.pos.set(p);
        }
    }

    pub fn clear_queue(self) {
        self.session.stop();
        self.queue.set(Vec::new());
        self.order.set(Vec::new());
        self.pos.set(None);
        self.loaded.set(None);
        self.radio_song.set(None);
        self.publish(&PlaybackSnapshot::default());
    }

    /// Jump to play-order position `at`.
    pub fn play_at(self, at: usize) {
        if at < self.order.with_untracked(|o| o.len()) {
            self.pos.set(Some(at));
            self.load_current();
        }
    }

    pub fn toggle(self) {
        match self.snapshot.with_untracked(|s| s.state.clone()) {
            PlaybackState::Playing | PlaybackState::Loading => self.session.pause(),
            PlaybackState::Paused => self.session.play(),
            PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error(_) => {
                if self.loaded.with_untracked(|l| l.is_some()) {
                    self.load_current();
                }
            }
        }
    }

    pub fn pause(self) {
        self.session.pause();
    }

    pub fn play(self) {
        if matches!(
            self.snapshot.with_untracked(|s| s.state.clone()),
            PlaybackState::Paused
        ) {
            self.session.play();
        } else {
            self.toggle();
        }
    }

    pub fn stop(self) {
        self.session.stop();
        self.radio_song.set(None);
    }

    pub fn next(self) {
        self.advance(false);
    }

    /// Back one entry; or to the start of a file that has run for more than a few seconds.
    pub fn previous(self) {
        let live = self
            .loaded
            .with_untracked(|l| l.as_ref().is_some_and(|l| l.live));
        if !live
            && self.snapshot.with_untracked(|s| s.timeline.position) > 3.0
            && self.can_seek_untracked()
        {
            let _ = self.session.seek(0.0);
            return;
        }
        let len = self.order.with_untracked(|o| o.len());
        let Some(p) = self.pos.get_untracked() else {
            return;
        };
        let target = if p > 0 {
            p - 1
        } else if self.repeat.get_untracked() == Repeat::All && len > 0 {
            len - 1
        } else {
            let _ = self.session.seek(0.0);
            return;
        };
        self.pos.set(Some(target));
        self.load_current();
    }

    fn can_seek_untracked(self) -> bool {
        self.snapshot
            .with_untracked(|s| s.timeline.capabilities.seek)
    }

    /// Step to the next entry; at the end, repeat or stop. `auto` is the end of a file.
    fn advance(self, auto: bool) {
        let len = self.order.with_untracked(|o| o.len());
        let Some(p) = self.pos.get_untracked() else {
            return;
        };
        if auto && self.repeat.get_untracked() == Repeat::One {
            self.load_current();
            return;
        }
        if p + 1 < len {
            self.pos.set(Some(p + 1));
            self.load_current();
        } else if self.repeat.get_untracked() == Repeat::All && len > 0 {
            self.pos.set(Some(0));
            self.load_current();
        } else if auto {
            self.session.stop();
        }
    }

    pub fn seek(self, seconds: f64) {
        if self.can_seek_untracked() {
            let _ = self.session.seek(seconds.max(0.0));
        }
    }

    pub fn skip(self, delta: f64) {
        let pos = self.snapshot.with_untracked(|s| s.timeline.position);
        self.seek((pos + delta).max(0.0));
    }

    pub fn set_shuffle(self, on: bool) {
        self.shuffle.set(on);
        let current = self
            .pos
            .get_untracked()
            .and_then(|p| self.order.with_untracked(|o| o.get(p).copied()));
        let n = self.queue.with_untracked(|q| q.len());
        let order = self.make_order(n, current);
        let pos = current.and_then(|c| order.iter().position(|i| *i == c));
        self.order.set(order);
        self.pos.set(pos);
    }

    pub fn cycle_repeat(self) {
        self.repeat.update(|r| {
            *r = match r {
                Repeat::Off => Repeat::All,
                Repeat::All => Repeat::One,
                Repeat::One => Repeat::Off,
            }
        });
    }

    /// The play order for `n` entries: shuffled with `first` at the front, or 0..n.
    fn make_order(self, n: usize, first: Option<usize>) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        if self.shuffle.get_untracked() && n > 1 {
            let mut seed = (library::now_secs() as u64) ^ 0x9e37_79b9_7f4a_7c15;
            for i in (1..n).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let j = (seed % (i as u64 + 1)) as usize;
                order.swap(i, j);
            }
            if let Some(f) = first
                && let Some(k) = order.iter().position(|i| *i == f)
            {
                order.swap(0, k);
            }
        }
        order
    }

    /// Load the entry at `pos` and play it.
    fn load_current(self) {
        let app = crate::App::app();
        let Some(item) = self
            .pos
            .get_untracked()
            .and_then(|p| self.order.with_untracked(|o| o.get(p).copied()))
            .and_then(|i| self.queue.with_untracked(|q| q.get(i).copied()))
        else {
            return;
        };
        let Some(loaded) = describe(app.library, item) else {
            // The row is gone; skip it.
            self.remove_at(self.pos.get_untracked().unwrap_or(0));
            return;
        };
        info!("playing {} / {}", loaded.title, loaded.artist);
        self.pending_seek.set(None);
        self.counted.set(false);
        self.saved_at.set(0.0);
        self.radio_song.set(None);
        self.started.set(library::now_secs());
        match item {
            Item::Track(id) => app.library.touch_track(id),
            Item::Station(id) => {
                if let Some(s) = app.library.station(id) {
                    app.library.touch_station(&s);
                    app.radio.played(&s.uuid);
                }
            }
            Item::Episode(id) => {
                app.library.touch_episode(id);
                if let Some(ep) = app.library.episode(id)
                    && ep.position > 5.0
                    && (ep.duration <= 0.0 || ep.position < ep.duration - 5.0)
                {
                    self.pending_seek.set(Some(ep.position));
                }
            }
        }
        let url = loaded.url.clone();
        let is_episode = matches!(item, Item::Episode(_));
        self.loaded.set(Some(loaded.clone()));
        if let Err(e) = self.session.load(url, true) {
            error!("could not load: {e}");
        }
        let _ = self.session.set_rate(if is_episode {
            self.rate.get_untracked()
        } else {
            1.0
        });
        if !loaded.live && app.scrobbling.wants(item.kind()) {
            app.scrobbling.now_playing(Listen {
                artist: loaded.artist,
                title: loaded.title,
                album: loaded.album,
                duration: loaded.duration,
                started: self.started.get_untracked(),
            });
        }
    }

    /// Tell the system what is on, for the lock screen and the media keys.
    fn publish(self, snap: &PlaybackSnapshot) {
        let info = self.loaded.with_untracked(|l| {
            l.as_ref().map(|l| {
                let (title, artist) = match (&snap.metadata, l.live) {
                    (Some(m), true) if !m.title.is_empty() => (
                        m.title.clone(),
                        if m.artist.is_empty() {
                            l.title.clone()
                        } else {
                            m.artist.clone()
                        },
                    ),
                    _ => (l.title.clone(), l.artist.clone()),
                };
                NowPlayingInfo {
                    title,
                    artist,
                    album: if l.live {
                        l.title.clone()
                    } else {
                        l.album.clone()
                    },
                    artwork_path: match &l.art {
                        Art::File(key) => crate::App::app()
                            .library
                            .file_path(key)
                            .map(|p| p.to_string_lossy().into_owned()),
                        _ => None,
                    },
                    artwork_url: match &l.art {
                        Art::Url(u) => Some(u.clone()),
                        _ => None,
                    },
                    duration: snap
                        .timeline
                        .duration
                        .or((l.duration > 0.0).then_some(l.duration)),
                    position: snap.timeline.position,
                    rate: snap.timeline.rate,
                    playing: snap.state == PlaybackState::Playing,
                    live: l.live,
                    has_next: untrack(|| self.has_next()),
                    has_previous: untrack(|| self.pos.get().is_some()),
                }
            })
        });
        self.session.now_playing(info);
    }

    fn remote(self, cmd: RemoteCommand) {
        match cmd {
            RemoteCommand::Play => self.play(),
            RemoteCommand::Pause => self.pause(),
            RemoteCommand::Toggle => self.toggle(),
            RemoteCommand::Stop => self.stop(),
            RemoteCommand::Next => self.next(),
            RemoteCommand::Previous => self.previous(),
            RemoteCommand::Seek(s) => self.seek(s),
            RemoteCommand::SkipForward(d) => self.skip(d),
            RemoteCommand::SkipBackward(d) => self.skip(-d),
        }
    }
}

/// An item as the now-playing surfaces describe it, from the store.
pub fn describe(lib: &Library, item: Item) -> Option<Loaded> {
    match item {
        Item::Track(id) => {
            let t = lib.track(id)?;
            let url = lib.track_url(&t);
            Some(Loaded {
                item,
                title: t.title,
                artist: t.artist,
                album: t.album,
                art: art_for_key(lib, &t.artwork),
                duration: t.duration,
                live: false,
                url,
            })
        }
        Item::Station(id) => {
            let s = lib.station(id)?;
            Some(Loaded {
                item,
                title: s.name.clone(),
                artist: s.country.clone(),
                album: String::new(),
                art: if s.favicon.starts_with("http") {
                    Art::Url(s.favicon.clone())
                } else {
                    Art::None
                },
                duration: 0.0,
                live: true,
                url: crate::radio::stream_url(&s),
            })
        }
        Item::Episode(id) => {
            let ep = lib.episode(id)?;
            let show = lib.podcast(ep.podcast);
            let image = if ep.image.is_empty() {
                show.as_ref().map(|s| s.image.clone()).unwrap_or_default()
            } else {
                ep.image.clone()
            };
            let url = if !ep.local.is_empty() && std::path::Path::new(&ep.local).is_file() {
                ep.local.clone()
            } else {
                ep.url.clone()
            };
            Some(Loaded {
                item,
                title: ep.title,
                artist: show.as_ref().map(|s| s.title.clone()).unwrap_or_default(),
                album: show.map(|s| s.author).unwrap_or_default(),
                art: if image.starts_with("http") {
                    Art::Url(image)
                } else {
                    Art::None
                },
                duration: ep.duration,
                live: false,
                url,
            })
        }
    }
}

/// The art for an artwork cache key.
pub fn art_for_key(lib: &Library, key: &str) -> Art {
    if key.is_empty() || lib.files.is_none() {
        Art::None
    } else {
        Art::File(key.to_string())
    }
}

/// `3:04`, `1:02:03`.
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as i64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_format() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(184.4), "3:04");
        assert_eq!(clock(3723.0), "1:02:03");
    }
}
