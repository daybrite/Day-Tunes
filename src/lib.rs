//! Day Tunes, a [Day](https://daybrite.dev) internet-radio app. `root()` sets the app up once
//! and opens the first window; `window_shell` is one window's UI, shared by every platform and
//! by File ▸ New Window. The station catalog, its sync, and the player are app-wide ([`App`]);
//! what a window is looking at is per window ([`Scene`]).
//!
//! The catalog is the Tune Out project's published `stations.sqlite`, kept as a local copy
//! (src/sync.rs) and read in place (src/catalog.rs). The listener's own rows live beside it.

use day::prelude::*;
use day_piece_media::{PlaybackState, StreamMetadata, media};

mod catalog;
mod pages;
mod sync;
use crate::catalog::{Scope, Station};
use crate::pages::*;
use crate::sync::Sync;

// The mobile / embedded entry point; a plain cargo desktop build enters through src/main.rs.
day::day_start!(options: window(), root);

/// The window every entry point opens. The locale catalog and title are handed to `launch`
/// rather than installed here (https://daybrite.dev/docs/localization).
pub fn window() -> day::WindowOptions {
    day::WindowOptions {
        locales: Some((res::locales::DEFAULT, res::locales::CATALOG)),
        title_fn: Some(|| res::str::app_title().format()),
        // A desktop default; mobile fills the screen regardless.
        size: day::prelude::Size::new(1100.0, 700.0),
        ..Default::default()
    }
}

// Typed constants for everything under `resource/` (https://daybrite.dev/docs/resources).
day::resources!();

/// The settings' `day::prefs` keys: read at startup, written by the Settings page.
const THEME_KEY: &str = "app.theme";
const LOCALE_KEY: &str = "app.locale";
/// The volume slider's `day::prefs` key.
const VOLUME_KEY: &str = "app.volume";

day::routes! {
    /// The app's sections, typed (https://daybrite.dev/docs/navigation). Now Playing sits in
    /// the middle of the tab bar, the way a music app's transport does.
    pub(crate) enum Section {
        Browse => "browse",
        Search => "search",
        Playing => "playing",
        Library => "library",
        Settings => "settings",
    }
}

/// Whether this platform has a menu bar; there, Settings lives in the App menu instead of the
/// navigation (https://daybrite.dev/docs/menus).
pub(crate) fn has_menu_bar() -> bool {
    capability(Cap::AppMenu) != Support::Unsupported
}

/// Whether this platform has window-level chrome that persists across pages
/// (https://daybrite.dev/docs/toolbars), which decides whether the transport belongs there or
/// in the Now Playing page. It no longer decides whether a command can be shown: a toolbar item
/// declared on a page rides that page's chrome on every platform.
pub(crate) fn has_toolbar() -> bool {
    capability(Cap::Toolbar) != Support::Unsupported
}

/// The one player, shared by every window: which station is on, what the native player is
/// doing, what the stream says it is playing, and the triggers that drive it. `Copy`, because
/// every field is a handle.
#[derive(Clone, Copy)]
pub(crate) struct Player {
    /// The station on air, or the last one asked for: a copy of its catalog row, so the
    /// transport and the window title need no lookup, and keep their station through a
    /// catalog update, whose rowids owe the old ones nothing.
    pub current: Signal<Option<Station>>,
    /// The stream the media piece is bound to; `load` re-reads it.
    pub url: Signal<String>,
    /// What the native player reports (https://daybrite.dev/docs/media).
    pub state: Signal<PlaybackState>,
    /// What the stream says it is playing: the ICY `StreamTitle` or HLS ID3, parsed into
    /// title / artist / album (https://daybrite.dev/docs/media). `None` until it says.
    pub track: Signal<Option<StreamMetadata>>,
    /// The stations around the one on air: the list it was started from, by uuid, so
    /// Previous and Next step through that list (the genre, the search, the favorites).
    pub queue: Signal<Vec<Vec<u8>>>,
    /// `0.0..=1.0`, persisted.
    pub volume: Signal<f64>,
    pub play: Trigger,
    pub pause: Trigger,
    pub stop: Trigger,
    pub load: Trigger,
}

impl Player {
    fn new() -> Player {
        let volume = day::prefs::get(VOLUME_KEY)
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(0.85, |v| v.clamp(0.0, 1.0));
        Player {
            current: Signal::new(None),
            url: Signal::new(String::new()),
            state: Signal::new(PlaybackState::Idle),
            track: Signal::new(None),
            queue: Signal::new(Vec::new()),
            volume: Signal::new(volume),
            play: Trigger::new(),
            pause: Trigger::new(),
            stop: Trigger::new(),
            load: Trigger::new(),
        }
    }

    /// Start `station` from `url`, the one path every "play this" command takes. `queue` is
    /// the list the station was picked from (empty keeps the current one), for Previous and
    /// Next.
    pub fn play_station(self, station: Station, url: String, queue: Vec<Vec<u8>>) {
        info!(
            "playing {} ({})",
            station.name,
            catalog::uuid_text(&station.uuid)
        );
        App::app().catalog.touch_recent(&station.uuid);
        if !queue.is_empty() {
            self.queue.set(queue);
        }
        self.current.set(Some(station));
        self.url.set(url);
        // The piece reads `url` when the trigger fires, so the order above matters.
        self.load.notify();
    }

    /// Play/pause: what the transport button and the menu item do. A stopped or failed player
    /// starts its station over rather than resuming nothing.
    pub fn toggle(self) {
        match self.state.get_untracked() {
            PlaybackState::Playing | PlaybackState::Loading => self.pause.notify(),
            PlaybackState::Paused => self.play.notify(),
            PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error(_) => {
                if self.current.get_untracked().is_some() && !self.url.get_untracked().is_empty() {
                    self.load.notify();
                }
            }
        }
    }

    /// Where the station on air sits in the queue (tracked).
    fn position(self) -> Option<usize> {
        let uuid = self.current_uuid()?;
        self.queue.with(|q| q.iter().position(|u| *u == uuid))
    }

    pub fn has_previous(self) -> bool {
        self.position().is_some_and(|i| i > 0)
    }

    pub fn has_next(self) -> bool {
        let len = self.queue.with(|q| q.len());
        self.position().is_some_and(|i| i + 1 < len)
    }

    /// Move `delta` stations along the queue (−1 previous, +1 next) and play the one there.
    pub fn step(self, delta: isize) {
        let app = App::app();
        let Some(pos) = untrack(|| self.position()) else {
            return;
        };
        let target = pos as isize + delta;
        if target < 0 {
            return;
        }
        let Some(uuid) = self
            .queue
            .with_untracked(|q| q.get(target as usize).cloned())
        else {
            return;
        };
        if let Some(s) = app.catalog.station_by_uuid(&uuid) {
            let url = stream_url(&s);
            self.play_station(s, url, Vec::new());
        }
    }

    pub fn is_on_air(self) -> bool {
        self.state.get().is_active()
    }

    /// The uuid of the station on air (tracked).
    pub fn current_uuid(self) -> Option<Vec<u8>> {
        self.current.with(|s| s.as_ref().map(|s| s.uuid.clone()))
    }

    /// Whether `uuid` names the station on air (tracked).
    pub fn is_current(self, uuid: &[u8]) -> bool {
        self.current
            .with(|s| s.as_ref().is_some_and(|s| s.uuid == uuid))
    }

    /// Whether the station on air is a favorite (tracked).
    pub fn is_favorite(self) -> bool {
        self.current_uuid()
            .is_some_and(|u| App::app().catalog.is_favorite(&u))
    }

    /// Favorite or unfavorite the station on air.
    pub fn toggle_favorite(self) {
        if let Some(uuid) = self
            .current
            .with_untracked(|s| s.as_ref().map(|s| s.uuid.clone()))
        {
            App::app().catalog.toggle_favorite(&uuid);
        }
    }
}

/// Everything the process owns: the store, the catalog sync, and the player. One instance,
/// created on first use, visible from every window and menu command
/// (https://daybrite.dev/docs/state).
#[derive(Clone, Copy)]
pub(crate) struct App {
    /// Leaked once: the store lives as long as the process, and a `'static` reference is what
    /// lets `App` stay `Copy` and ride into every closure like a `Scene` does.
    pub catalog: &'static catalog::Catalog,
    pub sync: Sync,
    pub player: Player,
}

impl Ambient for App {
    fn create() -> Self {
        let catalog = match catalog::Catalog::open() {
            Ok(c) => c,
            Err(e) => {
                // An unwritable data directory is not worth a crash: an in-memory store reads
                // the same catalog and forgets favorites at exit, and that fallback is logged
                // as such.
                error!("could not open the store, running in memory: {e}");
                catalog::Catalog::in_memory()
            }
        };
        App {
            catalog: Box::leak(Box::new(catalog)),
            sync: Sync::new(),
            player: Player::new(),
        }
    }
}

/// Everything one window owns: where it is looking (https://daybrite.dev/docs/state).
#[derive(Clone, Copy)]
pub(crate) struct Scene {
    /// Which section the navigation is showing.
    pub section: Signal<Section>,
    /// The station the detail pane shows, by its rowid in the attached catalog.
    pub selected: Signal<Option<u64>>,
    /// The list the selected station was picked from, by uuid: what the player steps
    /// through once that station plays.
    pub queue: Signal<Vec<Vec<u8>>>,
    /// Whether the detail is showing, on the shapes that show one pane at a time
    /// (`nav(…).detail_visible` in `window_shell`).
    pub detail_open: Signal<bool>,
    /// The Browse pane's cut: 0 top, 1 countries, 2 genres, 3 languages.
    pub browse_cut: Signal<usize>,
    /// The Browse drill-down: empty at the cut's own list, one scope deep on a country, genre,
    /// or language: a real push, with the platform's back (https://daybrite.dev/docs/navigation).
    pub browse_path: Signal<Vec<Scope>>,
    /// The Search pane's text.
    pub query: Signal<String>,
    /// The Library pane's cut: 0 favorites, 1 recents.
    pub library_cut: Signal<usize>,
    /// The Library's filter by the listener's own tag: 0 is every row, `n` the n-th tag.
    pub library_tag: Signal<usize>,
    /// The tag being typed on a station page.
    pub new_tag: Signal<String>,
}

impl Ambient for Scene {
    fn create() -> Self {
        Scene {
            section: Signal::new(Section::Browse),
            selected: Signal::new(None),
            queue: Signal::new(Vec::new()),
            detail_open: Signal::new(false),
            browse_cut: Signal::new(0),
            browse_path: Signal::new(Vec::new()),
            query: Signal::new(String::new()),
            library_cut: Signal::new(0),
            library_tag: Signal::new(0),
            new_tag: Signal::new(String::new()),
        }
    }
}

impl Scene {
    /// Show `station` in the detail pane: beside the list on a wide window, pushed over it on
    /// a narrow one; the host decides which (https://daybrite.dev/docs/navigation). `queue` is
    /// the list it was picked from.
    pub fn open_from(self, station: u64, queue: Vec<Vec<u8>>) {
        self.queue.set(queue);
        self.selected.set(Some(station));
        self.detail_open.set(true);
    }

    pub fn clear_selection(self) {
        self.selected.set(None);
    }

    /// Jump to a station's page from anywhere (the now-playing page's "Open station").
    pub fn show(self, station: u64) {
        self.section.set(Section::Library);
        self.open_from(station, Vec::new());
    }
}

/// App startup: everything that happens once, however many windows open, and then the first
/// window's content.
pub fn root() -> impl Piece {
    // `info!` and friends need no setup: Day installs a logger at launch.
    info!("Day Tunes starting");
    // Re-apply the saved theme and language before anything is built.
    day_piece_settings::apply_startup(THEME_KEY, LOCALE_KEY);
    // Where `.restore("app.section")` keeps the last section between launches.
    day::prefs::install_nav_store();

    // A real Settings window plus the App ▸ Settings… item on desktop; a fullscreen cover
    // where windows are unsupported (https://daybrite.dev/docs/windows).
    day::register_preferences(settings_body);
    // File ▸ New Window (⌘N / Ctrl+N) and the macOS tab-bar "+": the same shell as the first
    // window. The player is app-wide, so a second window is another remote for the same
    // stream (https://daybrite.dev/docs/state).
    day::register_new_window(|| window_shell(false));
    app_menu(menus());

    let app = App::app();
    // The volume outlives every window, so its persistence is installed once, here.
    let player = app.player;
    watch(
        move || player.volume.get(),
        |v, _| {
            day::prefs::set(VOLUME_KEY, &format!("{v:.3}"));
        },
    );
    // The catalog: the copy on disk first, so the lists fill at once, then the publisher's
    // manifest, and a download when it names a newer file (src/sync.rs).
    // Let the host finish mounting its first frame before attaching the catalog and
    // populating the lists. This also keeps mobile startup watchdogs out of catalog work.
    day::task(async move {
        day::sleep(1).await;
        app.sync.attach_installed(app.catalog);
        app.sync.check(app.catalog);
    });

    window_shell(true)
}

/// One window's UI: the first window's, and every File ▸ New Window's.
///
/// `primary` marks the window that owns the route namespace and carries the player itself:
/// the media piece lives in exactly one tree, and closing that window ends playback.
fn window_shell(primary: bool) -> impl Piece {
    Scene::scoped(move |scene| {
        let app = App::app();
        let player = app.player;
        // A section change starts from its list: the station page belongs to the list it was
        // opened from, and a favorite's pane should not open on a search result.
        watch(
            move || scene.section.get(),
            move |_, _| {
                scene.clear_selection();
                scene.detail_open.set(false);
            },
        );
        // Closing the station page drops the selection with it: the row un-highlights, and where
        // the page sits beside the list, the back that closed it leaves the empty state behind.
        watch(
            move || scene.detail_open.get(),
            move |open, _| {
                if !open {
                    scene.clear_selection();
                }
            },
        );
        // A cut change starts from the cut's own list: the drill-down belonged to the last one.
        watch(
            move || scene.browse_cut.get(),
            move |_, _| {
                if !scene.browse_path.get_untracked().is_empty() {
                    scene.browse_path.set(Vec::new());
                }
            },
        );
        // A catalog update re-keys every station: whatever page was open belongs to the old
        // file, so it closes rather than showing a stranger under the same rowid.
        watch(
            move || app.catalog.generation.get(),
            move |_, _| scene.clear_selection(),
        );
        // Name the window after what it plays (https://daybrite.dev/docs/windows).
        day::window_title(move || {
            match player.current.with(|s| s.as_ref().map(|s| s.name.clone())) {
                Some(name) if player.is_on_air() => name,
                _ => res::str::app_title().format(),
            }
        });
        // The transport belongs to the window: it is playing whatever page is showing, so it
        // rides the window's chrome rather than any page's
        // (https://daybrite.dev/docs/toolbars). The Browse cut is not here; it belongs to the
        // Browse list's page, which is where it is now declared (pages.rs).
        let transport = move || {
            if !has_menu_bar() {
                return Vec::new();
            }
            let on_air = player.is_on_air();
            let has_station = player.current_uuid().is_some();
            let fav = player.is_favorite();
            vec![
                toolbar_label("tb-now", move || now_playing_line(app))
                    .placement(ToolbarPlacement::Principal),
                toolbar_button("tb-previous", res::str::playing_previous())
                    .image(res::vectors::skip_previous)
                    .tooltip(res::str::playing_previous())
                    .enabled(player.has_previous())
                    .action(move || player.step(-1)),
                toolbar_button(
                    "tb-play",
                    if on_air {
                        res::str::cmd_pause()
                    } else {
                        res::str::cmd_play()
                    },
                )
                .icon(if on_air { Symbol::Pause } else { Symbol::Play })
                .tooltip(if on_air {
                    res::str::cmd_pause()
                } else {
                    res::str::cmd_play()
                })
                .enabled(has_station)
                .action(move || player.toggle()),
                toolbar_button("tb-next", res::str::playing_next())
                    .image(res::vectors::skip_next)
                    .tooltip(res::str::playing_next())
                    .enabled(player.has_next())
                    .action(move || player.step(1)),
                toolbar_button("tb-stop", res::str::cmd_stop())
                    .icon(Symbol::Stop)
                    .tooltip(res::str::cmd_stop())
                    .enabled(has_station)
                    .action(move || player.stop.notify()),
                toolbar_button(
                    "tb-favorite",
                    if fav {
                        res::str::cmd_unfavorite()
                    } else {
                        res::str::cmd_favorite()
                    },
                )
                .icon(if fav { Symbol::Star } else { Symbol::Bookmark })
                .tooltip(if fav {
                    res::str::cmd_unfavorite()
                } else {
                    res::str::cmd_favorite()
                })
                .enabled(has_station)
                .action(move || player.toggle_favorite()),
            ]
        };

        // A nav is adaptive by default: tabs on a phone, a rail on a tablet, a sidebar on a
        // desktop (https://daybrite.dev/docs/navigation).
        let nav = nav(scene.section)
            .title(res::str::app_title())
            .toolbar(transport)
            // The station list as a real content-list pane: its own column where the toolkit
            // has one, the pushed middle layer on a phone.
            .content_list(content_pane)
            .content_list_width(360.0)
            // Now Playing and Settings keep the whole detail area.
            .content_list_for(|s: &Section| {
                matches!(s, Section::Browse | Section::Search | Section::Library)
            })
            .detail_visible(scene.detail_open)
            .detail_title(move || detail_title(scene))
            .item_icon(
                Section::Browse,
                res::str::nav_browse(),
                res::vectors::nav_browse,
                station_page,
            )
            .icon_tint(Color::hex(0x3B82F6))
            .item_icon(
                Section::Search,
                res::str::nav_search(),
                res::vectors::nav_search,
                station_page,
            )
            .icon_tint(Color::hex(0x8B5CF6))
            .item_icon(
                Section::Playing,
                res::str::nav_playing(),
                res::vectors::nav_playing,
                playing_page,
            )
            .icon_tint(Color::hex(0x10B981))
            .item_icon(
                Section::Library,
                res::str::nav_library(),
                res::vectors::nav_library,
                station_page,
            )
            .icon_tint(Color::hex(0xEF4444))
            // Settings is a row only where there is no menu bar (see `has_menu_bar`).
            .items(
                move || {
                    if has_menu_bar() {
                        Vec::new()
                    } else {
                        vec![Section::Settings]
                    }
                },
                |s: &Section| {
                    item(*s, res::str::nav_settings())
                        .icon(res::vectors::tab_settings)
                        .icon_tint(Color::hex(0x6B7280))
                },
            )
            .destination(|_: &Section| settings_page())
            .id("nav");
        // Only the first window joins the route namespace and restores where it left off.
        let nav = if primary {
            nav.restore("app.section")
        } else {
            nav.local()
        };
        if primary {
            // The player itself: sound only, no size, driven by the app's own transport
            // (https://daybrite.dev/docs/media). It sits under the navigation in the first
            // window's tree, so it outlives every page change.
            column((
                nav.grow(),
                media(player.url)
                    .audio_only(true)
                    .autoplay(false)
                    .controls(false)
                    .volume(player.volume)
                    .play(player.play)
                    .pause(player.pause)
                    .stop(player.stop)
                    .load(player.load)
                    .state(player.state)
                    .metadata(player.track)
                    .id("player"),
            ))
            .any()
        } else {
            nav.any()
        }
    })
}

/// The pushed detail's navigation-bar title: the station it shows, else the section's name.
fn detail_title(scene: Scene) -> String {
    let app = App::app();
    match scene.selected.get().and_then(|id| app.catalog.station(id)) {
        Some(s) => s.name,
        None => match scene.section.get() {
            Section::Search => res::str::nav_search().format(),
            Section::Library => res::str::nav_library().format(),
            _ => res::str::nav_browse().format(),
        },
    }
}

/// The toolbar's readout: the station on air, the track when the stream names one, and what
/// the player says about it.
fn now_playing_line(app: App) -> String {
    let player = app.player;
    let Some(name) = player.current.with(|s| s.as_ref().map(|s| s.name.clone())) else {
        return String::new();
    };
    match player.state.get() {
        PlaybackState::Loading => {
            format!("{name} · {}", res::str::playing_state_loading().format())
        }
        PlaybackState::Error(_) => format!("{name} · {}", res::str::station_offline().format()),
        _ => match player.track.get() {
            Some(t) if !t.raw.is_empty() => format!("{name} · {}", t.raw),
            _ => name,
        },
    }
}

/// Run a command on the window that currently has FOCUS.
///
/// A desktop menu bar is one bar for the whole app, so its items belong to no window and cannot
/// capture a `Scene`; they resolve the front one when they run (https://daybrite.dev/docs/state).
#[allow(dead_code)]
fn front(f: impl Fn(Scene) + 'static) -> impl Fn() + 'static {
    move || {
        if let Some(scene) = Scene::focused() {
            f(scene)
        }
    }
}

/// The desktop menu bar; mobile toolkits ignore it (https://daybrite.dev/docs/menus).
fn menus() -> Vec<MenuEntry> {
    let app = App::app();
    let player = app.player;
    vec![
        sub_menu(
            res::str::menu_file().format(),
            vec![
                menu_role(MenuRole::NewWindow),
                menu_separator(),
                menu_item(res::str::catalog_check().format())
                    .action(move || app.sync.check(app.catalog)),
                menu_separator(),
                menu_role(MenuRole::CloseWindow),
            ],
        ),
        sub_menu(
            res::str::menu_edit().format(),
            vec![
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
        // The player is app-wide, so these need no window: whichever window is in front,
        // there is one stream.
        sub_menu(
            res::str::menu_playback().format(),
            vec![
                menu_item(res::str::cmd_play().format())
                    .shortcut(Shortcut::new("p"))
                    .action(move || player.toggle()),
                menu_item(res::str::cmd_stop().format())
                    .shortcut(Shortcut::new("."))
                    .action(move || player.stop.notify()),
                menu_separator(),
                menu_item(res::str::playing_previous().format())
                    .shortcut(Shortcut::new("["))
                    .action(move || player.step(-1)),
                menu_item(res::str::playing_next().format())
                    .shortcut(Shortcut::new("]"))
                    .action(move || player.step(1)),
                menu_separator(),
                menu_item(res::str::cmd_favorite().format())
                    .shortcut(Shortcut::new("d"))
                    .action(move || player.toggle_favorite()),
            ],
        ),
    ]
}
