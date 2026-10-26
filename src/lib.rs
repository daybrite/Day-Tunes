//! Day Tunes, a [Day](https://daybrite.dev) music player: the listener's own files, internet
//! radio, and podcasts, in one library. `root()` sets the app up once and opens the first
//! window; `window_shell` is one window's UI, built again for every File ▸ New Window.
//!
//! The store, the radio index, the podcast directory, scrobbling and the transport are
//! app-wide ([`App`]); what a window is looking at is per window ([`Scene`]). A desktop window
//! is the three-pane shape (sources, list, detail) over a player bar; a phone is five tabs,
//! each a stack, with a mini player above the tab bar.

use day::prelude::*;
use day_piece_settings::prelude::*;

pub mod library;
pub mod pages;
pub mod podcasts;
pub mod radio;
pub mod scrobble;
pub mod transport;

use crate::library::Library;
use crate::podcasts::Podcasts;
use crate::radio::{Browse, Radio};
use crate::scrobble::Scrobbling;
use crate::transport::{Item, Transport};
use pages::{PodRoute, RadioRoute, ShelfRoute};

// The mobile / embedded entry point; a plain cargo desktop build enters through src/main.rs.
day::day_start!(options: window(), root);

/// The window every entry point opens: the generated options carry the locale catalog, the
/// localized title and the version; the size is a desktop default, mobile fills the screen.
pub fn window() -> day::WindowOptions {
    res::window_options().size(1180.0, 760.0)
}

// Typed constants for everything under `resource/` (https://daybrite.dev/docs/how-to/resources).
day::resources!();

/// The radio index server of the listener's choice (Settings); empty picks one.
pub static RADIO_SERVER: Pref<String> = Pref::new("radio.server", String::new);
/// The podcast directory's storefront.
pub static PODCAST_COUNTRY: Pref<String> = Pref::new("podcasts.country", || "us".to_string());
/// The launch-time library, for scripts: `TUNES_LIBRARY_IMPORT=asset:library-fixture`.
pub const LIBRARY_IMPORT_ENV: &str = "TUNES_LIBRARY_IMPORT";
/// Refresh subscriptions at launch.
pub static PODCAST_REFRESH_AT_LAUNCH: Pref<bool> = Pref::new("podcasts.refresh_at_launch", || true);

/// Whether this platform has a menu bar: the desktop shape.
pub(crate) fn has_menu_bar() -> bool {
    capability(Cap::AppMenu) != Support::Unsupported
}

/// Whether the window lays out as a phone: tabs, stacks, a mini player. That is a touch
/// platform's window that is narrow (a phone, an iPad in Slide Over) or short (a phone on its
/// side); a tablet's window gets the desktop's panes. Read untracked: a page that sizes itself
/// from this need not rebuild on a rotation, because the shell rebuilds when the answer changes.
pub(crate) fn is_phone() -> bool {
    untrack(phone_layout)
}

/// [`is_phone`], tracked: what the window shell switches on.
fn phone_layout() -> bool {
    !has_menu_bar()
        && day::size_class()
            .is_none_or(|c| c.width == WidthClass::Compact || c.height == HeightClass::Compact)
}

/// The app's sections. A desktop sidebar lists the first group; a phone's tabs the second.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Section {
    Songs,
    Albums,
    Artists,
    Genres,
    RecentlyAdded,
    History,
    Playlist(u64),
    Stations,
    RadioBrowse,
    Episodes,
    Shows,
    Discover,
    Queue,
    // Phone tabs.
    Shelf,
    Radio,
    Podcasts,
    Playing,
    Settings,
}

impl Section {
    /// The section that shows the same thing in the other shell, for a window that crosses
    /// between the phone's tabs and the desktop's panes (an iPad entering Slide Over).
    fn for_shell(&self, phone: bool) -> Section {
        match (phone, self) {
            (
                true,
                Self::Songs
                | Self::Albums
                | Self::Artists
                | Self::Genres
                | Self::RecentlyAdded
                | Self::History
                | Self::Playlist(_),
            ) => Self::Shelf,
            (true, Self::Stations | Self::RadioBrowse) => Self::Radio,
            (true, Self::Episodes | Self::Shows | Self::Discover) => Self::Podcasts,
            (true, Self::Queue) => Self::Playing,
            (false, Self::Shelf) if pages::has_local_library() => Self::Songs,
            (false, Self::Shelf | Self::Radio) => Self::Stations,
            (false, Self::Podcasts) => Self::Episodes,
            (false, Self::Playing) => Self::Queue,
            (_, s) => s.clone(),
        }
    }
}

impl Route for Section {
    fn key(&self) -> String {
        match self {
            Section::Songs => "songs".into(),
            Section::Albums => "albums".into(),
            Section::Artists => "artists".into(),
            Section::Genres => "genres".into(),
            Section::RecentlyAdded => "recent".into(),
            Section::History => "history".into(),
            Section::Playlist(id) => format!("playlist-{id}"),
            Section::Stations => "stations".into(),
            Section::RadioBrowse => "browse".into(),
            Section::Episodes => "episodes".into(),
            Section::Shows => "shows".into(),
            Section::Discover => "discover".into(),
            Section::Queue => "queue".into(),
            Section::Shelf => "library".into(),
            Section::Radio => "radio".into(),
            Section::Podcasts => "podcasts".into(),
            Section::Playing => "playing".into(),
            Section::Settings => "settings".into(),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "songs" => Section::Songs,
            "albums" => Section::Albums,
            "artists" => Section::Artists,
            "genres" => Section::Genres,
            "recent" => Section::RecentlyAdded,
            "history" => Section::History,
            "stations" => Section::Stations,
            "browse" => Section::RadioBrowse,
            "episodes" => Section::Episodes,
            "shows" => Section::Shows,
            "discover" => Section::Discover,
            "queue" => Section::Queue,
            "library" => Section::Shelf,
            "radio" => Section::Radio,
            "podcasts" => Section::Podcasts,
            "playing" => Section::Playing,
            "settings" => Section::Settings,
            other => Section::Playlist(other.strip_prefix("playlist-")?.parse().ok()?),
        })
    }
    fn title(&self) -> String {
        pages::section_title(self)
    }
}

/// What the detail pane shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Detail {
    Track(u64),
    Album(u64),
    Artist(u64),
    Genre(u64),
    /// A station, as the index or the store describes it (a browse result is not a row).
    Station(library::Station),
    Episode(u64),
    Show(u64),
    /// A directory result, previewed before subscribing.
    Preview(podcasts::DirectoryShow),
}

/// Everything the process owns: the store, the sources, the transport. One instance,
/// created on first use, visible from every window and menu command
/// (https://daybrite.dev/docs/explanation/state).
#[derive(Clone, Copy)]
pub struct App {
    /// Leaked once: the store lives as long as the process, and a `'static` reference is what
    /// lets `App` stay `Copy` and ride into every closure like a `Scene` does.
    pub library: &'static Library,
    pub radio: &'static Radio,
    pub podcasts: &'static Podcasts,
    pub scrobbling: &'static Scrobbling,
    pub transport: Transport,
}

impl Ambient for App {
    fn create() -> Self {
        let library = match Library::open() {
            Ok(c) => c,
            Err(e) => {
                // An unwritable data directory is not worth a crash: an in-memory store works
                // for the session and forgets at exit, and that fallback is logged as such.
                error!("could not open the store, running in memory: {e}");
                Library::in_memory()
            }
        };
        App {
            library: Box::leak(Box::new(library)),
            radio: Box::leak(Box::new(Radio::new(&RADIO_SERVER.get_untracked()))),
            podcasts: Box::leak(Box::new(Podcasts::new(&PODCAST_COUNTRY.get_untracked()))),
            scrobbling: Box::leak(Box::new(Scrobbling::new())),
            transport: Transport::new(),
        }
    }
}

/// Everything one window owns: where it is looking (https://daybrite.dev/docs/explanation/state).
#[derive(Clone, Copy)]
pub struct Scene {
    pub section: Signal<Section>,
    /// The detail pane's subject.
    pub detail: Signal<Option<Detail>>,
    /// Whether the detail is showing, on the shapes that show one pane at a time.
    pub detail_open: Signal<bool>,
    /// The list the detail's subject was picked from, as the queue a Play there starts.
    pub context: Signal<Vec<Item>>,
    /// The Songs list's order and filter.
    pub song_sort: Signal<library::TrackSort>,
    pub song_query: Signal<String>,
    /// The desktop radio browse: the cut and the pushed scope.
    pub radio_cut: Signal<usize>,
    pub radio_path: Signal<Vec<Browse>>,
    pub radio_query: Signal<String>,
    /// The podcast directory search field.
    pub podcast_query: Signal<String>,
    /// The phone tabs' stacks.
    pub shelf_path: Signal<Vec<ShelfRoute>>,
    /// The Now Playing tab's stack on a phone: a track opened from the queue.
    pub playing_path: Signal<Vec<ShelfRoute>>,
    pub radio_tab_path: Signal<Vec<RadioRoute>>,
    pub pod_path: Signal<Vec<PodRoute>>,
}

impl Ambient for Scene {
    fn create() -> Self {
        Scene {
            section: Signal::new(if is_phone() {
                Section::Shelf
            } else if pages::has_local_library() {
                Section::Songs
            } else {
                Section::Stations
            }),
            detail: Signal::new(None),
            detail_open: Signal::new(false),
            context: Signal::new(Vec::new()),
            song_sort: Signal::new(library::TrackSort::Title),
            song_query: Signal::new(String::new()),
            radio_cut: Signal::new(0),
            radio_path: Signal::new(Vec::new()),
            radio_query: Signal::new(String::new()),
            podcast_query: Signal::new(String::new()),
            shelf_path: Signal::new(Vec::new()),
            playing_path: Signal::new(Vec::new()),
            radio_tab_path: Signal::new(Vec::new()),
            pod_path: Signal::new(Vec::new()),
        }
    }
}

impl Scene {
    /// Show `detail`: beside the list on a wide window, pushed over it on a narrow one.
    /// `context` is the list it was picked from.
    pub fn open(self, detail: Detail, context: Vec<Item>) {
        self.context.set(context);
        self.detail.set(Some(detail));
        self.detail_open.set(true);
    }

    pub fn close_detail(self) {
        self.detail.set(None);
        self.detail_open.set(false);
    }
}

/// App startup: everything that happens once, however many windows open, and then the first
/// window's content.
pub fn root() -> impl Piece {
    info!("Day Tunes starting");
    day::prefs::install_nav_store();
    let app = App::app();
    app.transport.install();
    // Settings and About (https://daybrite.dev/docs/reference/settings).
    let prefs = Preferences::new()
        .about(
            About::from_manifest()
                .tagline(res::str::about_tagline())
                .link(res::str::day_about_website(), "https://daybrite.dev")
                .source("https://github.com/daybrite/Day-Tunes"),
        )
        .group(general_group(res::locales::ALL))
        .group(pages::library_group(app))
        .group(pages::radio_group(app))
        .group(pages::podcasts_group(app))
        .group(pages::scrobbling_group(app))
        .install();
    RADIO_SERVER.watch(move |server| app.radio.set_server(server));
    PODCAST_COUNTRY.watch(move |country| app.podcasts.set_country(country));
    day::register_new_window({
        let prefs = prefs.clone();
        move || window_shell(false, prefs.clone())
    });
    if has_menu_bar() {
        app_menu(move || menus(app));
    }
    // The chart and the subscriptions, a moment after the first frame; a scripted run also
    // imports its bundled library here (`TUNES_LIBRARY_IMPORT=asset:library-fixture`).
    day::task(async move {
        day::sleep(400).await;
        if let Some(dir) = day::env(LIBRARY_IMPORT_ENV)
            .filter(|s| !s.is_empty())
            .and_then(|s| s.strip_prefix("asset:").map(|s| s.to_string()))
            && app.library.track_count.get_untracked() == 0
        {
            app.library.import_asset_dir(&dir);
        }
        app.podcasts.load_top();
        if PODCAST_REFRESH_AT_LAUNCH.get_untracked() {
            app.podcasts.refresh_all();
        }
        app.scrobbling.flush();
    });
    window_shell(true, prefs)
}

/// One window's UI: the first window's, and every File ▸ New Window's.
fn window_shell(primary: bool, prefs: Prefs) -> impl Piece {
    Scene::scoped(move |scene| {
        let app = App::app();
        // A section change starts from its list.
        watch(
            move || scene.section.get(),
            move |_, _| scene.close_detail(),
        );
        watch(
            move || scene.detail_open.get(),
            move |open, _| {
                if !open {
                    scene.detail.set(None);
                }
            },
        );
        watch(
            move || scene.radio_cut.get(),
            move |_, _| {
                if !scene.radio_path.get_untracked().is_empty() {
                    scene.radio_path.set(Vec::new());
                }
            },
        );
        day::window_title(move || match app.transport.loaded.get() {
            Some(l) if app.transport.is_active() => {
                if l.artist.is_empty() {
                    l.title
                } else {
                    format!("{} · {}", l.title, l.artist)
                }
            }
            _ => res::str::app_title().format(),
        });
        switch(phone_layout, move |&phone| {
            let section = scene.section.get_untracked();
            let moved = section.for_shell(phone);
            if moved != section {
                scene.section.set(moved);
            }
            if phone {
                pages::phone_shell(scene, prefs.clone(), primary).any()
            } else {
                pages::desktop_shell(scene, prefs.clone(), primary).any()
            }
        })
    })
}

/// Run a command on the window that currently has focus
/// (https://daybrite.dev/docs/explanation/state).
fn front(f: impl Fn(Scene) + 'static) -> impl Fn() + 'static {
    move || {
        if let Some(scene) = Scene::focused() {
            f(scene)
        }
    }
}

/// The desktop menu bar; mobile toolkits ignore it (https://daybrite.dev/docs/reference/menus).
fn menus(app: App) -> Vec<MenuEntry> {
    let t = app.transport;
    let active = t.is_active();
    let has_item = t.loaded.with(|l| l.is_some());
    vec![
        sub_menu(
            res::str::menu_file(),
            vec![
                menu_role(MenuRole::NewWindow),
                menu_separator(),
                menu_item(res::str::cmd_add_folder())
                    .id(day::id!(add_folder))
                    .enabled(capability(Cap::FolderDialog) != Support::Unsupported)
                    .action(move || pages::add_folder(app)),
                menu_item(res::str::cmd_import_files())
                    .id(day::id!(import_files))
                    .action(move || pages::import_files(app)),
                menu_separator(),
                menu_item(res::str::cmd_new_playlist())
                    .id(day::id!(new_playlist))
                    .action(front(move |scene| pages::new_playlist(app, scene))),
                menu_item(res::str::cmd_add_podcast())
                    .id(day::id!(add_podcast))
                    .action(front(move |scene| pages::add_podcast_by_url(app, scene))),
                menu_item(res::str::cmd_refresh_podcasts())
                    .id(day::id!(refresh_podcasts))
                    .action(move || app.podcasts.refresh_all()),
                menu_separator(),
                menu_role(MenuRole::CloseWindow),
            ],
        ),
        sub_menu(
            res::str::menu_edit(),
            vec![
                menu_role(MenuRole::Cut),
                menu_role(MenuRole::Copy),
                menu_role(MenuRole::Paste),
                menu_role(MenuRole::SelectAll),
            ],
        ),
        sub_menu(
            res::str::menu_controls(),
            vec![
                menu_item(if active {
                    res::str::cmd_pause()
                } else {
                    res::str::cmd_play()
                })
                .id(day::id!(play_pause))
                .shortcut(Shortcut::primary("p"))
                .enabled(has_item)
                .action(move || t.toggle()),
                menu_item(res::str::cmd_stop())
                    .id(day::id!(menu_stop))
                    .shortcut(Shortcut::primary("."))
                    .enabled(has_item)
                    .action(move || t.stop()),
                menu_separator(),
                menu_item(res::str::cmd_next())
                    .id(day::id!(menu_next))
                    .shortcut(Shortcut::primary("]"))
                    .enabled(t.has_next())
                    .action(move || t.next()),
                menu_item(res::str::cmd_previous())
                    .id(day::id!(menu_previous))
                    .shortcut(Shortcut::primary("["))
                    .enabled(has_item)
                    .action(move || t.previous()),
                menu_separator(),
                menu_item(res::str::cmd_shuffle())
                    .id(day::id!(menu_shuffle))
                    .checked(t.shuffle.get())
                    .action(move || t.set_shuffle(!t.shuffle.get_untracked())),
                menu_item(res::str::cmd_repeat())
                    .id(day::id!(menu_repeat))
                    .checked(t.repeat.get() != transport::Repeat::Off)
                    .action(move || t.cycle_repeat()),
                menu_separator(),
                menu_item(res::str::cmd_volume_up())
                    .shortcut(Shortcut::primary("ArrowUp"))
                    .action(move || t.volume.update(|v| *v = (*v + 0.1).min(1.0))),
                menu_item(res::str::cmd_volume_down())
                    .shortcut(Shortcut::primary("ArrowDown"))
                    .action(move || t.volume.update(|v| *v = (*v - 0.1).max(0.0))),
            ],
        ),
        sub_menu(
            res::str::menu_view(),
            vec![
                menu_item(res::str::nav_songs())
                    .shortcut(Shortcut::primary("1"))
                    .action(front(|s| s.section.set(Section::Songs))),
                menu_item(res::str::nav_albums())
                    .shortcut(Shortcut::primary("2"))
                    .action(front(|s| s.section.set(Section::Albums))),
                menu_item(res::str::nav_artists())
                    .shortcut(Shortcut::primary("3"))
                    .action(front(|s| s.section.set(Section::Artists))),
                menu_item(res::str::nav_stations())
                    .shortcut(Shortcut::primary("4"))
                    .action(front(|s| s.section.set(Section::Stations))),
                menu_item(res::str::nav_episodes())
                    .shortcut(Shortcut::primary("5"))
                    .action(front(|s| s.section.set(Section::Episodes))),
                menu_item(res::str::nav_queue())
                    .shortcut(Shortcut::primary("u"))
                    .action(front(|s| s.section.set(Section::Queue))),
                menu_separator(),
                menu_item(res::str::cmd_find())
                    .shortcut(Shortcut::primary("f"))
                    .action(day::focus_search),
            ],
        ),
    ]
}
