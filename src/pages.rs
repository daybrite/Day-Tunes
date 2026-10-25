//! Every surface: the desktop three-pane shell and the phone tabs, the lists each section
//! fills, the pages a row opens, the player bar and the mini player, the Now Playing page,
//! and the Settings groups.
//!
//! Nothing here is a global. Every page function starts by asking the environment for the
//! window it is being built into (`Scene::ambient()`, https://daybrite.dev/docs/explanation/state)
//! and the process-wide `App::app()`, and passes both down. Lists are shared by both shells:
//! a [`Place`] says what opening a row means (a detail pane on a desktop, a pushed page on a
//! phone).

use crate::library::{
    self, Episode, EpisodeFields, FolderFields, Group, HistoryFields, Library, Playlist,
    PlaylistEntryFields, PlaylistFields, Podcast, PodcastFields, Station, Track, TrackFields,
    TrackSort,
};
use crate::podcasts::DirectoryShow;
use crate::radio::{Browse, FacetKind, Fetched, flag, stream_url, tag_list};
use crate::transport::{Art, Item, Repeat, Transport, clock, describe};
use crate::{App, Detail, Scene, Section, is_phone, res};
use day::prelude::*;
use day_persistence::Query;
use day_piece_media::PlaybackState;
use day_piece_settings::prelude::*;
use std::rc::Rc;
use std::sync::Arc;

/// The accent the player and the chips share.
const ACCENT: Color = Color {
    r: 0.95,
    g: 0.36,
    b: 0.22,
    a: 1.0,
};
const GREY: Color = Color {
    r: 0.56,
    g: 0.56,
    b: 0.58,
    a: 1.0,
};

fn tinted(c: Color, a: f64) -> Color {
    Color { a, ..c }
}

// --- titles ---------------------------------------------------------------------------------------

pub fn section_title(s: &Section) -> String {
    match s {
        Section::Songs => res::str::nav_songs().format(),
        Section::Albums => res::str::nav_albums().format(),
        Section::Artists => res::str::nav_artists().format(),
        Section::Genres => res::str::nav_genres().format(),
        Section::RecentlyAdded => res::str::nav_recently_added().format(),
        Section::History => res::str::nav_history().format(),
        Section::Playlist(id) => App::app()
            .library
            .playlist(*id)
            .map(|p| playlist_name(&p.name))
            .unwrap_or_else(|| res::str::nav_playlists().format()),
        Section::Stations => res::str::nav_stations().format(),
        Section::RadioBrowse => res::str::nav_browse().format(),
        Section::Episodes => res::str::nav_episodes().format(),
        Section::Shows => res::str::nav_shows().format(),
        Section::Discover => res::str::nav_discover().format(),
        Section::Queue => res::str::nav_queue().format(),
        Section::Shelf => res::str::nav_library().format(),
        Section::Radio => res::str::nav_radio().format(),
        Section::Podcasts => res::str::nav_podcasts().format(),
        Section::Playing => res::str::nav_playing().format(),
        Section::Settings => res::str::nav_settings().format(),
    }
}

pub fn browse_title(b: &Browse) -> String {
    let app = App::app();
    match b {
        Browse::Top => res::str::radio_top().format(),
        Browse::Trending => res::str::radio_trending().format(),
        Browse::Country(code) => {
            let name = app.radio.country_name(code).unwrap_or_else(|| code.clone());
            format!("{} {}", flag(code), short_country(&name))
                .trim()
                .to_string()
        }
        Browse::Tag(t) => title_case(t),
        Browse::Language(l) => title_case(l),
        Browse::Search(q) => res::str::radio_search_title(q.clone()).format(),
    }
}

fn playlist_name(name: &str) -> String {
    if name.trim().is_empty() {
        res::str::playlist_untitled().format()
    } else {
        name.to_string()
    }
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The index spells countries out in full ("The United States Of America"); a row has room
/// for the everyday name.
pub(crate) fn short_country(name: &str) -> String {
    let n = name.strip_prefix("The ").unwrap_or(name);
    match n {
        "United States Of America" => "United States",
        "United Kingdom Of Great Britain And Northern Ireland" => "United Kingdom",
        "Russian Federation" => "Russia",
        "Islamic Republic Of Iran" => "Iran",
        "Republic Of Korea" => "South Korea",
        "Taiwan, Republic Of China" => "Taiwan",
        "Viet Nam" => "Vietnam",
        "Bolivarian Republic Of Venezuela" => "Venezuela",
        "Plurinational State Of Bolivia" => "Bolivia",
        "United Republic Of Tanzania" => "Tanzania",
        "Syrian Arab Republic" => "Syria",
        "Republic Of Moldova" => "Moldova",
        "Democratic Republic Of The Congo" => "DR Congo",
        "Kingdom Of The Netherlands" => "Netherlands",
        other => other,
    }
    .to_string()
}

fn artist_or_unknown(s: &str) -> String {
    if s.trim().is_empty() {
        res::str::unknown_artist().format()
    } else {
        s.to_string()
    }
}

fn album_or_unknown(s: &str) -> String {
    if s.trim().is_empty() {
        res::str::unknown_album().format()
    } else {
        s.to_string()
    }
}

/// `53,790`: a count with thousands grouped.
fn grouped(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// `MP3 · 128 kbps`.
fn stream_line(codec: &str, bitrate: i64, hls: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if hls {
        parts.push("HLS".into());
    }
    if !codec.is_empty() && codec != "UNKNOWN" {
        parts.push(codec.to_string());
    }
    if bitrate > 0 {
        parts.push(res::str::bitrate(bitrate).format());
    }
    parts.join(" · ")
}

/// `🇫🇷 France · MP3 · 128 kbps · jazz, blues`.
fn station_meta(s: &Station) -> String {
    let mut parts = Vec::new();
    if !s.country.is_empty() {
        parts.push(
            format!("{} {}", flag(&s.countrycode), short_country(&s.country))
                .trim()
                .to_string(),
        );
    }
    let stream = stream_line(&s.codec, s.bitrate, s.hls);
    if !stream.is_empty() {
        parts.push(stream);
    }
    let tags: Vec<String> = tag_list(&s.tags).into_iter().take(2).collect();
    if !tags.is_empty() {
        parts.push(tags.join(", "));
    }
    parts.join(" · ")
}

/// A date as `2025-06-10`, from seconds since the epoch.
fn date_text(secs: i64) -> String {
    if secs <= 0 {
        return String::new();
    }
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// --- opening rows ------------------------------------------------------------------------------

/// What opening a row means on this surface.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The desktop: the detail pane.
    Desktop,
    /// A phone tab's stack.
    Shelf,
    Radio,
    Pods,
}

impl Place {
    pub fn open(self, scene: Scene, detail: Detail, context: Vec<Item>) {
        let app = App::app();
        scene.context.set(context);
        match self {
            Place::Desktop => {
                scene.detail.set(Some(detail));
                scene.detail_open.set(true);
            }
            Place::Shelf => {
                let route = match detail {
                    Detail::Track(id) => ShelfRoute::Track(id),
                    Detail::Album(k) => ShelfRoute::Album(k),
                    Detail::Artist(k) => ShelfRoute::Artist(k),
                    Detail::Genre(k) => ShelfRoute::Genre(k),
                    _ => return,
                };
                scene.shelf_path.update(|p| p.push(route));
            }
            Place::Radio => {
                if let Detail::Station(s) = detail {
                    let id = app.library.remember_station(&s);
                    scene
                        .radio_tab_path
                        .update(|p| p.push(RadioRoute::Station(id)));
                }
            }
            Place::Pods => {
                let route = match detail {
                    Detail::Episode(id) => PodRoute::Episode(id),
                    Detail::Show(id) => PodRoute::Show(id),
                    Detail::Preview(show) => {
                        app.podcasts.remember_show(&show);
                        PodRoute::Preview(show.feed)
                    }
                    _ => return,
                };
                scene.pod_path.update(|p| p.push(route));
            }
        }
    }
}

// --- the desktop shell ----------------------------------------------------------------------------

/// Whether this build can hold a music library: a browser has no files to scan or import for
/// its player, so the web build is radio and podcasts.
pub fn has_local_library() -> bool {
    cfg!(not(feature = "dom"))
}

pub fn desktop_shell(scene: Scene, prefs: Prefs, primary: bool) -> impl Piece {
    let app = App::app();
    let lib = app.library;
    let playlists = lib.playlists.clone();
    let nav = nav(scene.section)
        .title(res::str::app_title())
        .content_list(
            ContentList::new(content_pane)
                .width(380.0)
                .only_for(|s: &Section| {
                    !matches!(s, Section::Queue | Section::Settings | Section::Playing)
                })
                .detail_visible(scene.detail_open)
                .detail_title(move || detail_title(scene)),
        );
    // The web build has no files to scan, so its sidebar starts at Radio.
    let nav = if has_local_library() {
        nav.section(
            res::str::nav_library(),
            (
                item(Section::Songs, res::str::nav_songs())
                    .icon(res::vectors::nav_songs)
                    .icon_tint(Color::hex(0xF4511E))
                    .page(detail_pane),
                item(Section::Albums, res::str::nav_albums())
                    .icon(res::vectors::nav_albums)
                    .icon_tint(Color::hex(0x8E24AA))
                    .page(detail_pane),
                item(Section::Artists, res::str::nav_artists())
                    .icon(res::vectors::nav_artists)
                    .icon_tint(Color::hex(0x1E88E5))
                    .page(detail_pane),
                item(Section::Genres, res::str::nav_genres())
                    .icon(res::vectors::nav_genres)
                    .icon_tint(Color::hex(0x00897B))
                    .page(detail_pane),
                item(Section::RecentlyAdded, res::str::nav_recently_added())
                    .icon(res::vectors::nav_recent)
                    .icon_tint(Color::hex(0x43A047))
                    .page(detail_pane),
                item(Section::History, res::str::nav_history())
                    .icon(res::vectors::nav_history)
                    .icon_tint(Color::hex(0x6D4C41))
                    .page(detail_pane),
                item(Section::Queue, res::str::nav_queue())
                    .icon(res::vectors::nav_queue)
                    .icon_tint(Color::hex(0x546E7A))
                    .page(queue_page),
            ),
        )
        .section(
            res::str::nav_playlists(),
            nav_items(
                move || {
                    playlists
                        .ids()
                        .iter()
                        .filter_map(|id| lib.playlist(id.handle()))
                        .collect::<Vec<_>>()
                },
                |p: &Playlist| {
                    let id = p.id;
                    item(Section::Playlist(id), playlist_name(&p.name))
                        .icon(res::vectors::nav_playlist)
                        .icon_tint(Color::hex(0xFB8C00))
                        .context_menu(vec![
                            menu_item(res::str::cmd_rename_playlist())
                                .action(move || rename_playlist(App::app(), id)),
                            menu_item(res::str::cmd_delete_playlist())
                                .action(move || delete_playlist(App::app(), id)),
                        ])
                },
            ),
        )
    } else {
        nav.section(
            res::str::nav_listening(),
            (
                item(Section::History, res::str::nav_history())
                    .icon(res::vectors::nav_history)
                    .icon_tint(Color::hex(0x6D4C41))
                    .page(detail_pane),
                item(Section::Queue, res::str::nav_queue())
                    .icon(res::vectors::nav_queue)
                    .icon_tint(Color::hex(0x546E7A))
                    .page(queue_page),
            ),
        )
    };
    let nav = nav
        .section(
            res::str::nav_radio(),
            (
                item(Section::Stations, res::str::nav_stations())
                    .icon(res::vectors::nav_stations)
                    .icon_tint(Color::hex(0xE53935))
                    .page(detail_pane),
                item(Section::RadioBrowse, res::str::nav_browse())
                    .icon(res::vectors::nav_browse)
                    .icon_tint(Color::hex(0x3949AB))
                    .page(detail_pane),
            ),
        )
        .section(
            res::str::nav_podcasts(),
            (
                item(Section::Episodes, res::str::nav_episodes())
                    .icon(res::vectors::nav_episodes)
                    .icon_tint(Color::hex(0x7B1FA2))
                    .badge(move || {
                        let n = app.library.unplayed_count().get();
                        if n == 0 { String::new() } else { n.to_string() }
                    })
                    .page(detail_pane),
                item(Section::Shows, res::str::nav_shows())
                    .icon(res::vectors::nav_shows)
                    .icon_tint(Color::hex(0x5E35B1))
                    .page(detail_pane),
                item(Section::Discover, res::str::nav_discover())
                    .icon(res::vectors::nav_discover)
                    .icon_tint(Color::hex(0x039BE5))
                    .page(detail_pane),
            ),
        )
        .items(
            prefs
                .nav_item(Section::Settings)
                .map(|i| i.icon(res::vectors::tab_settings)),
        )
        .destination(|_: &Section| detail_pane())
        .id(day::id!(nav));
    let nav = if primary {
        nav.restore("app.section")
    } else {
        nav.local()
    };
    column((nav.grow(), player_bar(scene))).grow()
}

/// The pushed detail's navigation-bar title, where the toolkit titles one.
fn detail_title(scene: Scene) -> String {
    let app = App::app();
    match scene.detail.get() {
        Some(Detail::Track(id)) => app.library.track(id).map(|t| t.title).unwrap_or_default(),
        Some(Detail::Album(k)) => app
            .library
            .album(k)
            .map(|g| album_or_unknown(&g.name))
            .unwrap_or_default(),
        Some(Detail::Artist(k)) => app
            .library
            .artist(k)
            .map(|g| artist_or_unknown(&g.name))
            .unwrap_or_default(),
        Some(Detail::Genre(k)) => app.library.genre(k).map(|g| g.name).unwrap_or_default(),
        Some(Detail::Station(s)) => s.name,
        Some(Detail::Episode(id)) => app.library.episode(id).map(|e| e.title).unwrap_or_default(),
        Some(Detail::Show(id)) => app.library.podcast(id).map(|p| p.title).unwrap_or_default(),
        Some(Detail::Preview(s)) => s.title,
        None => section_title(&scene.section.get()),
    }
}

/// The middle column, built once per window and re-scoped by the section.
fn content_pane() -> impl Piece {
    let scene = Scene::ambient();
    let app = App::app();
    switch(
        move || scene.section.get(),
        move |section| match section {
            Section::Songs => songs_pane(scene, app, Place::Desktop).any(),
            Section::Albums => album_grid(
                scene,
                app,
                Place::Desktop,
                move || app.library.albums.get(),
                day::id!(album_grid),
            )
            .any(),
            Section::Artists => artist_list(scene, app, Place::Desktop).any(),
            Section::Genres => genre_list(scene, app, Place::Desktop).any(),
            Section::RecentlyAdded => track_list(
                scene,
                app,
                Place::Desktop,
                app.library.recent_tracks(),
                day::id!(recent_list),
                TrackRow::Full,
            )
            .any(),
            Section::History => history_list(scene, app, Place::Desktop).any(),
            Section::Playlist(id) => playlist_pane(scene, app, Place::Desktop, *id).any(),
            Section::Stations => stations_pane(scene, app, Place::Desktop).any(),
            Section::RadioBrowse => radio_browse_pane(scene, app).any(),
            Section::Episodes => episode_list(
                scene,
                app,
                Place::Desktop,
                app.library.latest_episodes(),
                day::id!(episode_list),
                true,
            )
            .any(),
            Section::Shows => shows_pane(scene, app, Place::Desktop).any(),
            Section::Discover => discover_pane(scene, app, Place::Desktop).any(),
            _ => column(()).any(),
        },
    )
}

/// The detail column: what the selection is, else the section's empty state.
fn detail_pane() -> impl Piece {
    let scene = Scene::ambient();
    let app = App::app();
    column((
        when(
            move || scene.detail.get().is_none(),
            move || {
                switch(
                    move || (scene.section.get(), app.library.track_count.get() == 0),
                    move |(section, empty): &(Section, bool)| {
                        empty_detail(app, section.clone(), *empty)
                    },
                )
            },
        ),
        keyed(
            move || scene.detail.get(),
            move |d| detail_page(app, scene, d.clone(), Place::Desktop),
        ),
    ))
    .grow()
}

fn empty_detail(app: App, section: Section, no_tracks: bool) -> impl Piece {
    let (glyph, text): (day::VectorName, String) = match section {
        Section::Songs | Section::RecentlyAdded | Section::Playlist(_) | Section::History => {
            if no_tracks {
                (res::vectors::nav_songs, res::str::library_empty().format())
            } else {
                (
                    res::vectors::nav_songs,
                    res::str::detail_pick_song().format(),
                )
            }
        }
        Section::Albums => (
            res::vectors::nav_albums,
            res::str::detail_pick_album().format(),
        ),
        Section::Artists => (
            res::vectors::nav_artists,
            res::str::detail_pick_artist().format(),
        ),
        Section::Genres => (
            res::vectors::nav_genres,
            res::str::detail_pick_genre().format(),
        ),
        Section::Stations | Section::RadioBrowse => (
            res::vectors::nav_stations,
            res::str::detail_pick_station().format(),
        ),
        Section::Episodes => (
            res::vectors::nav_episodes,
            res::str::detail_pick_episode().format(),
        ),
        Section::Shows | Section::Discover => (
            res::vectors::nav_shows,
            res::str::detail_pick_show().format(),
        ),
        _ => (res::vectors::app_mark, String::new()),
    };
    let empty_library = no_tracks
        && matches!(
            section,
            Section::Songs
                | Section::Albums
                | Section::Artists
                | Section::Genres
                | Section::RecentlyAdded
        );
    column((
        spacer(),
        vector(glyph).tint(tinted(GREY, 0.6)).frame(64.0, 64.0),
        label(text)
            .font(Font::Title3)
            .secondary()
            .align(TextAlign::Center)
            .max_width(360.0)
            .id(day::id!(detail_empty)),
        when(
            move || empty_library,
            move || {
                row((
                    when(
                        || capability(Cap::FolderDialog) != Support::Unsupported,
                        move || {
                            button(res::str::cmd_add_folder())
                                .prominent()
                                .action(move || add_folder(app))
                                .id(day::id!(empty_add_folder))
                        },
                    ),
                    button(res::str::cmd_import_files())
                        .bordered()
                        .action(move || import_files(app))
                        .id(day::id!(empty_import)),
                ))
                .spacing(Space::S)
            },
        ),
        spacer(),
    ))
    .spacing(Space::M)
    .align(HAlign::Center)
    .grow()
    .padding(Space::XL)
}

/// One detail page, shared by the desktop pane and the phone stacks.
fn detail_page(app: App, scene: Scene, detail: Detail, place: Place) -> AnyPiece {
    match detail {
        Detail::Track(id) => track_page(app, scene, id).any(),
        Detail::Album(k) => album_page(app, scene, place, k).any(),
        Detail::Artist(k) => artist_page(app, scene, place, k).any(),
        Detail::Genre(k) => genre_page(app, scene, place, k).any(),
        Detail::Station(s) => station_page(app, scene, s).any(),
        Detail::Episode(id) => episode_page(app, scene, id).any(),
        Detail::Show(id) => show_page(app, scene, place, id).any(),
        Detail::Preview(s) => preview_page(app, scene, place, s).any(),
    }
}

// --- artwork --------------------------------------------------------------------------------------

/// A picture at `size`, rounded; the placeholder glyph until it loads, or forever.
pub fn art_piece(art: Art, size: f64, radius: f64, glyph: day::VectorName) -> impl Piece {
    let bytes: Signal<Option<Arc<Vec<u8>>>> = Signal::new(None);
    let placeholder = tinted(GREY, 0.18);
    match art {
        // A stack centers the glyph on both axes of the square; a column would pin it to the top.
        Art::None => zstack((vector(glyph)
            .tint(tinted(GREY, 0.7))
            .frame(size * 0.45, size * 0.45),))
        .frame(size, size)
        .background(placeholder)
        .corner_radius(radius)
        .any(),
        Art::File(key) => {
            let path = format!("tunes/{key}");
            let task = day::task(async move {
                if let Ok(data) = day_part_filesystem::read_future(&path).await {
                    bytes.set(Some(Arc::new(data)));
                }
            });
            day::reactive::Scope::current().on_cleanup(move || task.abort());
            async_image(bytes)
                .rounded(radius)
                .placeholder_color(placeholder)
                .frame(size, size)
                .any()
        }
        Art::Url(url) => {
            if cfg!(any(feature = "dom", feature = "arkui")) || !url.starts_with("http") {
                return art_piece(Art::None, size, radius, glyph).any();
            }
            day::http_image(url)
                .rounded(radius)
                .placeholder_color(placeholder)
                .frame(size, size)
                .any()
        }
    }
}

fn art_for_url(url: &str) -> Art {
    if url.starts_with("http") {
        Art::Url(url.to_string())
    } else {
        Art::None
    }
}

// --- songs ----------------------------------------------------------------------------------------

/// How a track row reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackRow {
    /// Art, title, artist · album, duration.
    Full,
    /// Number, title, (artist when it differs), duration: inside an album.
    Album,
}

fn songs_pane(scene: Scene, app: App, place: Place) -> impl Piece {
    let query = app.library.tracks(
        move || scene.song_sort.get(),
        move || scene.song_query.get(),
    );
    let pane = column((
        when(
            move || app.library.track_count.get() == 0 && app.library.scanning.get().is_none(),
            move || {
                label(res::str::library_empty())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(songs_empty))
            },
        ),
        scan_banner(app),
        when(
            {
                let query = query.clone();
                move || !scene.song_query.get().trim().is_empty() && query.count() == 0
            },
            move || {
                label(move || res::str::search_none(scene.song_query.get()).format())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(songs_none))
            },
        ),
        track_list(
            scene,
            app,
            place,
            query.clone(),
            day::id!(song_list),
            TrackRow::Full,
        )
        .grow(),
        // The footer every music app has: how much is here.
        when(
            {
                let query = query.clone();
                move || query.count() > 0
            },
            {
                let query = query.clone();
                move || {
                    let q = query.clone();
                    label(move || res::str::track_count(q.count() as f64).format())
                        .font(Font::Caption)
                        .secondary()
                        .align(TextAlign::Center)
                        .padding(Insets::symmetric(16.0, 8.0))
                        .grow_w()
                        .id(day::id!(songs_count))
                }
            },
        ),
    ))
    .grow();
    let pane = pane.searchable(Search::new(scene.song_query).prompt(res::str::search_songs()));
    if place == Place::Desktop {
        let shuffle_query = query.clone();
        pane.toolbar(move || {
            let shuffle_query = shuffle_query.clone();
            vec![
                toolbar_menu(day::id!(tb_sort), res::str::cmd_sort(), sort_entries(scene))
                    .icon(Symbol::Sort)
                    .tooltip(res::str::cmd_sort()),
                toolbar_button(day::id!(tb_shuffle_all), res::str::cmd_shuffle_all())
                    .image(res::vectors::shuffle)
                    .tooltip(res::str::cmd_shuffle_all())
                    .enabled(app.library.track_count.get() > 0)
                    .action(move || {
                        let ids = Library::ids_of(&shuffle_query);
                        app.transport.set_shuffle(true);
                        app.transport
                            .play_items(ids.into_iter().map(Item::Track).collect(), 0);
                    }),
            ]
        })
        .any()
    } else {
        pane.any()
    }
}

fn sort_entries(scene: Scene) -> Vec<MenuEntry> {
    let entry = |sort: TrackSort, text: LocalizedText| {
        menu_item(text)
            .checked(move || scene.song_sort.get() == sort)
            .action(move || scene.song_sort.set(sort))
    };
    vec![
        entry(TrackSort::Title, res::str::sort_title()),
        entry(TrackSort::Artist, res::str::sort_artist()),
        entry(TrackSort::Album, res::str::sort_album()),
        entry(TrackSort::Added, res::str::sort_added()),
        entry(TrackSort::Played, res::str::sort_played()),
    ]
}

/// A line while the scanner runs.
fn scan_banner(app: App) -> impl Piece {
    when(
        move || app.library.scanning.get().is_some(),
        move || {
            row((
                spinner(),
                label(move || match app.library.scanning.get() {
                    Some(p) if p.folder.is_empty() => {
                        res::str::import_progress(p.added as f64).format()
                    }
                    Some(p) => res::str::scan_progress(res::str::ScanProgress {
                        folder: p.folder,
                        seen: p.seen as f64,
                    })
                    .format(),
                    None => String::new(),
                })
                .font(Font::Caption)
                .secondary()
                .id(day::id!(scan_banner)),
            ))
            .spacing(Space::S)
            .align(VAlign::Center)
            .padding(Insets::symmetric(16.0, 6.0))
        },
    )
}

/// The one track list every pane uses: a live query as the row source, rows faulting in as
/// they scroll. A tap on a phone plays; a desktop click opens the page and a double-click plays.
fn track_list(
    scene: Scene,
    app: App,
    place: Place,
    query: Query<Track>,
    id: day::ElementId,
    style: TrackRow,
) -> impl Piece {
    let order = query.clone();
    let positions = query.clone();
    let play_from = Rc::new(move |key: u64| {
        let ids = Library::ids_of(&order);
        let start = ids.iter().position(|k| *k == key).unwrap_or(0);
        app.transport
            .play_items(ids.into_iter().map(Item::Track).collect(), start);
    });
    let activate = play_from.clone();
    let open_order = query.clone();
    let mut list = list(query, move |slot: ModelSlot<Track>| {
        track_row(app, slot, style)
    })
    .row_height(RowHeight::Uniform(if style == TrackRow::Full {
        56.0
    } else {
        44.0
    }))
    .on_activate(move |e: Elem<Track>| activate(e.key()));
    if place == Place::Desktop {
        list = list
            .on_select(move |e: Elem<Track>| {
                let ids: Vec<Item> = Library::ids_of(&open_order)
                    .into_iter()
                    .map(Item::Track)
                    .collect();
                place.open(scene, Detail::Track(e.key()), ids);
            })
            .selected_rows(move || match scene.detail.get() {
                Some(Detail::Track(id)) => positions
                    .ids()
                    .iter()
                    .position(|k| k.handle() == id)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            });
    }
    list.id(id)
}

fn track_row(app: App, slot: ModelSlot<Track>, style: TrackRow) -> impl Piece {
    let t = app.transport;
    let id = move || slot.key();
    let is_on = move || t.is_current(Item::Track(id()));
    let menu = move || track_menu(app, id());
    let duration = label(move || {
        let d = slot.duration().read();
        if d > 0.0 { clock(d) } else { String::new() }
    })
    .font(Font::Caption)
    .secondary()
    .tabular();
    let title = hilite(is_on, move || {
        label(move || slot.title().read())
            .font(Font::Body)
            .single_line()
    });
    match style {
        TrackRow::Full => row((
            switch(
                move || slot.artwork().read(),
                move |key| {
                    art_piece(
                        crate::transport::art_for_key(app.library, key),
                        40.0,
                        6.0,
                        res::vectors::nav_songs,
                    )
                },
            ),
            column((
                title,
                label(move || {
                    let artist = artist_or_unknown(&slot.artist().read());
                    let album = slot.album().read();
                    if album.is_empty() {
                        artist
                    } else {
                        format!("{artist} · {album}")
                    }
                })
                .font(Font::Caption)
                .secondary()
                .single_line(),
            ))
            .spacing(2.0)
            .align(HAlign::Leading)
            .grow(),
            playing_mark(is_on),
            duration,
        ))
        .spacing(Space::S)
        .align(VAlign::Center)
        .context_menu(menu)
        .id_of(move || day::id!(track_row, slot.key()))
        .any(),
        TrackRow::Album => row((
            label(move || {
                let n = slot.track_no().read();
                if n > 0 { n.to_string() } else { String::new() }
            })
            .font(Font::Caption)
            .secondary()
            .tabular()
            .width(28.0),
            column((
                title,
                when(
                    move || {
                        let artist = slot.artist().read();
                        !artist.is_empty() && artist != slot.album_artist().read()
                    },
                    move || {
                        label(move || slot.artist().read())
                            .font(Font::Caption)
                            .secondary()
                            .single_line()
                    },
                ),
            ))
            .spacing(1.0)
            .align(HAlign::Leading)
            .grow(),
            playing_mark(is_on),
            duration,
        ))
        .spacing(Space::S)
        .padding(Insets::symmetric(20.0, 0.0))
        .align(VAlign::Center)
        .context_menu(menu)
        .id_of(move || day::id!(track_row, slot.key()))
        .any(),
    }
}

/// A label that takes the accent while `on` holds (a row on air), its own color otherwise.
fn hilite(
    on: impl Fn() -> bool + Copy + 'static,
    make: impl Fn() -> day_pieces::Label + Copy + 'static,
) -> impl Piece {
    when(on, move || make().color(ACCENT)).otherwise(make)
}

/// The accent glyph beside the row on air.
fn playing_mark(on: impl Fn() -> bool + Copy + 'static) -> impl Piece {
    when(on, || {
        vector(res::vectors::now_playing)
            .tint(ACCENT)
            .frame(16.0, 16.0)
    })
}

/// The row's menu: queue, playlists, where it belongs, removal.
fn track_menu(app: App, id: u64) -> Vec<MenuEntry> {
    let t = app.transport;
    let mut entries = vec![
        menu_item(res::str::cmd_play()).action(move || t.play_item(Item::Track(id))),
        menu_item(res::str::cmd_play_next()).action(move || t.play_next(vec![Item::Track(id)])),
        menu_item(res::str::cmd_add_to_queue())
            .action(move || t.add_to_queue(vec![Item::Track(id)])),
        menu_separator(),
        sub_menu(
            res::str::cmd_add_to_playlist(),
            playlist_entries(app, vec![id]),
        ),
    ];
    if let Some(track) = app.library.track(id) {
        let (album, artist) = (track.album_key, track.artist_key);
        entries.push(menu_separator());
        entries.push(
            menu_item(res::str::cmd_go_to_album()).action(move || go_to(Detail::Album(album))),
        );
        entries.push(
            menu_item(res::str::cmd_go_to_artist()).action(move || go_to(Detail::Artist(artist))),
        );
    }
    entries.push(menu_separator());
    entries
        .push(menu_item(res::str::cmd_remove_from_library()).action(move || remove_track(app, id)));
    entries
}

/// Open a detail from anywhere (a menu, a now-playing title): the front window shows it.
fn go_to(detail: Detail) {
    let Some(scene) = Scene::focused() else {
        return;
    };
    if is_phone() {
        scene.section.set(Section::Shelf);
        Place::Shelf.open(scene, detail, Vec::new());
    } else {
        let section = match &detail {
            Detail::Album(_) => Section::Albums,
            Detail::Artist(_) => Section::Artists,
            Detail::Genre(_) => Section::Genres,
            Detail::Track(_) => Section::Songs,
            Detail::Station(_) => Section::Stations,
            Detail::Episode(_) => Section::Episodes,
            Detail::Show(_) | Detail::Preview(_) => Section::Shows,
        };
        if scene.section.get_untracked() != section {
            scene.section.set(section);
        }
        // The section watcher clears the detail at the end of the turn; open after it.
        day::task(async move {
            day::sleep(1).await;
            Place::Desktop.open(scene, detail, Vec::new());
        });
    }
}

/// The "Add to Playlist" submenu: every playlist, and a new one.
fn playlist_entries(app: App, tracks: Vec<u64>) -> Vec<MenuEntry> {
    let lib = app.library;
    let mut entries: Vec<MenuEntry> = lib
        .playlists
        .ids_untracked()
        .iter()
        .filter_map(|id| lib.playlist(id.handle()))
        .map(|p| {
            let tracks = tracks.clone();
            let id = p.id;
            menu_item(playlist_name(&p.name)).action(move || lib.add_to_playlist(id, &tracks))
        })
        .collect();
    if !entries.is_empty() {
        entries.push(menu_separator());
    }
    let tracks = tracks.clone();
    entries.push(menu_item(res::str::cmd_new_playlist()).action(move || {
        let tracks = tracks.clone();
        day::task(async move {
            if let Some(name) = prompt(res::str::new_playlist_title())
                .placeholder(res::str::new_playlist_hint())
                .await
            {
                let id = lib.create_playlist(&name);
                lib.add_to_playlist(id, &tracks);
            }
        });
    }));
    entries
}

fn remove_track(app: App, id: u64) {
    day::task(async move {
        if confirm(res::str::remove_track_title())
            .message(res::str::remove_track_message())
            .confirm_label(res::str::cmd_remove())
            .destructive()
            .await
        {
            if app.transport.is_current(Item::Track(id)) {
                app.transport.stop();
            }
            app.library.remove_track(id);
            app.library.refresh_groups();
            if let Some(scene) = Scene::focused()
                && scene.detail.get_untracked() == Some(Detail::Track(id))
            {
                scene.close_detail();
            }
        }
    });
}

/// A song's page: the art, the facts, and what to do with it.
fn track_page(app: App, scene: Scene, id: u64) -> impl Piece {
    let Some(t) = app.library.track(id) else {
        return column(()).any();
    };
    let transport = app.transport;
    let (album_key, artist_key) = (t.album_key, t.artist_key);
    let path = app.library.track_url(&t);
    let context = scene.context.get_untracked();
    let start = context
        .iter()
        .position(|i| *i == Item::Track(id))
        .unwrap_or(0);
    let play_ctx = if context.is_empty() {
        vec![Item::Track(id)]
    } else {
        context.clone()
    };
    let facts = vec![
        (res::str::fact_album(), album_or_unknown(&t.album)),
        (res::str::fact_artist(), artist_or_unknown(&t.artist)),
        (res::str::fact_album_artist(), t.album_artist.clone()),
        (res::str::fact_genre(), t.genre.clone()),
        (
            res::str::fact_year(),
            if t.year > 0 {
                t.year.to_string()
            } else {
                String::new()
            },
        ),
        (
            res::str::fact_track_no(),
            match (t.track_no, t.disc_no) {
                (0, _) => String::new(),
                (n, 0) => n.to_string(),
                (n, d) => format!("{n} · {}", res::str::disc_no(d).format()),
            },
        ),
        (
            res::str::fact_duration(),
            if t.duration > 0.0 {
                clock(t.duration)
            } else {
                String::new()
            },
        ),
        (
            res::str::fact_format(),
            stream_line(&t.format, t.bitrate, false),
        ),
        (res::str::fact_plays(), t.play_count.to_string()),
        (res::str::fact_added(), date_text(t.added)),
    ];
    let rows: Vec<AnyPiece> = facts
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| labeled(k, label(v).selectable()).any())
        .collect();
    scroll(
        column((
            column((
                art_piece(
                    crate::transport::art_for_key(app.library, &t.artwork),
                    220.0,
                    16.0,
                    res::vectors::nav_songs,
                )
                .id(day::id!(track_art)),
                label(t.title.clone())
                    .font(Font::Title2)
                    .align(TextAlign::Center)
                    .max_width(520.0)
                    .id(day::id!(track_title)),
                label(artist_or_unknown(&t.artist))
                    .font(Font::Headline)
                    .color(ACCENT)
                    .align(TextAlign::Center)
                    .on_tap(move || go_to(Detail::Artist(artist_key)))
                    .id(day::id!(track_artist)),
                label(album_or_unknown(&t.album))
                    .font(Font::Subheadline)
                    .secondary()
                    .align(TextAlign::Center)
                    .on_tap(move || go_to(Detail::Album(album_key))),
                row((
                    button(res::str::cmd_play())
                        .prominent()
                        .icon(Symbol::Play)
                        .action(move || transport.play_items(play_ctx.clone(), start))
                        .id(day::id!(track_play)),
                    button(res::str::cmd_play_next())
                        .bordered()
                        .action(move || transport.play_next(vec![Item::Track(id)]))
                        .id(day::id!(track_play_next)),
                    button(res::str::cmd_add_to_queue())
                        .bordered()
                        .action(move || transport.add_to_queue(vec![Item::Track(id)]))
                        .id(day::id!(track_queue)),
                ))
                .spacing(Space::S)
                .fit(RowFit::Wrap { run_spacing: 8.0 }),
            ))
            .spacing(Space::S)
            .align(HAlign::Center)
            .grow_w(),
            form((section(rows).title(res::str::section_details()),)),
            labeled(
                res::str::fact_file(),
                label(path).font(Font::Caption).secondary().selectable(),
            ),
        ))
        .spacing(Space::L)
        .align(HAlign::Leading)
        .padding(Space::L),
    )
    .grow()
    .any()
}

// --- albums, artists, genres --------------------------------------------------------------------

/// A wall of covers.
fn album_grid(
    scene: Scene,
    app: App,
    place: Place,
    groups: impl Fn() -> Vec<Group> + Copy + 'static,
    id: day::ElementId,
) -> impl Piece {
    let tile_size = if is_phone() { 150.0 } else { 160.0 };
    column((
        when(
            move || groups().is_empty() && app.library.scanning.get().is_none(),
            move || {
                label(res::str::library_empty())
                    .secondary()
                    .padding(Space::L)
            },
        ),
        scan_banner(app),
        scroll(
            row((each(
                items(groups, |g: &Group| g.key),
                move |slot: ItemSlot<Group, u64>| {
                    let key = slot.key();
                    let group = slot.get();
                    column((
                        switch(
                            move || slot.field(|g| g.artwork.clone()),
                            move |art| {
                                art_piece(
                                    crate::transport::art_for_key(app.library, art),
                                    tile_size,
                                    10.0,
                                    res::vectors::nav_albums,
                                )
                            },
                        ),
                        label(move || album_or_unknown(&slot.field(|g| g.name.clone())))
                            .font(Font::Subheadline)
                            .bold()
                            .single_line()
                            .max_width(tile_size),
                        label(move || {
                            let g = slot.get();
                            let artist = artist_or_unknown(&g.subtitle);
                            if g.year > 0 {
                                format!("{artist} · {}", g.year)
                            } else {
                                artist
                            }
                        })
                        .font(Font::Caption)
                        .secondary()
                        .single_line()
                        .max_width(tile_size),
                    ))
                    .spacing(4.0)
                    .align(HAlign::Leading)
                    .width(tile_size)
                    .on_tap(move || place.open(scene, Detail::Album(key), Vec::new()))
                    .context_menu(move || album_menu(app, key))
                    .id_of(move || day::id!(album_tile, slot.key()))
                    .any()
                    .apply_if(group.key == 0, |p| p)
                },
            ),))
            .spacing(Space::M)
            .fit(RowFit::Wrap { run_spacing: 20.0 })
            .padding(Space::L)
            .id(id),
        )
        .grow(),
    ))
    .grow()
}

fn album_menu(app: App, key: u64) -> Vec<MenuEntry> {
    let t = app.transport;
    let ids = move || -> Vec<u64> { Library::ids_of(&app.library.album_tracks(key)) };
    vec![
        menu_item(res::str::cmd_play())
            .action(move || t.play_items(ids().into_iter().map(Item::Track).collect(), 0)),
        menu_item(res::str::cmd_play_next())
            .action(move || t.play_next(ids().into_iter().map(Item::Track).collect())),
        menu_item(res::str::cmd_add_to_queue())
            .action(move || t.add_to_queue(ids().into_iter().map(Item::Track).collect())),
        menu_separator(),
        sub_menu(
            res::str::cmd_add_to_playlist(),
            playlist_entries(app, ids()),
        ),
    ]
}

/// An album's page: the cover, its facts, and its tracks in order.
fn album_page(app: App, scene: Scene, place: Place, key: u64) -> impl Piece {
    let Some(g) = app.library.album(key) else {
        return column(()).any();
    };
    let t = app.transport;
    let query = app.library.album_tracks(key);
    let ids_for_play = query.clone();
    let shuffle_ids = query.clone();
    let artist_key = library::artist_key(&g.subtitle);
    let header = group_header(
        app,
        crate::transport::art_for_key(app.library, &g.artwork),
        res::vectors::nav_albums,
        album_or_unknown(&g.name),
        artist_or_unknown(&g.subtitle),
        Some(move || go_to(Detail::Artist(artist_key))),
        {
            let mut parts = vec![res::str::track_count(g.tracks as f64).format()];
            if g.year > 0 {
                parts.push(g.year.to_string());
            }
            if g.duration > 0.0 {
                parts.push(clock(g.duration));
            }
            parts.join(" · ")
        },
        move || {
            t.play_items(
                Library::ids_of(&ids_for_play)
                    .into_iter()
                    .map(Item::Track)
                    .collect(),
                0,
            )
        },
        move || {
            t.set_shuffle(true);
            t.play_items(
                Library::ids_of(&shuffle_ids)
                    .into_iter()
                    .map(Item::Track)
                    .collect(),
                0,
            );
        },
        move || album_menu(app, key),
        day::id!(album_page),
    );
    let _ = place;
    column((
        header,
        track_list(
            scene,
            app,
            Place::Shelf,
            query,
            day::id!(album_tracks),
            TrackRow::Album,
        )
        .grow(),
    ))
    .grow()
    .any()
}

/// The shared head of an album, artist, genre, playlist or show page.
#[allow(clippy::too_many_arguments)]
fn group_header(
    _app: App,
    art: Art,
    glyph: day::VectorName,
    title: String,
    subtitle: String,
    on_subtitle: Option<impl Fn() + 'static>,
    meta: String,
    play: impl Fn() + 'static,
    shuffle: impl Fn() + 'static,
    menu: impl Fn() -> Vec<MenuEntry> + 'static,
    id: day::ElementId,
) -> impl Piece {
    let subtitle_label = match on_subtitle {
        Some(f) => label(subtitle)
            .font(Font::Headline)
            .color(ACCENT)
            .single_line()
            .on_tap(f)
            .any(),
        None => label(subtitle)
            .font(Font::Headline)
            .secondary()
            .single_line()
            .any(),
    };
    let art_size = if is_phone() { 120.0 } else { 150.0 };
    row((
        art_piece(art, art_size, 12.0, glyph).id(day::id!(group_art)),
        column((
            label(title)
                .font(Font::Title2)
                .max_lines(2)
                .id(day::id!(group_title)),
            subtitle_label,
            label(meta).font(Font::Caption).secondary(),
            row((
                button(res::str::cmd_play())
                    .prominent()
                    .icon(Symbol::Play)
                    .action(play)
                    .id(day::id!(group_play)),
                button(res::str::cmd_shuffle())
                    .bordered()
                    .image(res::vectors::shuffle)
                    .action(shuffle)
                    .id(day::id!(group_shuffle)),
            ))
            .spacing(Space::S)
            .fit(RowFit::Wrap { run_spacing: 8.0 }),
        ))
        .spacing(6.0)
        .align(HAlign::Leading)
        .grow(),
    ))
    .spacing(Space::L)
    .align(VAlign::Bottom)
    .padding(Space::L)
    .context_menu(menu)
    .id(id)
}

fn artist_list(scene: Scene, app: App, place: Place) -> impl Piece {
    column((
        when(
            move || app.library.artists.get().is_empty() && app.library.scanning.get().is_none(),
            move || {
                label(res::str::library_empty())
                    .secondary()
                    .padding(Space::L)
            },
        ),
        scan_banner(app),
        list(
            items(move || app.library.artists.get(), |g: &Group| g.key),
            move |slot: ItemSlot<Group, u64>| {
                list_item(move || artist_or_unknown(&slot.field(|g| g.name.clone())))
                    .subtitle(move || {
                        let g = slot.get();
                        let albums: i64 = g.subtitle.parse().unwrap_or(0);
                        format!(
                            "{} · {}",
                            res::str::album_count(albums as f64).format(),
                            res::str::track_count(g.tracks as f64).format()
                        )
                    })
                    .icon(res::vectors::nav_artists)
                    .id_of(move || day::id!(artist_row, slot.key()))
            },
        )
        .on_select(move |key: u64| place.open(scene, Detail::Artist(key), Vec::new()))
        .selected_rows(move || match scene.detail.get() {
            Some(Detail::Artist(k)) if place == Place::Desktop => app
                .library
                .artists
                .with(|a| a.iter().position(|g| g.key == k))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        })
        .id(day::id!(artist_list))
        .grow(),
    ))
    .grow()
}

/// An artist's page: the albums as a shelf, then every track.
fn artist_page(app: App, scene: Scene, place: Place, key: u64) -> impl Piece {
    let Some(g) = app.library.artist(key) else {
        return column(()).any();
    };
    let t = app.transport;
    let query = app.library.artist_tracks(key);
    let play_ids = query.clone();
    let shuffle_ids = query.clone();
    let albums = app.library.artist_albums(key);
    let menu_ids = query.clone();
    let albums_count = albums.len();
    let album_tiles: Vec<AnyPiece> = albums
        .into_iter()
        .map(|a| {
            let k = a.key;
            column((
                art_piece(
                    crate::transport::art_for_key(app.library, &a.artwork),
                    110.0,
                    8.0,
                    res::vectors::nav_albums,
                ),
                label(album_or_unknown(&a.name))
                    .font(Font::Caption)
                    .bold()
                    .single_line()
                    .max_width(110.0),
                label(if a.year > 0 {
                    a.year.to_string()
                } else {
                    res::str::track_count(a.tracks as f64).format()
                })
                .font(Font::Caption2)
                .secondary(),
            ))
            .spacing(3.0)
            .align(HAlign::Leading)
            .width(110.0)
            .on_tap(move || {
                if place == Place::Desktop {
                    go_to(Detail::Album(k))
                } else {
                    place.open(scene, Detail::Album(k), Vec::new())
                }
            })
            .id(day::id!(artist_album, k))
            .any()
        })
        .collect();
    column((
        group_header(
            app,
            crate::transport::art_for_key(app.library, &g.artwork),
            res::vectors::nav_artists,
            artist_or_unknown(&g.name),
            String::new(),
            None::<fn()>,
            format!(
                "{} · {}",
                res::str::album_count(albums_count as f64).format(),
                res::str::track_count(g.tracks as f64).format()
            ),
            move || {
                t.play_items(
                    Library::ids_of(&play_ids)
                        .into_iter()
                        .map(Item::Track)
                        .collect(),
                    0,
                )
            },
            move || {
                t.set_shuffle(true);
                t.play_items(
                    Library::ids_of(&shuffle_ids)
                        .into_iter()
                        .map(Item::Track)
                        .collect(),
                    0,
                );
            },
            move || {
                let ids = Library::ids_of(&menu_ids);
                vec![
                    menu_item(res::str::cmd_add_to_queue()).action({
                        let ids = ids.clone();
                        move || t.add_to_queue(ids.iter().map(|i| Item::Track(*i)).collect())
                    }),
                    sub_menu(res::str::cmd_add_to_playlist(), playlist_entries(app, ids)),
                ]
            },
            day::id!(artist_page),
        ),
        scroll(
            row(album_tiles)
                .spacing(Space::M)
                .padding(Insets::symmetric(20.0, 0.0)),
        )
        .horizontal()
        .height(176.0)
        .id(day::id!(artist_albums)),
        track_list(
            scene,
            app,
            Place::Shelf,
            query,
            day::id!(artist_tracks),
            TrackRow::Full,
        )
        .grow(),
    ))
    .spacing(Space::S)
    .grow()
    .any()
}

fn genre_list(scene: Scene, app: App, place: Place) -> impl Piece {
    column((
        when(
            move || app.library.genres.get().is_empty() && app.library.scanning.get().is_none(),
            move || {
                label(res::str::genres_empty())
                    .secondary()
                    .padding(Space::L)
            },
        ),
        list(
            items(move || app.library.genres.get(), |g: &Group| g.key),
            move |slot: ItemSlot<Group, u64>| {
                list_item(move || slot.field(|g| g.name.clone()))
                    .subtitle(move || {
                        res::str::track_count(slot.field(|g| g.tracks) as f64).format()
                    })
                    .icon(res::vectors::nav_genres)
                    .id_of(move || day::id!(genre_row, slot.key()))
            },
        )
        .on_select(move |key: u64| place.open(scene, Detail::Genre(key), Vec::new()))
        .selected_rows(move || match scene.detail.get() {
            Some(Detail::Genre(k)) if place == Place::Desktop => app
                .library
                .genres
                .with(|a| a.iter().position(|g| g.key == k))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        })
        .id(day::id!(genre_list))
        .grow(),
    ))
    .grow()
}

fn genre_page(app: App, scene: Scene, _place: Place, key: u64) -> impl Piece {
    let Some(g) = app.library.genre(key) else {
        return column(()).any();
    };
    let t = app.transport;
    let query = app.library.genre_tracks(g.name.clone());
    let play_ids = query.clone();
    let shuffle_ids = query.clone();
    let menu_ids = query.clone();
    column((
        group_header(
            app,
            crate::transport::art_for_key(app.library, &g.artwork),
            res::vectors::nav_genres,
            g.name.clone(),
            String::new(),
            None::<fn()>,
            res::str::track_count(g.tracks as f64).format(),
            move || {
                t.play_items(
                    Library::ids_of(&play_ids)
                        .into_iter()
                        .map(Item::Track)
                        .collect(),
                    0,
                )
            },
            move || {
                t.set_shuffle(true);
                t.play_items(
                    Library::ids_of(&shuffle_ids)
                        .into_iter()
                        .map(Item::Track)
                        .collect(),
                    0,
                );
            },
            move || {
                let ids = Library::ids_of(&menu_ids);
                vec![sub_menu(
                    res::str::cmd_add_to_playlist(),
                    playlist_entries(app, ids),
                )]
            },
            day::id!(genre_page),
        ),
        track_list(
            scene,
            app,
            Place::Shelf,
            query,
            day::id!(genre_tracks),
            TrackRow::Full,
        )
        .grow(),
    ))
    .grow()
    .any()
}

// --- playlists ----------------------------------------------------------------------------------------

fn playlist_pane(scene: Scene, app: App, place: Place, id: u64) -> impl Piece {
    let lib = app.library;
    let t = app.transport;
    let entries = lib.playlist_entries(id);
    let order = entries.clone();
    let tracks_of = Rc::new(move || -> Vec<u64> {
        Library::ids_of(&order)
            .into_iter()
            .filter_map(|e| lib.entry_track(e))
            .collect()
    });
    let play_from = {
        let tracks_of = tracks_of.clone();
        Rc::new(move |entry: u64| {
            let entries = Library::ids_of(&entries);
            let start = entries.iter().position(|e| *e == entry).unwrap_or(0);
            t.play_items(tracks_of().into_iter().map(Item::Track).collect(), start);
        })
    };
    let activate = play_from.clone();
    let select_tracks = tracks_of.clone();
    let header_tracks = tracks_of.clone();
    let shuffle_tracks = tracks_of.clone();
    let menu_tracks = tracks_of.clone();
    let name = lib
        .playlist(id)
        .map(|p| playlist_name(&p.name))
        .unwrap_or_default();
    let count_entries = lib.playlist_entries(id);
    let header = when(
        move || place != Place::Desktop,
        move || {
            let header_tracks = header_tracks.clone();
            let shuffle_tracks = shuffle_tracks.clone();
            let menu_tracks = menu_tracks.clone();
            let name = name.clone();
            let count = count_entries.count();
            group_header(
                app,
                Art::None,
                res::vectors::nav_playlist,
                name,
                String::new(),
                None::<fn()>,
                res::str::track_count(count as f64).format(),
                move || t.play_items(header_tracks().into_iter().map(Item::Track).collect(), 0),
                move || {
                    t.set_shuffle(true);
                    t.play_items(shuffle_tracks().into_iter().map(Item::Track).collect(), 0);
                },
                move || {
                    vec![
                        menu_item(res::str::cmd_add_to_queue()).action({
                            let menu_tracks = menu_tracks.clone();
                            move || {
                                t.add_to_queue(menu_tracks().into_iter().map(Item::Track).collect())
                            }
                        }),
                        menu_item(res::str::cmd_rename_playlist())
                            .action(move || rename_playlist(app, id)),
                        menu_item(res::str::cmd_delete_playlist())
                            .action(move || delete_playlist(app, id)),
                    ]
                },
                day::id!(playlist_page),
            )
        },
    );
    let mut rows = list(
        lib.playlist_entries(id),
        move |slot: ModelSlot<library::PlaylistEntry>| {
            let track = move || lib.track(slot.track().read());
            let entry = move || slot.key();
            row((
                switch(
                    move || track().map(|t| t.artwork).unwrap_or_default(),
                    move |key| {
                        art_piece(
                            crate::transport::art_for_key(lib, key),
                            40.0,
                            6.0,
                            res::vectors::nav_songs,
                        )
                    },
                ),
                column((
                    hilite(
                        move || t.is_current(Item::Track(slot.track().read())),
                        move || {
                            label(move || {
                                track()
                                    .map(|t| t.title)
                                    .unwrap_or_else(|| res::str::track_missing().format())
                            })
                            .single_line()
                        },
                    ),
                    label(move || {
                        track()
                            .map(|t| {
                                let artist = artist_or_unknown(&t.artist);
                                if t.album.is_empty() {
                                    artist
                                } else {
                                    format!("{artist} · {}", t.album)
                                }
                            })
                            .unwrap_or_default()
                    })
                    .font(Font::Caption)
                    .secondary()
                    .single_line(),
                ))
                .spacing(2.0)
                .align(HAlign::Leading)
                .grow(),
                playing_mark(move || t.is_current(Item::Track(slot.track().read()))),
                label(move || {
                    track()
                        .map(|t| {
                            if t.duration > 0.0 {
                                clock(t.duration)
                            } else {
                                String::new()
                            }
                        })
                        .unwrap_or_default()
                })
                .font(Font::Caption)
                .secondary()
                .tabular(),
            ))
            .spacing(Space::S)
            .align(VAlign::Center)
            .context_menu(move || {
                let mut m = track_menu(app, slot.track().read());
                m.push(menu_separator());
                m.push(
                    menu_item(res::str::cmd_remove_from_playlist())
                        .action(move || lib.remove_entry(entry())),
                );
                m
            })
            .id_of(move || day::id!(playlist_row, slot.key()))
        },
    )
    .row_height(RowHeight::Uniform(56.0))
    .on_activate(move |e: Elem<library::PlaylistEntry>| activate(e.key()))
    .reorderable(Reorder::new(move |from, to| lib.move_entry(id, from, to)))
    .deletable(Delete::new(move |index| {
        let entries = Library::ids_of(&lib.playlist_entries(id));
        if let Some(e) = entries.get(index) {
            lib.remove_entry(*e);
        }
    }));
    if place == Place::Desktop {
        rows = rows.on_select(move |e: Elem<library::PlaylistEntry>| {
            if let Some(track) = lib.entry_track(e.key()) {
                let ctx: Vec<Item> = select_tracks().into_iter().map(Item::Track).collect();
                place.open(scene, Detail::Track(track), ctx);
            }
        });
    }
    let pane = column((
        header,
        when(
            move || lib.playlist_entries(id).count() == 0,
            move || {
                label(res::str::playlist_empty())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(playlist_empty))
            },
        ),
        rows.id(day::id!(playlist_list)).grow(),
    ))
    .grow();
    if place == Place::Desktop {
        let header_tracks = tracks_of.clone();
        pane.toolbar(move || {
            let header_tracks = header_tracks.clone();
            vec![
                toolbar_button(day::id!(tb_play_playlist), res::str::cmd_play())
                    .icon(Symbol::Play)
                    .tooltip(res::str::cmd_play())
                    .action(move || {
                        t.play_items(header_tracks().into_iter().map(Item::Track).collect(), 0)
                    }),
                toolbar_menu(
                    day::id!(tb_playlist_menu),
                    res::str::cmd_more(),
                    vec![
                        menu_item(res::str::cmd_rename_playlist())
                            .action(move || rename_playlist(app, id)),
                        menu_item(res::str::cmd_delete_playlist())
                            .action(move || delete_playlist(app, id)),
                    ],
                )
                .icon(Symbol::More),
            ]
        })
        .any()
    } else {
        pane.any()
    }
}

pub fn new_playlist(app: App, scene: Scene) {
    day::task(async move {
        if let Some(name) = prompt(res::str::new_playlist_title())
            .placeholder(res::str::new_playlist_hint())
            .await
        {
            let id = app.library.create_playlist(&name);
            if is_phone() {
                scene
                    .shelf_path
                    .update(|p| p.push(ShelfRoute::Playlist(id)));
            } else {
                scene.section.set(Section::Playlist(id));
            }
        }
    });
}

fn rename_playlist(app: App, id: u64) {
    let current = app.library.playlist(id).map(|p| p.name).unwrap_or_default();
    day::task(async move {
        if let Some(name) = prompt(res::str::rename_playlist_title())
            .initial(current)
            .await
        {
            app.library.rename_playlist(id, &name);
        }
    });
}

fn delete_playlist(app: App, id: u64) {
    day::task(async move {
        if confirm(res::str::delete_playlist_title())
            .message(res::str::delete_playlist_message())
            .confirm_label(res::str::cmd_delete())
            .destructive()
            .await
        {
            if let Some(scene) = Scene::focused() {
                if scene.section.get_untracked() == Section::Playlist(id) {
                    scene.section.set(Section::Songs);
                }
                scene
                    .shelf_path
                    .update(|p| p.retain(|r| *r != ShelfRoute::Playlist(id)));
            }
            app.library.delete_playlist(id);
        }
    });
}

// --- history and queue --------------------------------------------------------------------------------

fn history_list(scene: Scene, app: App, place: Place) -> impl Piece {
    let lib = app.library;
    let t = app.transport;
    let history = lib.history();
    let item_of = move |id: u64| -> Option<Item> {
        let h = lib.history_row(id)?;
        Some(match h.kind {
            library::KIND_TRACK => Item::Track(h.item),
            library::KIND_STATION => Item::Station(h.item),
            _ => Item::Episode(h.item),
        })
    };
    column((
        when(
            move || history.count() == 0,
            move || {
                label(res::str::history_empty())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(history_empty))
            },
        ),
        list(lib.history(), move |slot: ModelSlot<library::History>| {
            let glyph = move || match slot.kind().read() {
                library::KIND_STATION => res::vectors::nav_stations,
                library::KIND_EPISODE => res::vectors::nav_episodes,
                _ => res::vectors::nav_songs,
            };
            list_item(move || slot.title().read())
                .subtitle(move || slot.subtitle().read())
                .icon(glyph)
                .detail(move || date_text(slot.played().read()))
                .id_of(move || day::id!(history_row, slot.key()))
        })
        .on_activate(move |e: Elem<library::History>| {
            if let Some(item) = item_of(e.key()) {
                t.play_item(item);
            }
        })
        .on_select(move |e: Elem<library::History>| {
            if place != Place::Desktop {
                return;
            }
            match item_of(e.key()) {
                Some(Item::Track(id)) => place.open(scene, Detail::Track(id), Vec::new()),
                Some(Item::Station(id)) => {
                    if let Some(s) = lib.station(id) {
                        place.open(scene, Detail::Station(s), Vec::new());
                    }
                }
                Some(Item::Episode(id)) => place.open(scene, Detail::Episode(id), Vec::new()),
                None => {}
            }
        })
        .id(day::id!(history_list))
        .grow(),
    ))
    .grow()
    .toolbar(move || {
        vec![
            toolbar_button(day::id!(tb_clear_history), res::str::cmd_clear_history())
                .icon(Symbol::Delete)
                .tooltip(res::str::cmd_clear_history())
                .action(move || lib.clear_history()),
        ]
    })
}

/// The queue, in play order, with the current entry marked; rows drag and delete.
fn queue_list(app: App) -> impl Piece {
    let t = app.transport;
    let rows = move || -> Vec<(usize, Item)> {
        let queue = t.queue.get();
        t.order
            .get()
            .iter()
            .enumerate()
            .filter_map(|(at, i)| queue.get(*i).map(|item| (at, *item)))
            .collect()
    };
    column((
        when(
            move || t.order.with(|o| o.is_empty()),
            move || {
                label(res::str::queue_empty())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(queue_empty))
            },
        ),
        list(
            items(rows, |(at, item): &(usize, Item)| {
                format!("{at}-{}", item.id())
            }),
            move |slot: ItemSlot<(usize, Item), String>| {
                let described = move || describe(app.library, slot.get().1);
                let at = move || slot.get().0;
                row((
                    label(move || (at() + 1).to_string())
                        .font(Font::Caption)
                        .secondary()
                        .tabular()
                        .width(28.0),
                    column((
                        hilite(
                            move || t.pos.get() == Some(at()),
                            move || {
                                label(move || described().map(|d| d.title).unwrap_or_default())
                                    .single_line()
                            },
                        ),
                        label(move || described().map(|d| d.artist).unwrap_or_default())
                            .font(Font::Caption)
                            .secondary()
                            .single_line(),
                    ))
                    .spacing(2.0)
                    .align(HAlign::Leading)
                    .grow(),
                    playing_mark(move || t.pos.get() == Some(at())),
                    label(move || {
                        described()
                            .map(|d| {
                                if d.duration > 0.0 {
                                    clock(d.duration)
                                } else {
                                    String::new()
                                }
                            })
                            .unwrap_or_default()
                    })
                    .font(Font::Caption)
                    .secondary()
                    .tabular(),
                ))
                .spacing(Space::S)
                .align(VAlign::Center)
                .context_menu(move || {
                    vec![
                        menu_item(res::str::cmd_play()).action(move || t.play_at(at())),
                        menu_item(res::str::cmd_remove_from_queue())
                            .action(move || t.remove_at(at())),
                    ]
                })
                .id_of(move || day::id!(queue_row, slot.key()))
            },
        )
        .row_height(RowHeight::Uniform(52.0))
        .on_activate(move |key: String| {
            if let Some(at) = key.split('-').next().and_then(|a| a.parse::<usize>().ok()) {
                t.play_at(at);
            }
        })
        .reorderable(Reorder::new(move |from, to| t.move_entry(from, to)))
        .deletable(Delete::new(move |index| t.remove_at(index)))
        .id(day::id!(queue_list))
        .grow(),
    ))
    .grow()
}

/// The desktop Queue section: Now Playing large, the queue beside it.
fn queue_page() -> impl Piece {
    let app = App::app();
    let t = app.transport;
    let page = row((
        scroll(now_playing_panel(app, false)).width(380.0),
        column((
            row((
                label(res::str::queue_up_next()).font(Font::Headline).grow(),
                label(move || res::str::queue_count(t.order.with(|o| o.len()) as f64).format())
                    .font(Font::Caption)
                    .secondary(),
                button(res::str::cmd_clear_queue())
                    .plain()
                    .compact()
                    .enabled(move || t.order.with(|o| !o.is_empty()))
                    .action(move || t.clear_queue())
                    .id(day::id!(queue_clear)),
            ))
            .spacing(Space::S)
            .align(VAlign::Center)
            .padding(Insets::symmetric(20.0, 12.0)),
            queue_list(app).grow(),
        ))
        .grow(),
    ))
    .grow();
    page.toolbar(move || {
        vec![
            toolbar_toggle(
                day::id!(tb_shuffle),
                res::str::cmd_shuffle(),
                ShuffleBinding(t),
            )
            .image(res::vectors::shuffle)
            .tooltip(res::str::cmd_shuffle()),
            toolbar_button(day::id!(tb_repeat), res::str::cmd_repeat())
                .image(match t.repeat.get() {
                    Repeat::One => res::vectors::repeat_one,
                    _ => res::vectors::repeat,
                })
                .tooltip(res::str::cmd_repeat())
                .action(move || t.cycle_repeat()),
        ]
    })
}

/// The shuffle flag as a two-way `bool` binding for a toolbar toggle.
#[derive(Clone, Copy)]
struct ShuffleBinding(Transport);

impl Binding<bool> for ShuffleBinding {
    fn read(&self) -> bool {
        self.0.shuffle.get()
    }
    fn peek(&self) -> bool {
        self.0.shuffle.get_untracked()
    }
    fn write(&self, v: bool) {
        self.0.set_shuffle(v);
    }
}

// --- radio ----------------------------------------------------------------------------------------------

/// The listener's stations: favorites, then the ones played lately.
fn stations_pane(scene: Scene, app: App, place: Place) -> impl Piece {
    let lib = app.library;
    let favorites = lib.favorite_stations();
    let recents = lib.recent_stations();
    let fav_count = favorites.clone();
    let rec_count = recents.clone();
    column((
        when(
            move || fav_count.count() == 0 && rec_count.count() == 0,
            move || {
                column((
                    label(res::str::stations_empty())
                        .secondary()
                        .id(day::id!(stations_empty)),
                    button(res::str::nav_browse())
                        .bordered()
                        .action(move || {
                            if is_phone() {
                                scene
                                    .radio_tab_path
                                    .set(vec![RadioRoute::Browse(Browse::Top)]);
                            } else {
                                scene.section.set(Section::RadioBrowse);
                            }
                        })
                        .id(day::id!(stations_browse)),
                ))
                .spacing(Space::S)
                .align(HAlign::Leading)
                .padding(Space::L)
            },
        ),
        when(
            move || favorites.count() > 0,
            move || heading(res::str::stations_favorites()),
        ),
        switch(
            move || lib.favorite_stations().count().min(6),
            move |n: &usize| match n {
                0 => column(()).any(),
                n => station_query_list(
                    scene,
                    app,
                    place,
                    lib.favorite_stations(),
                    day::id!(favorite_stations),
                    *n,
                )
                .any(),
            },
        ),
        when(
            move || recents.count() > 0,
            move || heading(res::str::stations_recent()),
        ),
        switch(
            move || lib.recent_stations().count().min(6),
            move |n: &usize| match n {
                0 => column(()).any(),
                n => station_query_list(
                    scene,
                    app,
                    place,
                    lib.recent_stations(),
                    day::id!(recent_stations),
                    *n,
                )
                .any(),
            },
        ),
    ))
    .grow()
}

/// Stations from the store (favorites, recents), as a sized list inside the pane's column.
fn station_query_list(
    scene: Scene,
    app: App,
    place: Place,
    query: Query<Station>,
    id: day::ElementId,
    rows: usize,
) -> impl Piece {
    let lib = app.library;
    let order = query.clone();
    let positions = query.clone();
    let query_ctx = query.clone();
    let t = app.transport;
    list(query, move |slot: ModelSlot<Station>| {
        let station = move || lib.station(slot.key());
        station_row_body(app, move || station().unwrap_or_default())
            .id_of(move || day::id!(station_row, slot.key()))
    })
    .row_height(RowHeight::Uniform(60.0))
    .on_activate(move |e: Elem<Station>| {
        let ids: Vec<Item> = Library::ids_of(&order)
            .into_iter()
            .map(Item::Station)
            .collect();
        let start = ids
            .iter()
            .position(|i| *i == Item::Station(e.key()))
            .unwrap_or(0);
        t.play_items(ids, start);
    })
    .on_select({
        let ctx = query_ctx.clone();
        move |e: Elem<Station>| {
            if let Some(s) = lib.station(e.key()) {
                let ids: Vec<Item> = Library::ids_of(&ctx)
                    .into_iter()
                    .map(Item::Station)
                    .collect();
                place.open(scene, Detail::Station(s), ids);
            }
        }
    })
    .selected_rows(move || match scene.detail.get() {
        Some(Detail::Station(s)) if place == Place::Desktop => positions
            .ids()
            .iter()
            .position(|k| k.handle() == s.id)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    })
    .id(id)
    .height(60.0 * rows as f64)
}

/// The Radio tab's stations on a phone: the kept and the recently played stations under their
/// headings, each list taking its share of the height and scrolling on its own, or one line on
/// what will show up here. The browse rows above it are the way in, so there is no button.
fn phone_stations(scene: Scene, app: App) -> impl Piece {
    let lib = app.library;
    let favorites = lib.favorite_stations();
    let recents = lib.recent_stations();
    switch(
        move || (favorites.count() > 0, recents.count() > 0),
        move |(fav, rec): &(bool, bool)| match (fav, rec) {
            (false, false) => label(res::str::stations_empty())
                .secondary()
                .padding(Space::L)
                .id(day::id!(stations_empty))
                .any(),
            (fav, rec) => {
                let section = move |on: bool,
                                    title: LocalizedText,
                                    query: Query<Station>,
                                    id: day::ElementId| {
                    when(
                        move || on,
                        move || {
                            column((
                                heading(title.clone()),
                                station_query_list(
                                    scene,
                                    app,
                                    Place::Radio,
                                    query.clone(),
                                    id.clone(),
                                    0,
                                )
                                .grow(),
                            ))
                            .grow()
                        },
                    )
                    .grow()
                };
                column((
                    section(
                        *fav,
                        res::str::stations_favorites(),
                        lib.favorite_stations(),
                        day::id!(favorite_stations),
                    ),
                    section(
                        *rec,
                        res::str::stations_recent(),
                        lib.recent_stations(),
                        day::id!(recent_stations),
                    ),
                ))
                .grow()
                .any()
            }
        },
    )
}

/// A section heading over a list, set to the leading edge like the rows under it.
fn heading(text: LocalizedText) -> impl Piece {
    row((label(text).font(Font::Headline), spacer()))
        .align(VAlign::Center)
        .padding(Insets::symmetric(16.0, 8.0))
}

/// A station row's content: logo, name, place and stream, a heart, the on-air mark.
fn station_row_body(app: App, station: impl Fn() -> Station + Copy + 'static) -> impl Piece {
    let lib = app.library;
    let t = app.transport;
    row((
        switch(
            move || station().favicon,
            move |url| art_piece(art_for_url(url), 40.0, 8.0, res::vectors::nav_stations),
        ),
        column((
            hilite(
                move || t.is_current(Item::Station(station().id)),
                move || label(move || station().name).single_line(),
            ),
            label(move || station_meta(&station()))
                .font(Font::Caption)
                .secondary()
                .single_line(),
        ))
        .spacing(2.0)
        .align(HAlign::Leading)
        .grow(),
        playing_mark(move || t.is_current(Item::Station(station().id))),
        heart(app, move || station().id, station),
    ))
    .spacing(Space::S)
    .align(VAlign::Center)
    .context_menu(move || station_menu(app, station()))
    .apply_if(lib.files.is_none(), |p| p)
}

/// The favorite toggle.
fn heart(
    app: App,
    id: impl Fn() -> u64 + Copy + 'static,
    station: impl Fn() -> Station + Copy + 'static,
) -> impl Piece {
    let lib = app.library;
    column((when(
        move || lib.is_favorite_station(id()),
        || {
            vector(res::vectors::heart_fill)
                .tint(ACCENT)
                .frame(20.0, 20.0)
        },
    )
    .otherwise(|| vector(res::vectors::heart).tint(GREY).frame(20.0, 20.0)),))
    .padding(6.0)
    .on_tap(move || lib.toggle_favorite_station(&station()))
    .id_of(move || day::id!(station_heart, id()))
}

fn station_menu(app: App, s: Station) -> Vec<MenuEntry> {
    let lib = app.library;
    let t = app.transport;
    let id = s.id;
    let fav = lib.is_favorite_station(id);
    let s2 = s.clone();
    let s3 = s.clone();
    let mut entries = vec![
        menu_item(res::str::cmd_play()).action(move || {
            lib.remember_station(&s2);
            t.play_item(Item::Station(id));
        }),
        menu_item(if fav {
            res::str::cmd_unfavorite()
        } else {
            res::str::cmd_favorite()
        })
        .action(move || lib.toggle_favorite_station(&s3)),
    ];
    if !s.homepage.is_empty() {
        let home = s.homepage.clone();
        entries.push(menu_separator());
        entries.push(menu_item(res::str::station_homepage()).action(move || open_url(&home)));
    }
    entries
}

/// Browse: the cut (Top, Trending, Countries, Tags, Languages) as the root, with a search
/// field, and a pushed page of stations for a pick.
fn radio_browse_pane(scene: Scene, app: App) -> impl Piece {
    nav_stack(
        scene.radio_path,
        radio_browse_root(scene, app, Place::Desktop),
    )
    .destination(move |b: &Browse| browse_stations(scene, app, Place::Desktop, b.clone()))
    .id(day::id!(radio_stack))
}

fn radio_browse_root(scene: Scene, app: App, place: Place) -> impl Piece {
    let search = move || {
        let q = scene.radio_query.get_untracked().trim().to_string();
        if q.is_empty() {
            return;
        }
        let route = Browse::Search(q);
        if place == Place::Desktop {
            scene.radio_path.set(vec![route]);
        } else {
            scene
                .radio_tab_path
                .update(|p| p.push(RadioRoute::Browse(route)));
        }
    };
    let body = switch(
        move || scene.radio_cut.get(),
        move |cut| match cut {
            1 => facet_list(
                scene,
                app,
                place,
                FacetKind::Countries,
                day::id!(country_list),
            )
            .any(),
            2 => facet_list(scene, app, place, FacetKind::Tags, day::id!(tag_list)).any(),
            3 => facet_list(
                scene,
                app,
                place,
                FacetKind::Languages,
                day::id!(language_list),
            )
            .any(),
            4 => browse_stations(scene, app, place, Browse::Trending).any(),
            _ => browse_stations(scene, app, place, Browse::Top).any(),
        },
    );
    let pane = column((
        when(
            move || place != Place::Desktop,
            move || {
                picker(
                    scene.radio_cut,
                    [
                        (0usize, res::str::radio_top()),
                        (4, res::str::radio_trending()),
                        (1, res::str::radio_countries()),
                        (2, res::str::radio_tags()),
                        (3, res::str::radio_languages()),
                    ],
                )
                .segmented()
                .id(day::id!(radio_cut_picker))
                .padding(Insets::symmetric(12.0, 8.0))
            },
        ),
        body.grow(),
    ))
    .grow()
    .searchable(
        Search::new(scene.radio_query)
            .prompt(res::str::search_stations())
            .on_submit(move |_| search()),
    );
    if place == Place::Desktop {
        pane.toolbar(move || {
            vec![
                toolbar_segmented(
                    day::id!(tb_radio_cut),
                    vec![
                        segment(res::str::radio_top()),
                        segment(res::str::radio_countries()),
                        segment(res::str::radio_tags()),
                        segment(res::str::radio_languages()),
                        segment(res::str::radio_trending()),
                    ],
                    scene.radio_cut,
                )
                .placement(ToolbarPlacement::Principal),
            ]
        })
        .any()
    } else {
        pane.any()
    }
}

/// A list fetched from the index: the spinner, the failure with a retry, or the rows.
fn fetched<T: Clone + PartialEq + 'static>(
    signal: Signal<Fetched<T>>,
    retry: impl Fn() + 'static,
    ready: impl Fn(T) -> AnyPiece + 'static,
    id: day::ElementId,
) -> impl Piece {
    let ready = Rc::new(ready);
    let retry = Rc::new(retry);
    switch(
        move || signal.get(),
        move |state: &Fetched<T>| match state {
            Fetched::Loading => column((
                spinner(),
                label(res::str::loading()).font(Font::Caption).secondary(),
            ))
            .spacing(Space::S)
            .align(HAlign::Center)
            .padding(Space::XL)
            .grow_w()
            .id(day::id!(loading))
            .any(),
            Fetched::Failed(e) => {
                let retry = retry.clone();
                column((
                    label(res::str::fetch_failed(e.clone()).format())
                        .font(Font::Caption)
                        .secondary()
                        .align(TextAlign::Center)
                        .max_width(360.0),
                    button(res::str::cmd_retry())
                        .bordered()
                        .action(move || retry())
                        .id(day::id!(retry)),
                ))
                .spacing(Space::S)
                .align(HAlign::Center)
                .padding(Space::XL)
                .grow_w()
                .id(day::id!(fetch_failed))
                .any()
            }
            Fetched::Ready(v) => ready(v.clone()),
        },
    )
    .grow()
    .id(id)
}

fn facet_list(
    scene: Scene,
    app: App,
    place: Place,
    kind: FacetKind,
    id: day::ElementId,
) -> impl Piece {
    let signal = app.radio.facets(kind);
    fetched(
        signal,
        move || app.radio.refresh_facets(kind),
        move |rows: Vec<crate::radio::Facet>| {
            let rows2 = rows.clone();
            list(
                items(
                    move || rows2.clone(),
                    |f: &crate::radio::Facet| f.key.clone(),
                ),
                move |slot: ItemSlot<crate::radio::Facet, String>| {
                    list_item(move || {
                        let f = slot.get();
                        match kind {
                            FacetKind::Countries => {
                                format!("{} {}", flag(&f.key), short_country(&f.name))
                                    .trim()
                                    .to_string()
                            }
                            _ => title_case(&f.name),
                        }
                    })
                    .detail(move || {
                        res::str::station_count(slot.field(|f| f.stations) as f64).format()
                    })
                    .id_of(move || day::id!(facet_row, slot.key()))
                },
            )
            .row_height(RowHeight::Uniform(44.0))
            .on_select(move |key: String| {
                let route = match kind {
                    FacetKind::Countries => Browse::Country(key),
                    FacetKind::Tags => Browse::Tag(key),
                    FacetKind::Languages => Browse::Language(key),
                };
                if place == Place::Desktop {
                    scene.radio_path.set(vec![route]);
                } else {
                    scene
                        .radio_tab_path
                        .update(|p| p.push(RadioRoute::Browse(route)));
                }
            })
            .id(id.clone())
            .grow()
            .any()
        },
        day::id!(
            facets,
            match kind {
                FacetKind::Countries => "countries",
                FacetKind::Tags => "tags",
                FacetKind::Languages => "languages",
            }
        ),
    )
}

/// The stations of one browse, from the index.
fn browse_stations(scene: Scene, app: App, place: Place, browse: Browse) -> impl Piece {
    let signal = app.radio.stations(&browse);
    let retry_browse = Rc::new(browse.clone());
    let retry = move || app.radio.refresh(&retry_browse);
    let lib = app.library;
    let t = app.transport;
    fetched(
        signal,
        retry,
        move |rows: Vec<Station>| {
            if rows.is_empty() {
                return label(res::str::browse_none())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(browse_none))
                    .any();
            }
            let all = Rc::new(rows);
            let data = all.clone();
            let open_rows = all.clone();
            let play_rows = all.clone();
            list(
                items(move || (*data).clone(), |s: &Station| s.id),
                move |slot: ItemSlot<Station, u64>| {
                    station_row_body(app, move || slot.get())
                        .id_of(move || day::id!(station_row, slot.key()))
                },
            )
            .row_height(RowHeight::Uniform(60.0))
            .on_activate(move |key: u64| {
                for s in play_rows.iter() {
                    lib.remember_station(s);
                }
                let ids: Vec<Item> = play_rows.iter().map(|s| Item::Station(s.id)).collect();
                let start = play_rows.iter().position(|s| s.id == key).unwrap_or(0);
                t.play_items(ids, start);
            })
            .on_select(move |key: u64| {
                if let Some(s) = open_rows.iter().find(|s| s.id == key) {
                    let ids: Vec<Item> = open_rows.iter().map(|s| Item::Station(s.id)).collect();
                    for s in open_rows.iter() {
                        lib.remember_station(s);
                    }
                    place.open(scene, Detail::Station(s.clone()), ids);
                }
            })
            .selected_rows({
                let rows = all.clone();
                move || match scene.detail.get() {
                    Some(Detail::Station(s)) if place == Place::Desktop => {
                        rows.iter().position(|r| r.id == s.id).into_iter().collect()
                    }
                    _ => Vec::new(),
                }
            })
            .id(day::id!(browse_list))
            .grow()
            .any()
        },
        day::id!(browse, browse.key()),
    )
}

/// A station's page.
fn station_page(app: App, scene: Scene, s: Station) -> impl Piece {
    let lib = app.library;
    let t = app.transport;
    let id = s.id;
    let context = scene.context.get_untracked();
    let start = context
        .iter()
        .position(|i| *i == Item::Station(id))
        .unwrap_or(0);
    let play_ctx = if context.is_empty() {
        vec![Item::Station(id)]
    } else {
        context
    };
    let station = s.clone();
    let tags: Vec<AnyPiece> = tag_list(&s.tags)
        .into_iter()
        .map(|tag| {
            let tag2 = tag.clone();
            label(title_case(&tag))
                .font(Font::Caption)
                .color(ACCENT)
                .padding(Insets::symmetric(10.0, 3.0))
                .background(tinted(ACCENT, 0.14))
                .corner_radius(11.0)
                .on_tap(move || {
                    let route = Browse::Tag(tag2.clone());
                    if is_phone() {
                        scene
                            .radio_tab_path
                            .update(|p| p.push(RadioRoute::Browse(route)));
                    } else {
                        scene.section.set(Section::RadioBrowse);
                        day::task(async move {
                            day::sleep(1).await;
                            scene.radio_path.set(vec![route]);
                        });
                    }
                })
                .any()
        })
        .collect();
    let place_line = {
        let mut parts = Vec::new();
        if !s.country.is_empty() {
            parts.push(
                format!("{} {}", flag(&s.countrycode), short_country(&s.country))
                    .trim()
                    .to_string(),
            );
        }
        if !s.state.is_empty() {
            parts.push(s.state.clone());
        }
        parts.join(" · ")
    };
    let vote_done: Signal<Option<Result<(), String>>> = Signal::new(None);
    let uuid = s.uuid.clone();
    let fav_station = s.clone();
    scroll(
        column((
            column((
                art_piece(
                    art_for_url(&s.favicon),
                    180.0,
                    20.0,
                    res::vectors::nav_stations,
                )
                .id(day::id!(station_art)),
                label(s.name.clone())
                    .font(Font::Title2)
                    .align(TextAlign::Center)
                    .max_width(520.0)
                    .id(day::id!(station_name)),
                label(place_line)
                    .font(Font::Subheadline)
                    .secondary()
                    .align(TextAlign::Center)
                    .id(day::id!(station_place)),
                // What the stream names, while this station is on air.
                when(
                    move || t.is_current(Item::Station(id)),
                    move || {
                        label(move || match t.stream_title() {
                            Some(m) if !m.raw.is_empty() => m.raw,
                            _ => state_line(&t.state()),
                        })
                        .font(Font::Headline)
                        .color(ACCENT)
                        .align(TextAlign::Center)
                        .max_width(480.0)
                        .id(day::id!(station_state))
                    },
                ),
                row((
                    button(move || {
                        if t.is_current(Item::Station(id)) && t.is_active() {
                            res::str::cmd_pause().format()
                        } else {
                            res::str::cmd_play().format()
                        }
                    })
                    .icon(move || {
                        if t.is_current(Item::Station(id)) && t.is_active() {
                            Symbol::Pause
                        } else {
                            Symbol::Play
                        }
                    })
                    .prominent()
                    .action(move || {
                        if t.is_current(Item::Station(id)) {
                            t.toggle();
                        } else {
                            lib.remember_station(&station);
                            t.play_items(play_ctx.clone(), start);
                        }
                    })
                    .id(day::id!(station_play)),
                    button(move || {
                        if lib.is_favorite_station(id) {
                            res::str::cmd_unfavorite().format()
                        } else {
                            res::str::cmd_favorite().format()
                        }
                    })
                    .bordered()
                    .action(move || lib.toggle_favorite_station(&fav_station))
                    .id(day::id!(station_favorite)),
                    (!s.homepage.is_empty()).then(|| {
                        link(res::str::station_homepage(), s.homepage.clone())
                            .id(day::id!(station_homepage))
                    }),
                ))
                .spacing(Space::S)
                .fit(RowFit::Wrap { run_spacing: 8.0 })
                .align(VAlign::Center),
                row(tags)
                    .spacing(6.0)
                    .fit(RowFit::Wrap { run_spacing: 6.0 })
                    .id(day::id!(station_tags)),
            ))
            .spacing(Space::S)
            .align(HAlign::Center)
            .grow_w(),
            form((section((
                labeled(
                    res::str::fact_stream(),
                    label(stream_line(&s.codec, s.bitrate, s.hls)),
                ),
                labeled(res::str::fact_language(), label(s.language.clone())),
                labeled(
                    res::str::fact_votes(),
                    row((
                        label(grouped(s.votes)).tabular(),
                        button(res::str::cmd_vote())
                            .plain()
                            .compact()
                            .enabled(move || vote_done.get().is_none())
                            .action(move || app.radio.vote(&uuid, vote_done))
                            .id(day::id!(station_vote)),
                        label(move || match vote_done.get() {
                            Some(Ok(())) => res::str::vote_thanks().format(),
                            Some(Err(e)) => e,
                            None => String::new(),
                        })
                        .font(Font::Caption)
                        .secondary(),
                    ))
                    .spacing(Space::S)
                    .align(VAlign::Center),
                ),
                labeled(
                    res::str::fact_stream_url(),
                    label(stream_url(&s))
                        .font(Font::Caption)
                        .secondary()
                        .selectable(),
                ),
            ))
            .title(res::str::section_details()),)),
        ))
        .spacing(Space::L)
        .align(HAlign::Leading)
        .padding(Space::L),
    )
    .grow()
}

/// What the player says, as a line.
fn state_line(state: &PlaybackState) -> String {
    match state {
        PlaybackState::Idle => res::str::state_idle().format(),
        PlaybackState::Loading => res::str::state_loading().format(),
        PlaybackState::Playing => res::str::state_playing().format(),
        PlaybackState::Paused => res::str::state_paused().format(),
        PlaybackState::Ended => res::str::state_ended().format(),
        PlaybackState::Error(reason) => res::str::state_error(reason.clone()).format(),
    }
}

// --- podcasts -------------------------------------------------------------------------------------------

/// Episodes from the store. `with_show` adds the show's name under the title.
fn episode_list(
    scene: Scene,
    app: App,
    place: Place,
    query: Query<Episode>,
    id: day::ElementId,
    with_show: bool,
) -> impl Piece {
    let lib = app.library;
    let t = app.transport;
    let order = query.clone();
    let positions = query.clone();
    let count = query.clone();
    let selected_positions = query.clone();
    column((
        when(
            move || count.count() == 0,
            move || {
                column((
                    label(res::str::episodes_empty())
                        .secondary()
                        .id(day::id!(episodes_empty)),
                    button(res::str::nav_discover())
                        .bordered()
                        .action(move || {
                            if is_phone() {
                                scene.pod_path.set(vec![PodRoute::Discover]);
                            } else {
                                scene.section.set(Section::Discover);
                            }
                        })
                        .id(day::id!(episodes_discover)),
                ))
                .spacing(Space::S)
                .align(HAlign::Leading)
                .padding(Space::L)
            },
        ),
        list(query, move |slot: ModelSlot<Episode>| {
            let episode = move || lib.episode(slot.key()).unwrap_or_default();
            let show = move || lib.podcast(slot.podcast().read());
            let is_on = move || t.is_current(Item::Episode(slot.key()));
            row((
                switch(
                    move || {
                        let e = episode();
                        if e.image.is_empty() {
                            show().map(|s| s.image).unwrap_or_default()
                        } else {
                            e.image
                        }
                    },
                    move |url| art_piece(art_for_url(url), 48.0, 8.0, res::vectors::nav_episodes),
                ),
                column((
                    hilite(is_on, move || {
                        label(move || slot.title().read()).max_lines(2).bold()
                    }),
                    label(move || {
                        let e = episode();
                        let mut parts = Vec::new();
                        if with_show && let Some(s) = show() {
                            parts.push(s.title);
                        }
                        let d = date_text(e.published);
                        if !d.is_empty() {
                            parts.push(d);
                        }
                        if e.duration > 0.0 {
                            parts.push(clock(e.duration));
                        }
                        parts.join(" · ")
                    })
                    .font(Font::Caption)
                    .secondary()
                    .single_line(),
                    episode_progress(episode),
                ))
                .spacing(3.0)
                .align(HAlign::Leading)
                .grow(),
                playing_mark(is_on),
                when(
                    move || !slot.played().read(),
                    || {
                        column(())
                            .frame(8.0, 8.0)
                            .background(ACCENT)
                            .corner_radius(4.0)
                    },
                ),
                download_mark(app, move || slot.key()),
            ))
            .spacing(Space::S)
            .align(VAlign::Center)
            .context_menu(move || episode_menu(app, slot.key()))
            .id_of(move || day::id!(episode_row, slot.key()))
        })
        .row_height(RowHeight::Uniform(if with_show { 76.0 } else { 68.0 }))
        .on_activate(move |e: Elem<Episode>| {
            let ids: Vec<Item> = Library::ids_of(&order)
                .into_iter()
                .map(Item::Episode)
                .collect();
            let start = ids
                .iter()
                .position(|i| *i == Item::Episode(e.key()))
                .unwrap_or(0);
            t.play_items(ids, start);
        })
        .on_select(move |e: Elem<Episode>| {
            if place == Place::Desktop || place == Place::Pods {
                let ids: Vec<Item> = Library::ids_of(&positions)
                    .into_iter()
                    .map(Item::Episode)
                    .collect();
                place.open(scene, Detail::Episode(e.key()), ids);
            }
        })
        .selected_rows({
            let positions = selected_positions;
            move || match scene.detail.get() {
                Some(Detail::Episode(id)) if place == Place::Desktop => positions
                    .ids()
                    .iter()
                    .position(|k| k.handle() == id)
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        })
        .id(id)
        .grow(),
    ))
    .grow()
}

/// The thin bar under an episode that was started.
fn episode_progress(episode: impl Fn() -> Episode + Copy + 'static) -> impl Piece {
    when(
        move || {
            let e = episode();
            e.position > 0.0 && e.duration > 0.0 && !e.played
        },
        move || {
            progress(move || {
                let e = episode();
                (e.position / e.duration.max(1.0)).clamp(0.0, 1.0)
            })
            .frame(120.0, 4.0)
        },
    )
}

/// The download state glyph.
fn download_mark(app: App, id: impl Fn() -> u64 + Copy + 'static) -> impl Piece {
    let lib = app.library;
    let state = move || lib.episode(id()).map(|e| e.download).unwrap_or(0);
    column((
        when(
            move || state() == library::DOWNLOAD_DONE,
            || {
                vector(res::vectors::downloaded)
                    .tint(GREY)
                    .frame(18.0, 18.0)
            },
        ),
        when(
            move || state() == library::DOWNLOAD_RUNNING,
            move || {
                progress(move || {
                    app.podcasts
                        .downloads
                        .with(|d| {
                            d.get(&id()).map(|p| {
                                if p.total > 0 {
                                    p.received as f64 / p.total as f64
                                } else {
                                    0.0
                                }
                            })
                        })
                        .unwrap_or(0.0)
                })
                .frame(40.0, 4.0)
            },
        ),
    ))
}

fn episode_menu(app: App, id: u64) -> Vec<MenuEntry> {
    let lib = app.library;
    let t = app.transport;
    let played = lib.episode(id).map(|e| e.played).unwrap_or(false);
    let downloaded = lib.episode(id).map(|e| e.download).unwrap_or(0);
    let mut entries = vec![
        menu_item(res::str::cmd_play()).action(move || t.play_item(Item::Episode(id))),
        menu_item(res::str::cmd_play_next()).action(move || t.play_next(vec![Item::Episode(id)])),
        menu_item(res::str::cmd_add_to_queue())
            .action(move || t.add_to_queue(vec![Item::Episode(id)])),
        menu_separator(),
        menu_item(if played {
            res::str::cmd_mark_unplayed()
        } else {
            res::str::cmd_mark_played()
        })
        .action(move || lib.set_episode_position(id, 0.0, Some(!played))),
    ];
    if cfg!(not(target_arch = "wasm32")) {
        entries.push(if downloaded == library::DOWNLOAD_DONE {
            menu_item(res::str::cmd_remove_download())
                .action(move || app.podcasts.remove_download(id))
        } else {
            menu_item(res::str::cmd_download()).action(move || app.podcasts.download(id))
        });
    }
    if let Some(show) = lib.episode(id).map(|e| e.podcast) {
        entries.push(menu_separator());
        entries
            .push(menu_item(res::str::cmd_go_to_show()).action(move || go_to(Detail::Show(show))));
    }
    entries
}

/// An episode's page: the art, the show, the notes, and the controls.
fn episode_page(app: App, scene: Scene, id: u64) -> impl Piece {
    let lib = app.library;
    let t = app.transport;
    let Some(e) = lib.episode(id) else {
        return column(()).any();
    };
    let show = lib.podcast(e.podcast);
    let image = if e.image.is_empty() {
        show.as_ref().map(|s| s.image.clone()).unwrap_or_default()
    } else {
        e.image.clone()
    };
    let show_id = e.podcast;
    let context = scene.context.get_untracked();
    let start = context
        .iter()
        .position(|i| *i == Item::Episode(id))
        .unwrap_or(0);
    let play_ctx = if context.is_empty() {
        vec![Item::Episode(id)]
    } else {
        context
    };
    let meta = {
        let mut parts = Vec::new();
        let d = date_text(e.published);
        if !d.is_empty() {
            parts.push(d);
        }
        if e.duration > 0.0 {
            parts.push(clock(e.duration));
        }
        if e.bytes > 0 {
            parts.push(format!("{:.0} MB", e.bytes as f64 / 1_048_576.0));
        }
        parts.join(" · ")
    };
    scroll(
        column((
            column((
                art_piece(art_for_url(&image), 180.0, 16.0, res::vectors::nav_episodes)
                    .id(day::id!(episode_art)),
                label(e.title.clone())
                    .font(Font::Title2)
                    .align(TextAlign::Center)
                    .max_width(560.0)
                    .id(day::id!(episode_title)),
                label(show.as_ref().map(|s| s.title.clone()).unwrap_or_default())
                    .font(Font::Headline)
                    .color(ACCENT)
                    .align(TextAlign::Center)
                    .on_tap(move || go_to(Detail::Show(show_id)))
                    .id(day::id!(episode_show)),
                label(meta)
                    .font(Font::Caption)
                    .secondary()
                    .align(TextAlign::Center),
                label(move || {
                    let e = lib.episode(id).unwrap_or_default();
                    if e.played {
                        res::str::episode_played().format()
                    } else if e.position > 0.0 {
                        res::str::episode_resume_at(clock(e.position)).format()
                    } else {
                        String::new()
                    }
                })
                .font(Font::Caption)
                .secondary()
                .align(TextAlign::Center)
                .id(day::id!(episode_status)),
                row((
                    button(move || {
                        if t.is_current(Item::Episode(id)) && t.is_active() {
                            res::str::cmd_pause().format()
                        } else {
                            res::str::cmd_play().format()
                        }
                    })
                    .icon(move || {
                        if t.is_current(Item::Episode(id)) && t.is_active() {
                            Symbol::Pause
                        } else {
                            Symbol::Play
                        }
                    })
                    .prominent()
                    .action(move || {
                        if t.is_current(Item::Episode(id)) {
                            t.toggle();
                        } else {
                            t.play_items(play_ctx.clone(), start);
                        }
                    })
                    .id(day::id!(episode_play)),
                    button(res::str::cmd_add_to_queue())
                        .bordered()
                        .action(move || t.add_to_queue(vec![Item::Episode(id)]))
                        .id(day::id!(episode_queue)),
                    when(
                        || cfg!(not(target_arch = "wasm32")),
                        move || {
                            button(
                                move || match lib.episode(id).map(|e| e.download).unwrap_or(0) {
                                    library::DOWNLOAD_DONE => {
                                        res::str::cmd_remove_download().format()
                                    }
                                    library::DOWNLOAD_RUNNING => res::str::downloading().format(),
                                    _ => res::str::cmd_download().format(),
                                },
                            )
                            .bordered()
                            .enabled(move || {
                                lib.episode(id).map(|e| e.download).unwrap_or(0)
                                    != library::DOWNLOAD_RUNNING
                            })
                            .action(move || {
                                if lib.episode(id).map(|e| e.download).unwrap_or(0)
                                    == library::DOWNLOAD_DONE
                                {
                                    app.podcasts.remove_download(id)
                                } else {
                                    app.podcasts.download(id)
                                }
                            })
                            .id(day::id!(episode_download))
                        },
                    ),
                    button(move || {
                        if lib.episode(id).map(|e| e.played).unwrap_or(false) {
                            res::str::cmd_mark_unplayed().format()
                        } else {
                            res::str::cmd_mark_played().format()
                        }
                    })
                    .bordered()
                    .action(move || {
                        let played = lib.episode(id).map(|e| e.played).unwrap_or(false);
                        lib.set_episode_position(id, 0.0, Some(!played));
                    })
                    .id(day::id!(episode_mark)),
                ))
                .spacing(Space::S)
                .fit(RowFit::Wrap { run_spacing: 8.0 })
                .align(VAlign::Center),
            ))
            .spacing(Space::S)
            .align(HAlign::Center)
            .grow_w(),
            label(e.summary.clone())
                .font(Font::Body)
                .selectable()
                .id(day::id!(episode_notes)),
        ))
        .spacing(Space::L)
        .align(HAlign::Leading)
        .padding(Space::L),
    )
    .grow()
    .any()
}

/// Subscriptions.
fn shows_pane(scene: Scene, app: App, place: Place) -> impl Piece {
    let lib = app.library;
    let shows = lib.podcasts();
    let count = shows.clone();
    let positions = shows.clone();
    let pane = column((
        when(
            move || count.count() == 0,
            move || {
                column((
                    label(res::str::shows_empty())
                        .secondary()
                        .id(day::id!(shows_empty)),
                    row((
                        button(res::str::nav_discover())
                            .bordered()
                            .action(move || {
                                if is_phone() {
                                    scene.pod_path.set(vec![PodRoute::Discover]);
                                } else {
                                    scene.section.set(Section::Discover);
                                }
                            })
                            .id(day::id!(shows_discover)),
                        button(res::str::cmd_add_podcast())
                            .bordered()
                            .action(move || add_podcast_by_url(app, scene))
                            .id(day::id!(shows_add_url)),
                    ))
                    .spacing(Space::S),
                ))
                .spacing(Space::S)
                .align(HAlign::Leading)
                .padding(Space::L)
            },
        ),
        when(
            move || app.podcasts.refreshing.get() > 0,
            || {
                row((
                    spinner(),
                    label(res::str::refreshing())
                        .font(Font::Caption)
                        .secondary(),
                ))
                .spacing(Space::S)
                .align(VAlign::Center)
                .padding(Insets::symmetric(16.0, 6.0))
            },
        ),
        list(shows, move |slot: ModelSlot<Podcast>| {
            row((
                switch(
                    move || slot.image().read(),
                    move |url| art_piece(art_for_url(url), 48.0, 8.0, res::vectors::nav_shows),
                ),
                column((
                    label(move || slot.title().read()).bold().single_line(),
                    label(move || {
                        let author = slot.author().read();
                        let n = slot.episodes().read();
                        if author.is_empty() {
                            res::str::episode_count(n as f64).format()
                        } else {
                            format!("{author} · {}", res::str::episode_count(n as f64).format())
                        }
                    })
                    .font(Font::Caption)
                    .secondary()
                    .single_line(),
                ))
                .spacing(2.0)
                .align(HAlign::Leading)
                .grow(),
            ))
            .spacing(Space::S)
            .align(VAlign::Center)
            .context_menu(move || show_menu(app, slot.key()))
            .id_of(move || day::id!(show_row, slot.key()))
        })
        .row_height(RowHeight::Uniform(64.0))
        .on_select(move |e: Elem<Podcast>| place.open(scene, Detail::Show(e.key()), Vec::new()))
        .selected_rows(move || match scene.detail.get() {
            Some(Detail::Show(id)) if place == Place::Desktop => positions
                .ids()
                .iter()
                .position(|k| k.handle() == id)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        })
        .id(day::id!(show_list))
        .grow(),
    ))
    .grow();
    if place == Place::Desktop {
        pane.toolbar(move || {
            vec![
                toolbar_button(day::id!(tb_add_podcast), res::str::cmd_add_podcast())
                    .icon(Symbol::Add)
                    .tooltip(res::str::cmd_add_podcast())
                    .action(move || add_podcast_by_url(app, scene)),
                toolbar_button(day::id!(tb_refresh_shows), res::str::cmd_refresh())
                    .icon(Symbol::Refresh)
                    .tooltip(res::str::cmd_refresh_podcasts())
                    .enabled(app.podcasts.refreshing.get() == 0)
                    .action(move || app.podcasts.refresh_all()),
            ]
        })
        .any()
    } else {
        pane.any()
    }
}

fn show_menu(app: App, id: u64) -> Vec<MenuEntry> {
    vec![
        menu_item(res::str::cmd_refresh()).action(move || app.podcasts.refresh(id)),
        menu_item(res::str::cmd_mark_all_played()).action(move || {
            for e in Library::ids_of(&app.library.podcast_episodes(id)) {
                app.library.set_episode_position(e, 0.0, Some(true));
            }
        }),
        menu_separator(),
        menu_item(res::str::cmd_unsubscribe()).action(move || unsubscribe(app, id)),
    ]
}

fn unsubscribe(app: App, id: u64) {
    day::task(async move {
        if confirm(res::str::unsubscribe_title())
            .message(res::str::unsubscribe_message())
            .confirm_label(res::str::cmd_unsubscribe())
            .destructive()
            .await
        {
            if let Some(scene) = Scene::focused() {
                if matches!(scene.detail.get_untracked(), Some(Detail::Show(s)) if s == id) {
                    scene.close_detail();
                }
                scene
                    .pod_path
                    .update(|p| p.retain(|r| !matches!(r, PodRoute::Show(s) if *s == id)));
            }
            app.library.unsubscribe(id);
        }
    });
}

/// A show's page: its header, then its episodes.
fn show_page(app: App, scene: Scene, place: Place, id: u64) -> impl Piece {
    let lib = app.library;
    let Some(p) = lib.podcast(id) else {
        return column(()).any();
    };
    let link = p.link.clone();
    let episodes = lib.podcast_episodes(id);
    let t = app.transport;
    let play_ids = episodes.clone();
    let count = episodes.clone();
    let header = row((
        art_piece(art_for_url(&p.image), 140.0, 12.0, res::vectors::nav_shows)
            .id(day::id!(show_art)),
        column((
            label(p.title.clone())
                .font(Font::Title2)
                .max_lines(2)
                .id(day::id!(show_title)),
            label(p.author.clone())
                .font(Font::Headline)
                .secondary()
                .single_line(),
            label(move || res::str::episode_count(count.count() as f64).format())
                .font(Font::Caption)
                .secondary(),
            row((
                button(res::str::cmd_play_latest())
                    .prominent()
                    .icon(Symbol::Play)
                    .action(move || {
                        let ids = Library::ids_of(&play_ids);
                        if !ids.is_empty() {
                            t.play_items(ids.into_iter().map(Item::Episode).collect(), 0);
                        }
                    })
                    .id(day::id!(show_play)),
                button(res::str::cmd_refresh())
                    .bordered()
                    .action(move || app.podcasts.refresh(id))
                    .id(day::id!(show_refresh)),
                (!link.is_empty()).then(|| link_piece(res::str::show_website(), link.clone())),
            ))
            .spacing(Space::S)
            .fit(RowFit::Wrap { run_spacing: 8.0 })
            .align(VAlign::Center),
        ))
        .spacing(6.0)
        .align(HAlign::Leading)
        .grow(),
    ))
    .spacing(Space::L)
    .align(VAlign::Top)
    .padding(Space::L)
    .context_menu(move || show_menu(app, id));
    column((
        header,
        when(
            {
                let has = !p.summary.is_empty();
                move || has
            },
            {
                let summary = p.summary.clone();
                move || {
                    label(summary.clone())
                        .font(Font::Callout)
                        .secondary()
                        .max_lines(4)
                        .padding(Insets::symmetric(20.0, 0.0))
                }
            },
        ),
        episode_list(scene, app, place, episodes, day::id!(show_episodes), false).grow(),
    ))
    .align(HAlign::Leading)
    .spacing(Space::S)
    .grow()
    .any()
}

fn link_piece(text: LocalizedText, url: String) -> impl Piece {
    link(text, url)
}

/// Discover: the chart and a search over the directory.
fn discover_pane(scene: Scene, app: App, place: Place) -> impl Piece {
    let pods = app.podcasts;
    let run = move || pods.run_search(&scene.podcast_query.get_untracked());
    let pane = column((when(
        move || pods.search_term.get().is_empty(),
        move || {
            column((
                heading(res::str::discover_top()),
                directory_list(
                    scene,
                    app,
                    place,
                    pods.top,
                    move || pods.refresh_top(),
                    day::id!(top_shows),
                )
                .grow(),
            ))
            .grow()
        },
    )
    .otherwise(move || {
        directory_list(
            scene,
            app,
            place,
            pods.search,
            move || pods.run_search(&pods.search_term.get_untracked()),
            day::id!(search_shows),
        )
        .grow()
    }),))
    .grow()
    .searchable(
        Search::new(scene.podcast_query)
            .prompt(res::str::search_podcasts())
            .on_submit(move |_| run()),
    );
    // A cleared field goes back to the chart.
    watch(
        move || scene.podcast_query.get(),
        move |q, _| {
            if q.trim().is_empty() && !pods.search_term.get_untracked().is_empty() {
                pods.run_search("");
            }
        },
    );
    if place == Place::Desktop {
        pane.toolbar(move || {
            vec![
                toolbar_button(day::id!(tb_add_podcast_url), res::str::cmd_add_podcast())
                    .icon(Symbol::Add)
                    .tooltip(res::str::cmd_add_podcast())
                    .action(move || add_podcast_by_url(app, scene)),
            ]
        })
        .any()
    } else {
        pane.any()
    }
}

fn directory_list(
    scene: Scene,
    app: App,
    place: Place,
    signal: Signal<Fetched<Vec<DirectoryShow>>>,
    retry: impl Fn() + Copy + 'static,
    id: day::ElementId,
) -> impl Piece {
    let lib = app.library;
    let id_text = format!("{id:?}");
    fetched(
        signal,
        retry,
        move |rows: Vec<DirectoryShow>| {
            if rows.is_empty() {
                return label(res::str::discover_none())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(discover_none))
                    .any();
            }
            let all = Rc::new(rows);
            let data = all.clone();
            let open_rows = all.clone();
            list(
                items(move || (*data).clone(), |s: &DirectoryShow| s.id.clone()),
                move |slot: ItemSlot<DirectoryShow, String>| {
                    row((
                        switch(
                            move || slot.field(|s| s.image.clone()),
                            move |url| {
                                art_piece(art_for_url(url), 48.0, 8.0, res::vectors::nav_shows)
                            },
                        ),
                        column((
                            label(move || slot.field(|s| s.title.clone()))
                                .bold()
                                .single_line(),
                            label(move || {
                                let s = slot.get();
                                if s.genre.is_empty() {
                                    s.author
                                } else {
                                    format!("{} · {}", s.author, s.genre)
                                }
                            })
                            .font(Font::Caption)
                            .secondary()
                            .single_line(),
                        ))
                        .spacing(2.0)
                        .align(HAlign::Leading)
                        .grow(),
                        when(
                            move || lib.is_subscribed(&slot.field(|s| s.feed.clone())),
                            || {
                                vector(res::vectors::subscribed)
                                    .tint(ACCENT)
                                    .frame(18.0, 18.0)
                            },
                        ),
                    ))
                    .spacing(Space::S)
                    .align(VAlign::Center)
                    .id_of(move || day::id!(directory_row, slot.key()))
                },
            )
            .row_height(RowHeight::Uniform(64.0))
            .on_select(move |key: String| {
                if let Some(s) = open_rows.iter().find(|s| s.id == key) {
                    if lib.is_subscribed(&s.feed) {
                        place.open(
                            scene,
                            Detail::Show(library::podcast_id(&s.feed)),
                            Vec::new(),
                        );
                    } else {
                        place.open(scene, Detail::Preview(s.clone()), Vec::new());
                    }
                }
            })
            .selected_rows({
                let rows = all.clone();
                move || match scene.detail.get() {
                    Some(Detail::Preview(s)) if place == Place::Desktop => {
                        rows.iter().position(|r| r.id == s.id).into_iter().collect()
                    }
                    Some(Detail::Show(id)) if place == Place::Desktop => rows
                        .iter()
                        .position(|r| library::podcast_id(&r.feed) == id)
                        .into_iter()
                        .collect(),
                    _ => Vec::new(),
                }
            })
            .id(id.clone())
            .grow()
            .any()
        },
        day::id!(directory, id_text),
    )
}

/// A show from the directory, before subscribing: the feed's own description and latest
/// episodes, which can be played without subscribing.
fn preview_page(app: App, scene: Scene, place: Place, show: DirectoryShow) -> impl Piece {
    let lib = app.library;
    let pods = app.podcasts;
    let feed_url = show.feed.clone();
    let preview = pods.preview(&feed_url);
    let done: Signal<Option<Result<u64, String>>> = Signal::new(None);
    let sub_feed = feed_url.clone();
    let header = row((
        art_piece(
            art_for_url(&show.image),
            140.0,
            12.0,
            res::vectors::nav_shows,
        )
        .id(day::id!(preview_art)),
        column((
            label(show.title.clone())
                .font(Font::Title2)
                .max_lines(2)
                .id(day::id!(preview_title)),
            label(show.author.clone())
                .font(Font::Headline)
                .secondary()
                .single_line(),
            label(if show.genre.is_empty() {
                String::new()
            } else if show.episodes > 0 {
                format!(
                    "{} · {}",
                    show.genre,
                    res::str::episode_count(show.episodes as f64).format()
                )
            } else {
                show.genre.clone()
            })
            .font(Font::Caption)
            .secondary(),
            row((
                button(move || {
                    if lib.is_subscribed(&sub_feed) {
                        res::str::cmd_subscribed().format()
                    } else {
                        res::str::cmd_subscribe().format()
                    }
                })
                .prominent()
                .icon(Symbol::Add)
                .action({
                    let feed = feed_url.clone();
                    move || {
                        if lib.is_subscribed(&feed) {
                            place.open(scene, Detail::Show(library::podcast_id(&feed)), Vec::new());
                        } else {
                            pods.subscribe(&feed, done);
                        }
                    }
                })
                .id(day::id!(preview_subscribe)),
                label(move || match done.get() {
                    Some(Err(e)) => e,
                    _ => String::new(),
                })
                .font(Font::Caption)
                .secondary(),
            ))
            .spacing(Space::S)
            .align(VAlign::Center),
        ))
        .spacing(6.0)
        .align(HAlign::Leading)
        .grow(),
    ))
    .spacing(Space::L)
    .align(VAlign::Top)
    .padding(Space::L);
    // Once subscribed, the show page takes over.
    watch(
        move || done.get(),
        move |d, _| {
            if let Some(Ok(id)) = d {
                if is_phone() {
                    scene.pod_path.update(|p| {
                        p.pop();
                        p.push(PodRoute::Show(*id));
                    });
                } else {
                    scene.section.set(Section::Shows);
                    let id = *id;
                    day::task(async move {
                        day::sleep(1).await;
                        Place::Desktop.open(scene, Detail::Show(id), Vec::new());
                    });
                }
            }
        },
    );
    column((
        header,
        fetched(
            preview,
            {
                let feed_url = Rc::new(feed_url.clone());
                move || {
                    let _ = pods.preview(&feed_url);
                }
            },
            move |feed: crate::podcasts::Feed| {
                let episodes = feed.episodes.clone();
                column((
                    when(
                        {
                            let has = !feed.summary.is_empty();
                            move || has
                        },
                        {
                            let summary = feed.summary.clone();
                            move || {
                                label(summary.clone())
                                    .font(Font::Callout)
                                    .secondary()
                                    .max_lines(5)
                                    .padding(Insets::symmetric(20.0, 0.0))
                            }
                        },
                    ),
                    heading(res::str::preview_latest()),
                    list(
                        items(
                            move || episodes.clone(),
                            |e: &crate::podcasts::FeedEpisode| e.guid.clone(),
                        ),
                        move |slot: ItemSlot<crate::podcasts::FeedEpisode, String>| {
                            list_item(move || slot.field(|e| e.title.clone()))
                                .subtitle(move || {
                                    let e = slot.get();
                                    let mut parts = Vec::new();
                                    let d = date_text(e.published);
                                    if !d.is_empty() {
                                        parts.push(d);
                                    }
                                    if e.duration > 0.0 {
                                        parts.push(clock(e.duration));
                                    }
                                    parts.join(" · ")
                                })
                                .id_of(move || day::id!(preview_row, slot.key()))
                        },
                    )
                    .row_height(RowHeight::Uniform(56.0))
                    .id(day::id!(preview_episodes))
                    .grow(),
                ))
                .align(HAlign::Leading)
                .grow()
                .any()
            },
            day::id!(preview_feed),
        ),
    ))
    .grow()
    .any()
}

pub fn add_podcast_by_url(app: App, scene: Scene) {
    day::task(async move {
        let Some(url) = prompt(res::str::add_podcast_title())
            .message(res::str::add_podcast_message())
            .placeholder("https://example.org/feed.xml")
            .await
        else {
            return;
        };
        let url = url.trim().to_string();
        if url.is_empty() {
            return;
        }
        let done: Signal<Option<Result<u64, String>>> = Signal::new(None);
        app.podcasts.subscribe(&url, done);
        watch(
            move || done.get(),
            move |d, _| match d {
                Some(Ok(id)) => {
                    if is_phone() {
                        scene.section.set(Section::Podcasts);
                        scene.pod_path.set(vec![PodRoute::Show(*id)]);
                    } else {
                        scene.section.set(Section::Shows);
                        let id = *id;
                        day::task(async move {
                            day::sleep(1).await;
                            Place::Desktop.open(scene, Detail::Show(id), Vec::new());
                        });
                    }
                }
                Some(Err(e)) => {
                    let e = e.clone();
                    day::task(async move {
                        alert(res::str::add_podcast_failed()).message(e).await;
                    });
                }
                None => {}
            },
        );
    });
}

// --- the player bar and now playing -----------------------------------------------------------------

/// The desktop's bottom bar: what is on, the transport, the position, the volume.
fn player_bar(scene: Scene) -> impl Piece {
    let app = App::app();
    let t = app.transport;
    let loaded = move || t.loaded.get();
    let has = move || loaded().is_some();
    row((
        // What is on.
        row((
            keyed(
                move || loaded().map(|l| (l.item, l.art)),
                move |(_, art): &(Item, Art)| {
                    art_piece(art.clone(), 56.0, 6.0, untrack(|| loaded_glyph(t)))
                },
            ),
            column((
                label(move || now_title(t))
                    .font(Font::Headline)
                    .single_line()
                    .id(day::id!(bar_title)),
                label(move || now_subtitle(t))
                    .font(Font::Caption)
                    .secondary()
                    .single_line()
                    .id(day::id!(bar_subtitle)),
            ))
            .spacing(2.0)
            .align(HAlign::Leading)
            .grow(),
        ))
        .spacing(Space::S)
        .align(VAlign::Center)
        .on_tap(move || open_loaded(scene))
        .width(300.0),
        // The transport, with the position under it.
        column((
            row((
                small_glyph(
                    res::vectors::shuffle,
                    move || t.shuffle.get(),
                    move || true,
                    move || t.set_shuffle(!t.shuffle.get_untracked()),
                    day::id!(bar_shuffle),
                ),
                transport_glyph(
                    res::vectors::skip_previous,
                    26.0,
                    has,
                    move || t.previous(),
                    day::id!(bar_previous),
                ),
                play_button(t, 44.0, day::id!(bar_play)),
                transport_glyph(
                    res::vectors::skip_next,
                    26.0,
                    move || t.has_next(),
                    move || t.next(),
                    day::id!(bar_next),
                ),
                repeat_glyph(t, day::id!(bar_repeat)),
            ))
            .spacing(18.0)
            .align(VAlign::Center),
            seek_row(t, 440.0),
        ))
        .spacing(2.0)
        .align(HAlign::Center)
        .grow(),
        // Volume and the queue.
        row((
            vector(res::vectors::volume_down)
                .tint(GREY)
                .frame(18.0, 18.0),
            slider(t.volume)
                .range(0.0..=1.0)
                .frame(110.0, 28.0)
                .id(day::id!(bar_volume)),
            vector(res::vectors::volume_up).tint(GREY).frame(18.0, 18.0),
            small_glyph(
                res::vectors::nav_queue,
                move || scene.section.get() == Section::Queue,
                move || true,
                move || scene.section.set(Section::Queue),
                day::id!(bar_queue),
            ),
        ))
        .spacing(Space::S)
        .align(VAlign::Center)
        .width(300.0),
    ))
    .spacing(Space::M)
    .align(VAlign::Center)
    .padding(Insets::symmetric(16.0, 10.0))
    .height(84.0)
    .id(day::id!(player_bar))
}

/// The placeholder glyph for what is loaded.
fn loaded_glyph(t: Transport) -> day::VectorName {
    match t.current() {
        Some(Item::Station(_)) => res::vectors::nav_stations,
        Some(Item::Episode(_)) => res::vectors::nav_episodes,
        _ => res::vectors::nav_songs,
    }
}

/// Show the loaded item's page.
fn open_loaded(scene: Scene) {
    let app = App::app();
    let Some(item) = app.transport.current() else {
        return;
    };
    let _ = scene;
    match item {
        Item::Track(id) => go_to(Detail::Track(id)),
        Item::Station(id) => {
            if let Some(s) = app.library.station(id) {
                go_to(Detail::Station(s));
            }
        }
        Item::Episode(id) => go_to(Detail::Episode(id)),
    }
}

fn now_title(t: Transport) -> String {
    match t.loaded.get() {
        Some(l) if l.live => match t.stream_title() {
            Some(m) if !m.title.is_empty() => m.title,
            _ => l.title,
        },
        Some(l) => l.title,
        None => res::str::playing_nothing().format(),
    }
}

fn now_subtitle(t: Transport) -> String {
    match t.loaded.get() {
        Some(l) if l.live => match t.stream_title() {
            Some(m) if !m.title.is_empty() => {
                if m.artist.is_empty() {
                    l.title
                } else {
                    format!("{} · {}", m.artist, l.title)
                }
            }
            _ => match t.state() {
                PlaybackState::Playing if !l.artist.is_empty() => l.artist,
                PlaybackState::Playing => res::str::live().format(),
                s => state_line(&s),
            },
        },
        Some(l) => match t.state() {
            PlaybackState::Loading | PlaybackState::Error(_) => state_line(&t.state()),
            _ => {
                let artist = artist_or_unknown(&l.artist);
                if l.album.is_empty() {
                    artist
                } else {
                    format!("{artist} · {}", l.album)
                }
            }
        },
        None => res::str::playing_pick().format(),
    }
}

/// The big round play/pause.
fn play_button(t: Transport, size: f64, id: day::ElementId) -> impl Piece {
    let has = move || t.loaded.get().is_some();
    // A stack centers on both axes; a column would leave the glyph at the top of the disc.
    zstack((when(
        move || t.is_active(),
        move || {
            vector(res::vectors::pause)
                .tint(Color::hex(0xFFFFFF))
                .frame(size * 0.5, size * 0.5)
        },
    )
    .otherwise(move || {
        vector(res::vectors::play_arrow)
            .tint(Color::hex(0xFFFFFF))
            .frame(size * 0.5, size * 0.5)
    }),))
    .frame(size, size)
    .background(move || if has() { ACCENT } else { tinted(ACCENT, 0.4) })
    .corner_radius(size / 2.0)
    .on_tap(move || t.toggle())
    .id(id)
}

fn transport_glyph(
    name: day::VectorName,
    size: f64,
    enabled: impl Fn() -> bool + Copy + 'static,
    action: impl Fn() + 'static,
    id: day::ElementId,
) -> impl Piece {
    zstack((vector(name)
        .tint(move || {
            if enabled() {
                Color::hex(0x6E6E73)
            } else {
                tinted(GREY, 0.4)
            }
        })
        .frame(size, size),))
    .padding(6.0)
    .on_tap(action)
    .id(id)
}

/// A small mode glyph (shuffle, repeat, queue) that lights up when on.
fn small_glyph(
    name: day::VectorName,
    on: impl Fn() -> bool + Copy + 'static,
    enabled: impl Fn() -> bool + Copy + 'static,
    action: impl Fn() + 'static,
    id: day::ElementId,
) -> impl Piece {
    zstack((vector(name)
        .tint(move || {
            if !enabled() {
                tinted(GREY, 0.4)
            } else if on() {
                ACCENT
            } else {
                GREY
            }
        })
        .frame(18.0, 18.0),))
    .padding(6.0)
    .on_tap(action)
    .id(id)
}

fn repeat_glyph(t: Transport, id: day::ElementId) -> impl Piece {
    zstack((vector(move || match t.repeat.get() {
        Repeat::One => res::vectors::repeat_one,
        _ => res::vectors::repeat,
    })
    .tint(move || {
        if t.repeat.get() == Repeat::Off {
            GREY
        } else {
            ACCENT
        }
    })
    .frame(18.0, 18.0),))
    .padding(6.0)
    .on_tap(move || t.cycle_repeat())
    .id(id)
}

/// Elapsed, the slider, remaining. A live stream shows its state instead.
/// A slider's height: the desktop's thin control, or a phone's touch target, which Material
/// draws below a shorter frame and so out of line with the labels beside it.
fn slider_height() -> f64 {
    if is_phone() { 44.0 } else { 24.0 }
}

fn seek_row(t: Transport, width: f64) -> impl Piece {
    let live = move || t.loaded.with(|l| l.as_ref().is_some_and(|l| l.live));
    let duration = move || {
        t.loaded
            .with(|l| l.as_ref().map(|l| l.duration).unwrap_or(0.0))
            .max(0.0)
    };
    let position = move || t.scrub.get().unwrap_or_else(|| t.position());
    row((
        label(move || {
            if live() {
                String::new()
            } else {
                clock(position())
            }
        })
        .font(Font::Caption2)
        .secondary()
        .tabular()
        .width(44.0)
        .align(TextAlign::Trailing),
        when(
            move || !live(),
            move || {
                slider(ScrubBinding(t))
                    .range(0.0..=1.0)
                    .enabled(move || t.can_seek() && duration() > 0.0)
                    .frame(width - 100.0, slider_height())
                    .id(day::id!(bar_seek))
            },
        )
        .otherwise(move || {
            label(move || match t.stream_title() {
                Some(m) if !m.raw.is_empty() && t.is_active() => res::str::live().format(),
                _ => state_line(&t.state()),
            })
            .font(Font::Caption)
            .color(ACCENT)
            .align(TextAlign::Center)
            .frame(width - 100.0, slider_height())
        }),
        label(move || {
            if live() || duration() <= 0.0 {
                String::new()
            } else {
                format!("-{}", clock((duration() - position()).max(0.0)))
            }
        })
        .font(Font::Caption2)
        .secondary()
        .tabular()
        .width(44.0),
    ))
    .spacing(Space::XS)
    .align(VAlign::Center)
}

/// The seek slider's binding: a fraction of the duration, written to the transport as a seek
/// when it settles (the slider writes on every tick; the seek goes a beat later).
#[derive(Clone, Copy)]
struct ScrubBinding(Transport);

impl ScrubBinding {
    fn duration(&self) -> f64 {
        self.0
            .loaded
            .with_untracked(|l| l.as_ref().map(|l| l.duration).unwrap_or(0.0))
    }
}

impl Binding<f64> for ScrubBinding {
    fn read(&self) -> f64 {
        let d = self
            .0
            .loaded
            .with(|l| l.as_ref().map(|l| l.duration).unwrap_or(0.0));
        if d <= 0.0 {
            return 0.0;
        }
        (self.0.scrub.get().unwrap_or_else(|| self.0.position()) / d).clamp(0.0, 1.0)
    }
    fn peek(&self) -> f64 {
        let d = self.duration();
        if d <= 0.0 {
            0.0
        } else {
            (self.0.position() / d).clamp(0.0, 1.0)
        }
    }
    fn write(&self, v: f64) {
        let d = self.duration();
        if d <= 0.0 {
            return;
        }
        let secs = (v.clamp(0.0, 1.0) * d).min(d - 0.5).max(0.0);
        let t = self.0;
        t.scrub.set(Some(secs));
        day::task(async move {
            day::sleep(250).await;
            if t.scrub.get_untracked() == Some(secs) {
                t.seek(secs);
                day::sleep(300).await;
                if t.scrub.get_untracked() == Some(secs) {
                    t.scrub.set(None);
                }
            }
        });
    }
}

/// The full Now Playing surface: the art, the words, the position, the transport, the modes.
/// `tall` is the phone's tab; the desktop Queue page shows a narrower one beside the queue.
fn now_playing_panel(app: App, tall: bool) -> impl Piece {
    let t = app.transport;
    let art_size = if tall { 240.0 } else { 260.0 };
    let episode = move || matches!(t.current(), Some(Item::Episode(_)));
    column((
        spacer(),
        keyed(
            move || t.loaded.get().map(|l| (l.item, l.art)),
            move |(_, art): &(Item, Art)| {
                art_piece(art.clone(), art_size, 20.0, untrack(|| loaded_glyph(t)))
            },
        )
        .id(day::id!(playing_art)),
        label(move || now_title(t))
            .font(Font::Title2)
            .align(TextAlign::Center)
            .max_width(art_size + 60.0)
            .id(day::id!(playing_title)),
        label(move || now_subtitle(t))
            .font(Font::Headline)
            .secondary()
            .align(TextAlign::Center)
            .max_width(art_size + 60.0)
            .id(day::id!(playing_subtitle)),
        seek_row(t, art_size + 40.0),
        row((
            small_glyph(
                res::vectors::shuffle,
                move || t.shuffle.get(),
                move || !episode(),
                move || t.set_shuffle(!t.shuffle.get_untracked()),
                day::id!(playing_shuffle),
            ),
            when(episode, move || {
                transport_glyph(
                    res::vectors::replay_15,
                    30.0,
                    move || t.can_seek(),
                    move || t.skip(-15.0),
                    day::id!(playing_back15),
                )
            })
            .otherwise(move || {
                transport_glyph(
                    res::vectors::skip_previous,
                    34.0,
                    move || t.loaded.get().is_some(),
                    move || t.previous(),
                    day::id!(playing_previous),
                )
            }),
            play_button(t, 64.0, day::id!(playing_toggle)),
            when(episode, move || {
                transport_glyph(
                    res::vectors::forward_30,
                    30.0,
                    move || t.can_seek(),
                    move || t.skip(30.0),
                    day::id!(playing_fwd30),
                )
            })
            .otherwise(move || {
                transport_glyph(
                    res::vectors::skip_next,
                    34.0,
                    move || t.has_next(),
                    move || t.next(),
                    day::id!(playing_next),
                )
            }),
            repeat_glyph(t, day::id!(playing_repeat)),
        ))
        .spacing(20.0)
        .align(VAlign::Center),
        when(episode, move || rate_picker(t)),
        row((
            vector(res::vectors::volume_down)
                .tint(GREY)
                .frame(20.0, 20.0),
            slider(t.volume)
                .range(0.0..=1.0)
                .frame(art_size - 60.0, slider_height().max(36.0))
                .id(day::id!(playing_volume)),
            vector(res::vectors::volume_up).tint(GREY).frame(20.0, 20.0),
        ))
        .spacing(Space::S)
        .align(VAlign::Center),
        spacer(),
    ))
    .spacing(Space::M)
    .align(HAlign::Center)
    .grow()
    .padding(Space::L)
}

/// Podcast speed.
fn rate_picker(t: Transport) -> impl Piece {
    let rates = [0.8f64, 1.0, 1.25, 1.5, 2.0];
    row(rates
        .iter()
        .map(|r| {
            let r = *r;
            label(format!("{r}×"))
                .font(Font::Caption)
                .color(move || {
                    if (t.rate.get() - r).abs() < 0.01 {
                        ACCENT
                    } else {
                        GREY
                    }
                })
                .padding(Insets::symmetric(8.0, 4.0))
                .on_tap(move || t.rate.set(r))
                .id(day::id!(rate, format!("{r}")))
                .any()
        })
        .collect::<Vec<_>>())
    .spacing(Space::XS)
    .id(day::id!(rate_picker))
}

// --- the phone shell ----------------------------------------------------------------------------------

pub fn phone_shell(scene: Scene, prefs: Prefs, primary: bool) -> impl Piece {
    let app = App::app();
    let nav = nav(scene.section)
        .title(res::str::app_title())
        .items((
            item(Section::Shelf, res::str::nav_library())
                .icon(res::vectors::nav_songs)
                .page(move || with_mini(scene, shelf_tab(scene, app))),
            item(Section::Radio, res::str::nav_radio())
                .icon(res::vectors::nav_stations)
                .page(move || with_mini(scene, radio_tab(scene, app))),
            item(Section::Podcasts, res::str::nav_podcasts())
                .icon(res::vectors::nav_shows)
                .badge(move || {
                    let n = app.library.unplayed_count().get();
                    if n == 0 { String::new() } else { n.to_string() }
                })
                .page(move || with_mini(scene, podcasts_tab(scene, app))),
            item(Section::Playing, res::str::nav_playing())
                .icon(res::vectors::nav_playing)
                .page(move || playing_tab(scene, app)),
            prefs
                .nav_item(Section::Settings)
                .map(|i| i.icon(res::vectors::tab_settings)),
        ))
        .id(day::id!(nav));
    let nav = if primary {
        nav.restore("app.section")
    } else {
        nav.local()
    };
    nav.grow()
}

/// A tab's page over the mini player, which shows once something is loaded.
fn with_mini(scene: Scene, page: impl Piece) -> impl Piece {
    let app = App::app();
    let t = app.transport;
    column((
        page.grow(),
        when(
            move || t.loaded.get().is_some(),
            move || {
                row((
                    keyed(
                        move || t.loaded.get().map(|l| (l.item, l.art)),
                        move |(_, art): &(Item, Art)| {
                            art_piece(art.clone(), 44.0, 6.0, untrack(|| loaded_glyph(t)))
                        },
                    ),
                    column((
                        label(move || now_title(t))
                            .font(Font::Subheadline)
                            .bold()
                            .single_line(),
                        label(move || now_subtitle(t))
                            .font(Font::Caption)
                            .secondary()
                            .single_line(),
                    ))
                    .spacing(1.0)
                    .align(HAlign::Leading)
                    .grow()
                    .on_tap(move || scene.section.set(Section::Playing)),
                    play_button(t, 38.0, day::id!(mini_play)),
                    transport_glyph(
                        res::vectors::skip_next,
                        24.0,
                        move || t.has_next(),
                        move || t.next(),
                        day::id!(mini_next),
                    ),
                ))
                .spacing(Space::S)
                .align(VAlign::Center)
                .padding(Insets::symmetric(12.0, 8.0))
                .card()
                .padding(Insets::symmetric(8.0, 4.0))
                .id(day::id!(mini_player))
            },
        ),
    ))
    .grow()
}

fn playing_tab(scene: Scene, app: App) -> impl Piece {
    let t = app.transport;
    let root = column((
        now_playing_panel(app, true),
        when(
            move || t.order.with(|o| o.len() > 1),
            move || column((heading(res::str::queue_up_next()), queue_list(app).grow())).grow(),
        )
        .grow(),
    ))
    .grow();
    nav_stack(scene.playing_path, root)
        .title(res::str::nav_playing())
        .destination(move |route: &ShelfRoute| match route {
            ShelfRoute::Track(id) => track_page(app, scene, *id).any(),
            _ => column(()).any(),
        })
}

/// The Library tab's stack.
#[derive(Clone, Debug, PartialEq)]
pub enum ShelfRoute {
    Songs,
    Albums,
    Artists,
    Genres,
    Playlists,
    RecentlyAdded,
    History,
    Album(u64),
    Artist(u64),
    Genre(u64),
    Playlist(u64),
    Track(u64),
}

impl Route for ShelfRoute {
    fn key(&self) -> String {
        match self {
            ShelfRoute::Songs => "songs".into(),
            ShelfRoute::Albums => "albums".into(),
            ShelfRoute::Artists => "artists".into(),
            ShelfRoute::Genres => "genres".into(),
            ShelfRoute::Playlists => "playlists".into(),
            ShelfRoute::RecentlyAdded => "recent".into(),
            ShelfRoute::History => "history".into(),
            ShelfRoute::Album(k) => format!("album-{k}"),
            ShelfRoute::Artist(k) => format!("artist-{k}"),
            ShelfRoute::Genre(k) => format!("genre-{k}"),
            ShelfRoute::Playlist(k) => format!("playlist-{k}"),
            ShelfRoute::Track(k) => format!("track-{k}"),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "songs" => ShelfRoute::Songs,
            "albums" => ShelfRoute::Albums,
            "artists" => ShelfRoute::Artists,
            "genres" => ShelfRoute::Genres,
            "playlists" => ShelfRoute::Playlists,
            "recent" => ShelfRoute::RecentlyAdded,
            "history" => ShelfRoute::History,
            other => {
                let (kind, id) = other.split_once('-')?;
                let id: u64 = id.parse().ok()?;
                match kind {
                    "album" => ShelfRoute::Album(id),
                    "artist" => ShelfRoute::Artist(id),
                    "genre" => ShelfRoute::Genre(id),
                    "playlist" => ShelfRoute::Playlist(id),
                    "track" => ShelfRoute::Track(id),
                    _ => return None,
                }
            }
        })
    }
    fn title(&self) -> String {
        let app = App::app();
        match self {
            ShelfRoute::Songs => res::str::nav_songs().format(),
            ShelfRoute::Albums => res::str::nav_albums().format(),
            ShelfRoute::Artists => res::str::nav_artists().format(),
            ShelfRoute::Genres => res::str::nav_genres().format(),
            ShelfRoute::Playlists => res::str::nav_playlists().format(),
            ShelfRoute::RecentlyAdded => res::str::nav_recently_added().format(),
            ShelfRoute::History => res::str::nav_history().format(),
            ShelfRoute::Album(k) => app
                .library
                .album(*k)
                .map(|g| album_or_unknown(&g.name))
                .unwrap_or_default(),
            ShelfRoute::Artist(k) => app
                .library
                .artist(*k)
                .map(|g| artist_or_unknown(&g.name))
                .unwrap_or_default(),
            ShelfRoute::Genre(k) => app.library.genre(*k).map(|g| g.name).unwrap_or_default(),
            ShelfRoute::Playlist(k) => app
                .library
                .playlist(*k)
                .map(|p| playlist_name(&p.name))
                .unwrap_or_default(),
            ShelfRoute::Track(k) => app.library.track(*k).map(|t| t.title).unwrap_or_default(),
        }
    }
}

fn shelf_tab(scene: Scene, app: App) -> impl Piece {
    let lib = app.library;
    let root = {
        let rows: Vec<(ShelfRoute, LocalizedText, day::VectorName)> = vec![
            (
                ShelfRoute::Playlists,
                res::str::nav_playlists(),
                res::vectors::nav_playlist,
            ),
            (
                ShelfRoute::Artists,
                res::str::nav_artists(),
                res::vectors::nav_artists,
            ),
            (
                ShelfRoute::Albums,
                res::str::nav_albums(),
                res::vectors::nav_albums,
            ),
            (
                ShelfRoute::Songs,
                res::str::nav_songs(),
                res::vectors::nav_songs,
            ),
            (
                ShelfRoute::Genres,
                res::str::nav_genres(),
                res::vectors::nav_genres,
            ),
            (
                ShelfRoute::RecentlyAdded,
                res::str::nav_recently_added(),
                res::vectors::nav_recent,
            ),
            (
                ShelfRoute::History,
                res::str::nav_history(),
                res::vectors::nav_history,
            ),
        ];
        let keys: Vec<ShelfRoute> = rows.iter().map(|(r, _, _)| r.clone()).collect();
        column((
            list(
                items(
                    move || rows.clone(),
                    |(r, _, _): &(ShelfRoute, LocalizedText, day::VectorName)| r.key(),
                ),
                move |slot: ItemSlot<(ShelfRoute, LocalizedText, day::VectorName), String>| {
                    list_item(move || slot.field(|(_, t, _)| t.format()))
                        .icon(move || slot.field(|(_, _, v)| v.clone()))
                        .icon_tint(ACCENT)
                        .id_of(move || day::id!(shelf_row, slot.key()))
                },
            )
            .row_height(RowHeight::Uniform(48.0))
            .on_select(move |key: String| {
                if let Some(r) = keys.iter().find(|r| r.key() == key) {
                    scene.shelf_path.update(|p| p.push(r.clone()));
                }
            })
            .height(48.0 * 7.0)
            .id(day::id!(shelf_root)),
            heading(res::str::recently_added_heading()),
            scroll(
                row((each(
                    items(
                        move || lib.albums.get().into_iter().take(12).collect::<Vec<_>>(),
                        |g: &Group| g.key,
                    ),
                    move |slot: ItemSlot<Group, u64>| {
                        let key = slot.key();
                        column((
                            switch(
                                move || slot.field(|g| g.artwork.clone()),
                                move |art| {
                                    art_piece(
                                        crate::transport::art_for_key(lib, art),
                                        120.0,
                                        10.0,
                                        res::vectors::nav_albums,
                                    )
                                },
                            ),
                            label(move || album_or_unknown(&slot.field(|g| g.name.clone())))
                                .font(Font::Caption)
                                .bold()
                                .single_line()
                                .max_width(120.0),
                            label(move || artist_or_unknown(&slot.field(|g| g.subtitle.clone())))
                                .font(Font::Caption2)
                                .secondary()
                                .single_line()
                                .max_width(120.0),
                        ))
                        .spacing(3.0)
                        .align(HAlign::Leading)
                        .width(120.0)
                        .on_tap(move || scene.shelf_path.update(|p| p.push(ShelfRoute::Album(key))))
                        .id_of(move || day::id!(shelf_album, slot.key()))
                    },
                ),))
                .spacing(Space::M)
                .padding(Insets::symmetric(16.0, 0.0)),
            )
            .horizontal(),
            when(
                move || lib.track_count.get() == 0 && lib.scanning.get().is_none(),
                move || {
                    column((
                        label(res::str::library_empty_phone())
                            .secondary()
                            .align(TextAlign::Center)
                            .max_width(320.0),
                        button(res::str::cmd_import_files())
                            .prominent()
                            .action(move || import_files(app))
                            .id(day::id!(shelf_import)),
                    ))
                    .spacing(Space::M)
                    .align(HAlign::Center)
                    .grow_w()
                    .padding(Space::L)
                },
            ),
            scan_banner(app),
            spacer(),
        ))
        .grow()
    };
    let root = scroll(root).grow().toolbar(move || {
        vec![
            toolbar_menu(
                day::id!(tb_shelf_add),
                res::str::cmd_add(),
                vec![
                    menu_item(res::str::cmd_import_files()).action(move || import_files(app)),
                    menu_item(res::str::cmd_new_playlist())
                        .action(move || new_playlist(app, scene)),
                ],
            )
            .icon(Symbol::Add),
        ]
    });
    nav_stack(scene.shelf_path, root)
        .title(res::str::nav_library())
        .destination(move |route: &ShelfRoute| shelf_destination(scene, app, route.clone()))
        .id(day::id!(shelf_stack))
}

fn shelf_destination(scene: Scene, app: App, route: ShelfRoute) -> AnyPiece {
    let place = Place::Shelf;
    match route {
        ShelfRoute::Songs => songs_pane(scene, app, place).any(),
        ShelfRoute::Albums => album_grid(
            scene,
            app,
            place,
            move || app.library.albums.get(),
            day::id!(album_grid),
        )
        .any(),
        ShelfRoute::Artists => artist_list(scene, app, place).any(),
        ShelfRoute::Genres => genre_list(scene, app, place).any(),
        ShelfRoute::Playlists => playlists_list(scene, app).any(),
        ShelfRoute::RecentlyAdded => track_list(
            scene,
            app,
            place,
            app.library.recent_tracks(),
            day::id!(recent_list),
            TrackRow::Full,
        )
        .any(),
        ShelfRoute::History => history_list(scene, app, place).any(),
        ShelfRoute::Album(k) => album_page(app, scene, place, k).any(),
        ShelfRoute::Artist(k) => artist_page(app, scene, place, k).any(),
        ShelfRoute::Genre(k) => genre_page(app, scene, place, k).any(),
        ShelfRoute::Playlist(k) => playlist_pane(scene, app, place, k).any(),
        ShelfRoute::Track(k) => track_page(app, scene, k).any(),
    }
}

/// The phone's playlists page.
fn playlists_list(scene: Scene, app: App) -> impl Piece {
    let lib = app.library;
    let count = lib.playlists.clone();
    column((
        when(
            move || count.count() == 0,
            move || {
                label(res::str::playlists_empty())
                    .secondary()
                    .padding(Space::L)
                    .id(day::id!(playlists_empty))
            },
        ),
        list(
            lib.playlists.clone(),
            move |slot: ModelSlot<library::Playlist>| {
                list_item(move || playlist_name(&slot.name().read()))
                    .subtitle(move || {
                        res::str::track_count(lib.playlist_entries(slot.key()).count() as f64)
                            .format()
                    })
                    .icon(res::vectors::nav_playlist)
                    .icon_tint(Color::hex(0xFB8C00))
                    .context_menu(move || {
                        let id = slot.key();
                        vec![
                            menu_item(res::str::cmd_rename_playlist())
                                .action(move || rename_playlist(app, id)),
                            menu_item(res::str::cmd_delete_playlist())
                                .action(move || delete_playlist(app, id)),
                        ]
                    })
                    .id_of(move || day::id!(playlist_item, slot.key()))
            },
        )
        .on_select(move |e: Elem<library::Playlist>| {
            scene
                .shelf_path
                .update(|p| p.push(ShelfRoute::Playlist(e.key())))
        })
        .id(day::id!(playlists_list))
        .grow(),
    ))
    .grow()
    .toolbar(move || {
        vec![
            toolbar_button(day::id!(tb_new_playlist), res::str::cmd_new_playlist())
                .icon(Symbol::Add)
                .action(move || new_playlist(app, scene)),
        ]
    })
}

/// The Radio tab's stack.
#[derive(Clone, Debug, PartialEq)]
pub enum RadioRoute {
    Browse(Browse),
    Station(u64),
}

impl Route for RadioRoute {
    fn key(&self) -> String {
        match self {
            RadioRoute::Browse(b) => format!("browse-{}", b.key()),
            RadioRoute::Station(id) => format!("station-{id}"),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        if let Some(b) = key.strip_prefix("browse-") {
            return Browse::from_key(b).map(RadioRoute::Browse);
        }
        key.strip_prefix("station-")
            .and_then(|id| id.parse().ok())
            .map(RadioRoute::Station)
    }
    fn title(&self) -> String {
        match self {
            RadioRoute::Browse(b) => browse_title(b),
            RadioRoute::Station(id) => App::app()
                .library
                .station(*id)
                .map(|s| s.name)
                .unwrap_or_default(),
        }
    }
}

fn radio_tab(scene: Scene, app: App) -> impl Piece {
    let root = column((
        {
            let cuts: Vec<(usize, LocalizedText)> = vec![
                (0, res::str::radio_top()),
                (4, res::str::radio_trending()),
                (1, res::str::radio_countries()),
                (2, res::str::radio_tags()),
                (3, res::str::radio_languages()),
            ];
            list(
                items(move || cuts.clone(), |(i, _): &(usize, LocalizedText)| *i),
                move |slot: ItemSlot<(usize, LocalizedText), usize>| {
                    list_item(move || slot.field(|(_, t)| t.format()))
                        .icon(res::vectors::nav_browse)
                        .icon_tint(ACCENT)
                        .id_of(move || day::id!(radio_cut_row, slot.key()))
                },
            )
            .row_height(RowHeight::Uniform(48.0))
            .on_select(move |cut: usize| {
                scene.radio_cut.set(cut);
                let route = match cut {
                    4 => Browse::Trending,
                    _ => Browse::Top,
                };
                scene
                    .radio_tab_path
                    .update(|p| p.push(RadioRoute::Browse(route)));
            })
            .height(48.0 * 5.0)
            .id(day::id!(radio_cuts))
        },
        phone_stations(scene, app).grow(),
    ))
    .grow()
    .searchable(
        Search::new(scene.radio_query)
            .prompt(res::str::search_stations())
            .on_submit(move |q: &str| {
                let q = q.trim().to_string();
                if !q.is_empty() {
                    scene
                        .radio_tab_path
                        .update(|p| p.push(RadioRoute::Browse(Browse::Search(q))));
                }
            }),
    );
    nav_stack(scene.radio_tab_path, root)
        .title(res::str::nav_radio())
        .destination(move |route: &RadioRoute| match route {
            RadioRoute::Browse(b) => match (b, scene.radio_cut.get_untracked()) {
                (Browse::Top, 1) => facet_list(
                    scene,
                    app,
                    Place::Radio,
                    FacetKind::Countries,
                    day::id!(country_list),
                )
                .any(),
                (Browse::Top, 2) => facet_list(
                    scene,
                    app,
                    Place::Radio,
                    FacetKind::Tags,
                    day::id!(tag_list),
                )
                .any(),
                (Browse::Top, 3) => facet_list(
                    scene,
                    app,
                    Place::Radio,
                    FacetKind::Languages,
                    day::id!(language_list),
                )
                .any(),
                _ => browse_stations(scene, app, Place::Radio, b.clone()).any(),
            },
            RadioRoute::Station(id) => match app.library.station(*id) {
                Some(s) => station_page(app, scene, s).any(),
                None => column(()).any(),
            },
        })
        .id(day::id!(radio_stack))
}

/// The Podcasts tab's stack.
#[derive(Clone, Debug, PartialEq)]
pub enum PodRoute {
    Discover,
    Downloads,
    Show(u64),
    Episode(u64),
    Preview(String),
}

impl Route for PodRoute {
    fn key(&self) -> String {
        match self {
            PodRoute::Discover => "discover".into(),
            PodRoute::Downloads => "downloads".into(),
            PodRoute::Show(id) => format!("show-{id}"),
            PodRoute::Episode(id) => format!("episode-{id}"),
            PodRoute::Preview(feed) => format!("preview-{}", crate::radio::encode(feed)),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "discover" => return Some(PodRoute::Discover),
            "downloads" => return Some(PodRoute::Downloads),
            _ => {}
        }
        if let Some(id) = key.strip_prefix("show-") {
            return id.parse().ok().map(PodRoute::Show);
        }
        if let Some(id) = key.strip_prefix("episode-") {
            return id.parse().ok().map(PodRoute::Episode);
        }
        key.strip_prefix("preview-")
            .map(|f| PodRoute::Preview(percent_decode(f)))
    }
    fn title(&self) -> String {
        let app = App::app();
        match self {
            PodRoute::Discover => res::str::nav_discover().format(),
            PodRoute::Downloads => res::str::nav_downloads().format(),
            PodRoute::Show(id) => app
                .library
                .podcast(*id)
                .map(|p| p.title)
                .unwrap_or_default(),
            PodRoute::Episode(id) => app
                .library
                .episode(*id)
                .map(|e| e.title)
                .unwrap_or_default(),
            PodRoute::Preview(feed) => app
                .podcasts
                .show_for(feed)
                .map(|s| s.title)
                .unwrap_or_default(),
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn podcasts_tab(scene: Scene, app: App) -> impl Piece {
    let lib = app.library;
    let root = column((
        {
            let rows: Vec<(PodRoute, LocalizedText, day::VectorName)> = vec![
                (
                    PodRoute::Discover,
                    res::str::nav_discover(),
                    res::vectors::nav_discover,
                ),
                (
                    PodRoute::Downloads,
                    res::str::nav_downloads(),
                    res::vectors::downloaded,
                ),
            ];
            let keys: Vec<PodRoute> = rows.iter().map(|(r, _, _)| r.clone()).collect();
            list(
                items(
                    move || rows.clone(),
                    |(r, _, _): &(PodRoute, LocalizedText, day::VectorName)| r.key(),
                ),
                move |slot: ItemSlot<(PodRoute, LocalizedText, day::VectorName), String>| {
                    list_item(move || slot.field(|(_, t, _)| t.format()))
                        .icon(move || slot.field(|(_, _, v)| v.clone()))
                        .icon_tint(ACCENT)
                        .id_of(move || day::id!(pod_row, slot.key()))
                },
            )
            .row_height(RowHeight::Uniform(48.0))
            .on_select(move |key: String| {
                if let Some(r) = keys.iter().find(|r| r.key() == key) {
                    scene.pod_path.update(|p| p.push(r.clone()));
                }
            })
            .height(48.0 * 2.0)
            .id(day::id!(pod_root))
        },
        heading(res::str::nav_shows()),
        switch(
            move || lib.podcasts().count().min(4),
            move |n: &usize| {
                // No shows yet: room for the empty state's line and its two buttons.
                let height = if *n == 0 { 150.0 } else { 64.0 * *n as f64 };
                shows_pane(scene, app, Place::Pods).height(height).any()
            },
        ),
        heading(res::str::nav_episodes()),
        episode_list(
            scene,
            app,
            Place::Pods,
            lib.latest_episodes(),
            day::id!(episode_list),
            true,
        )
        .grow(),
    ))
    .grow()
    .toolbar(move || {
        vec![
            toolbar_button(day::id!(tb_add_podcast), res::str::cmd_add_podcast())
                .icon(Symbol::Add)
                .action(move || add_podcast_by_url(app, scene)),
            toolbar_button(day::id!(tb_refresh_shows), res::str::cmd_refresh())
                .icon(Symbol::Refresh)
                .action(move || app.podcasts.refresh_all()),
        ]
    });
    nav_stack(scene.pod_path, root)
        .title(res::str::nav_podcasts())
        .destination(move |route: &PodRoute| match route {
            PodRoute::Discover => discover_pane(scene, app, Place::Pods).any(),
            PodRoute::Downloads => episode_list(
                scene,
                app,
                Place::Pods,
                lib.downloaded_episodes(),
                day::id!(download_list),
                true,
            )
            .any(),
            PodRoute::Show(id) => show_page(app, scene, Place::Pods, *id).any(),
            PodRoute::Episode(id) => episode_page(app, scene, *id).any(),
            PodRoute::Preview(feed) => match app.podcasts.show_for(feed) {
                Some(s) => preview_page(app, scene, Place::Pods, s).any(),
                None => preview_page(
                    app,
                    scene,
                    Place::Pods,
                    DirectoryShow {
                        feed: feed.clone(),
                        title: feed.clone(),
                        ..Default::default()
                    },
                )
                .any(),
            },
        })
        .id(day::id!(pod_stack))
}

// --- library actions ------------------------------------------------------------------------------

/// Pick a folder and scan it.
pub fn add_folder(app: App) {
    day::task(async move {
        if let Some(dir) = open_folder().title(res::str::add_folder_title()).await
            && let Some(path) = dir.local_path()
        {
            app.library.scan_folder(path);
        }
    });
}

/// Pick files and copy them into the library.
pub fn import_files(app: App) {
    day::task(async move {
        let files = open_files()
            .title(res::str::import_files_title())
            .filter(
                res::str::audio_files().format(),
                day_piece_media::tags::AUDIO_EXTENSIONS,
            )
            .await;
        let paths: Vec<std::path::PathBuf> = files.iter().filter_map(|f| f.local_path()).collect();
        if !paths.is_empty() {
            app.library.import_files(paths);
        }
    });
}

// --- settings ------------------------------------------------------------------------------------------

pub fn library_group(app: App) -> Group_ {
    let lib = app.library;
    let mut g = Group_::new("library", res::str::settings_library())
        .icon(res::vectors::nav_songs)
        .subtitle(move || res::str::track_count(lib.track_count.get() as f64).format())
        .row(
            rows::info(res::str::settings_tracks(), move || {
                grouped(lib.track_count.get())
            })
            .id(day::id!(pref_tracks)),
        );
    // Folders exist where a folder can be picked; a phone's library is imported files only.
    if capability(Cap::FolderDialog) != Support::Unsupported {
        g = g.row(rows::custom(move || folders_rows(app))).row(
            rows::action(res::str::cmd_add_folder(), move || add_folder(app))
                .caption(res::str::add_folder_caption())
                .id(day::id!(pref_add_folder)),
        );
    }
    g.row(
        rows::action(res::str::cmd_import_files(), move || import_files(app))
            .caption(res::str::import_files_caption())
            .id(day::id!(pref_import)),
    )
    .row(
        rows::action(res::str::cmd_rescan(), move || {
            for id in Library::ids_of(&lib.folders()) {
                if let Some(f) = lib.container.get::<library::Folder>(id) {
                    lib.scan_folder(std::path::PathBuf::from(FolderFields::path(f).read()));
                }
            }
        })
        .busy(move || lib.scanning.get().is_some())
        .id(day::id!(pref_rescan)),
    )
    .row(
        rows::action(res::str::cmd_clear_history(), move || lib.clear_history())
            .destructive()
            .id(day::id!(pref_clear_history)),
    )
}

type Group_ = day_piece_settings::Group;

/// The watched folders, each with its count and a remove button.
fn folders_rows(app: App) -> impl Piece {
    let lib = app.library;
    let folders = lib.folders();
    column((each(
        lib.folders(),
        move |slot: ModelSlot<library::Folder>| {
            let id = move || slot.key();
            row((
                vector(res::vectors::folder).tint(GREY).frame(18.0, 18.0),
                column((
                    label(move || {
                        let p = FolderFields::path(slot).read();
                        std::path::Path::new(&p)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or(p)
                    })
                    .bold()
                    .single_line(),
                    label(move || FolderFields::path(slot).read())
                        .font(Font::Caption2)
                        .secondary()
                        .single_line(),
                ))
                .spacing(1.0)
                .align(HAlign::Leading)
                .grow(),
                label(move || res::str::track_count(slot.tracks().read() as f64).format())
                    .font(Font::Caption)
                    .secondary(),
                button(res::str::cmd_remove())
                    .plain()
                    .compact()
                    .action(move || {
                        let id = id();
                        day::task(async move {
                            if confirm(res::str::remove_folder_title())
                                .message(res::str::remove_folder_message())
                                .confirm_label(res::str::cmd_remove())
                                .destructive()
                                .await
                            {
                                lib.remove_folder(id);
                            }
                        });
                    })
                    .id_of(move || day::id!(folder_remove, slot.key())),
            ))
            .spacing(Space::S)
            .align(VAlign::Center)
            .padding(Insets::symmetric(0.0, 4.0))
        },
    ),))
    .align(HAlign::Leading)
    .apply_if(folders.ids_untracked().is_empty(), |p| p)
    .id(day::id!(pref_folders))
}

pub fn radio_group(app: App) -> Group_ {
    Group_::new("radio", res::str::settings_radio())
        .icon(res::vectors::nav_stations)
        .subtitle(move || app.radio.index_name.get())
        .row(
            rows::info(res::str::settings_radio_index(), move || {
                app.radio.index_name.get()
            })
            .id(day::id!(pref_radio_index)),
        )
        .row(
            rows::text(&crate::RADIO_SERVER, res::str::settings_radio_server())
                .placeholder("de2.api.radio-browser.info")
                .caption(res::str::settings_radio_server_caption())
                .id(day::id!(pref_radio_server)),
        )
        .row(rows::note(res::str::settings_radio_note()))
}

pub fn podcasts_group(app: App) -> Group_ {
    Group_::new("podcasts", res::str::settings_podcasts())
        .icon(res::vectors::nav_shows)
        .subtitle(move || app.podcasts.directory_name.get())
        .row(
            rows::info(res::str::settings_directory(), move || {
                app.podcasts.directory_name.get()
            })
            .id(day::id!(pref_directory)),
        )
        .row(
            rows::text(
                &crate::PODCAST_COUNTRY,
                res::str::settings_podcast_country(),
            )
            .placeholder("us")
            .caption(res::str::settings_podcast_country_caption())
            .id(day::id!(pref_podcast_country)),
        )
        .row(
            rows::toggle(
                &crate::PODCAST_REFRESH_AT_LAUNCH,
                res::str::settings_refresh_at_launch(),
            )
            .id(day::id!(pref_refresh_launch)),
        )
        .row(
            rows::action(res::str::cmd_refresh_podcasts(), move || {
                app.podcasts.refresh_all()
            })
            .busy(move || app.podcasts.refreshing.get() > 0)
            .id(day::id!(pref_refresh_podcasts)),
        )
        .row(rows::info(res::str::settings_last_error(), move || {
            app.podcasts
                .last_error
                .get()
                .unwrap_or_else(|| res::str::none().format())
        }))
}

pub fn scrobbling_group(app: App) -> Group_ {
    use crate::scrobble::{Account, Service};
    let s = app.scrobbling;
    let password: Signal<String> = Signal::new(String::new());
    Group_::new("scrobbling", res::str::settings_scrobbling())
        .icon(res::vectors::nav_history)
        .subtitle(move || match s.account.get() {
            Account::SignedIn(user) => user,
            _ => res::str::scrobble_off().format(),
        })
        .row(
            rows::choice(
                &crate::scrobble::SERVICE,
                res::str::settings_service(),
                [
                    (Service::Off, res::str::service_off()),
                    (Service::LastFm, res::str::service_lastfm()),
                    (Service::LibreFm, res::str::service_librefm()),
                    (Service::ListenBrainz, res::str::service_listenbrainz()),
                    (Service::Custom, res::str::service_custom()),
                ],
            )
            .id(day::id!(pref_service)),
        )
        .row(
            rows::text(
                &crate::scrobble::CUSTOM_BASE,
                res::str::settings_custom_base(),
            )
            .placeholder("https://libre.fm/2.0/")
            .available(|| crate::scrobble::SERVICE.get() == Service::Custom)
            .id(day::id!(pref_custom_base)),
        )
        .row(
            rows::text(&crate::scrobble::API_KEY, res::str::settings_api_key())
                .caption(res::str::settings_api_key_caption())
                .available(|| {
                    matches!(
                        crate::scrobble::SERVICE.get(),
                        Service::LastFm | Service::LibreFm | Service::Custom
                    )
                })
                .id(day::id!(pref_api_key)),
        )
        .row(
            rows::text(
                &crate::scrobble::API_SECRET,
                res::str::settings_api_secret(),
            )
            .secure()
            .available(|| {
                matches!(
                    crate::scrobble::SERVICE.get(),
                    Service::LastFm | Service::LibreFm | Service::Custom
                )
            })
            .id(day::id!(pref_api_secret)),
        )
        .row(
            rows::text(&crate::scrobble::USERNAME, res::str::settings_username())
                .available(|| {
                    matches!(
                        crate::scrobble::SERVICE.get(),
                        Service::LastFm | Service::LibreFm | Service::Custom
                    )
                })
                .id(day::id!(pref_username)),
        )
        .row(
            rows::custom(move || {
                row((
                    text_field(password)
                        .placeholder(move || {
                            if crate::scrobble::SERVICE.get() == Service::ListenBrainz {
                                res::str::settings_token().format()
                            } else {
                                res::str::settings_password().format()
                            }
                        })
                        .secure(true)
                        .id(day::id!(pref_password))
                        .grow(),
                    button(res::str::cmd_sign_in())
                        .prominent()
                        .enabled(move || {
                            !matches!(s.account.get(), Account::SigningIn)
                                && crate::scrobble::SERVICE.get() != Service::Off
                        })
                        .action(move || {
                            s.sign_in(
                                crate::scrobble::USERNAME.get_untracked(),
                                password.get_untracked(),
                            )
                        })
                        .id(day::id!(pref_sign_in)),
                ))
                .spacing(Space::S)
                .align(VAlign::Center)
            })
            .available(move || {
                !matches!(s.account.get(), Account::SignedIn(_))
                    && crate::scrobble::SERVICE.get() != Service::Off
            }),
        )
        .row(
            rows::info(res::str::settings_account(), move || {
                match s.account.get() {
                    Account::None => res::str::account_none().format(),
                    Account::SigningIn => res::str::account_signing_in().format(),
                    Account::SignedIn(u) => res::str::account_signed_in(u).format(),
                    Account::Failed(e) => res::str::account_failed(e).format(),
                }
            })
            .id(day::id!(pref_account)),
        )
        .row(
            rows::action(res::str::cmd_sign_out(), move || s.sign_out())
                .available(move || matches!(s.account.get(), Account::SignedIn(_)))
                .id(day::id!(pref_sign_out)),
        )
        .row(
            rows::toggle(
                &crate::scrobble::SCROBBLE_MUSIC,
                res::str::settings_scrobble_music(),
            )
            .id(day::id!(pref_scrobble_music)),
        )
        .row(
            rows::toggle(
                &crate::scrobble::SCROBBLE_RADIO,
                res::str::settings_scrobble_radio(),
            )
            .id(day::id!(pref_scrobble_radio)),
        )
        .row(
            rows::toggle(
                &crate::scrobble::SCROBBLE_PODCASTS,
                res::str::settings_scrobble_podcasts(),
            )
            .id(day::id!(pref_scrobble_podcasts)),
        )
        .row(
            rows::info(res::str::settings_pending(), move || {
                let pending = app.library.pending_scrobbles().count();
                match s.last_result.get() {
                    Some(Err(e)) => format!("{pending} · {e}"),
                    _ => pending.to_string(),
                }
            })
            .id(day::id!(pref_pending)),
        )
}
