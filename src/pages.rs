//! The pages: the content-list pane each section fills (Browse, Search, Library), the station
//! page they open, the Now Playing page, and Settings.
//!
//! Nothing here is a global. Every page function starts by asking the environment for the
//! window it is being built into (`Scene::ambient()`, https://daybrite.dev/docs/state) and the
//! process-wide `App::app()`, and passes both down.

use crate::catalog::{
    Facet, NoteFields, Scope, Station, StationFields, StreamRow, flag, station_id,
};
use crate::sync::{DEFAULT_BASE, SyncState};
use crate::{App, Scene, Section, has_toolbar, res};
use day::persistence::Query;
use day::prelude::*;
use day_piece_media::PlaybackState;
use day_piece_remote_image::remote_image_url;
use std::rc::Rc;

/// The accent the station rows and chips share.
const ACCENT: Color = Color {
    r: 0.23,
    g: 0.51,
    b: 0.96,
    a: 1.0,
};

fn tinted(c: Color, a: f64) -> Color {
    Color { a, ..c }
}

/// The genre names' Fluent keys, one per canonical tag in the catalog: named here so the keys
/// count as referenced (resolved by slug at runtime), and so a slug the catalog adds later
/// shows as itself rather than as a missing message.
const TAG_KEYS: [&str; 96] = [
    "tag_2000s",
    "tag_2010s",
    "tag_50s",
    "tag_60s",
    "tag_70s",
    "tag_80s",
    "tag_90s",
    "tag_adult_contemporary",
    "tag_alternative",
    "tag_ambient",
    "tag_anime",
    "tag_arabic_music",
    "tag_ballad",
    "tag_blues",
    "tag_bollywood",
    "tag_business",
    "tag_catholic",
    "tag_chillout",
    "tag_christian_music",
    "tag_classic_hits",
    "tag_classic_rock",
    "tag_classical",
    "tag_comedy",
    "tag_community_radio",
    "tag_country",
    "tag_culture",
    "tag_cumbia",
    "tag_dance",
    "tag_disco",
    "tag_downtempo",
    "tag_drum_and_bass",
    "tag_dubstep",
    "tag_edm",
    "tag_education",
    "tag_electronic",
    "tag_experimental",
    "tag_folk",
    "tag_funk",
    "tag_gospel",
    "tag_hard_rock",
    "tag_hardcore",
    "tag_hip_hop",
    "tag_hits",
    "tag_house",
    "tag_indie",
    "tag_instrumental",
    "tag_islamic",
    "tag_j_pop",
    "tag_jazz",
    "tag_k_pop",
    "tag_kids",
    "tag_latin",
    "tag_lifestyle",
    "tag_local_news",
    "tag_lofi",
    "tag_lounge",
    "tag_merengue",
    "tag_metal",
    "tag_new_wave",
    "tag_news",
    "tag_news_talk",
    "tag_oldies",
    "tag_opera",
    "tag_party",
    "tag_podcast",
    "tag_politics",
    "tag_pop",
    "tag_pop_rock",
    "tag_prog_rock",
    "tag_public_radio",
    "tag_punk",
    "tag_r_and_b",
    "tag_rap",
    "tag_reggae",
    "tag_reggaeton",
    "tag_regional_mexican",
    "tag_religious",
    "tag_retro",
    "tag_rock",
    "tag_romantic",
    "tag_salsa",
    "tag_ska",
    "tag_sleep",
    "tag_smooth_jazz",
    "tag_soft_rock",
    "tag_soul",
    "tag_soundtrack",
    "tag_sports",
    "tag_sports_talk",
    "tag_synthpop",
    "tag_talk",
    "tag_techno",
    "tag_top_40",
    "tag_trance",
    "tag_tropical",
    "tag_world",
];

/// The genre's display name, from the catalog's own key (`tag_classic_rock`); a slug the
/// catalog added after this build shows as itself.
pub(crate) fn tag_name(slug: &str) -> String {
    let key = format!("tag_{}", slug.replace('-', "_"));
    match TAG_KEYS.iter().find(|k| **k == key) {
        Some(k) => tr(k).format(),
        None => slug.replace('-', " "),
    }
}

/// The stream a station is played from: the resolved endpoint when the catalog found one.
pub(crate) fn stream_url(s: &Station) -> String {
    if s.url_resolved.is_empty() {
        s.url.clone()
    } else {
        s.url_resolved.clone()
    }
}

/// `53,790`: a count with thousands grouped, without a formatting crate.
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

/// The catalog spells countries out in full ("The United States Of America"); a row has room
/// for the everyday name.
pub(crate) fn short_country(name: &str) -> String {
    let n = name.strip_prefix("The ").unwrap_or(name);
    match n {
        "United States Of America" => "United States",
        "United Kingdom Of Great Britain And Northern Ireland" => "United Kingdom",
        "Russian Federation" => "Russia",
        "Islamic Republic Of Iran" => "Iran",
        "Republic Of Korea" => "South Korea",
        "Democratic People's Republic Of Korea" => "North Korea",
        "Taiwan, Republic Of China" => "Taiwan",
        "Viet Nam" => "Vietnam",
        "Bolivarian Republic Of Venezuela" => "Venezuela",
        "Plurinational State Of Bolivia" => "Bolivia",
        "United Republic Of Tanzania" => "Tanzania",
        "Lao People's Democratic Republic" => "Laos",
        "Syrian Arab Republic" => "Syria",
        "Republic Of Moldova" => "Moldova",
        "Democratic Republic Of The Congo" => "DR Congo",
        "Netherlands Kingdom Of The" | "Kingdom Of The Netherlands" => "Netherlands",
        "Czechia" | "Czech Republic" => "Czechia",
        other => other,
    }
    .to_string()
}

/// `MP3 · 128 kbps`: what a row says about the stream, minus the words nobody needs.
fn stream_line(codec: &str, bitrate: i64, hls: bool) -> String {
    let mut parts: Vec<String> = Vec::new();
    if hls {
        parts.push(res::str::station_hls().format());
    }
    if !codec.is_empty() && codec != "UNKNOWN" {
        parts.push(codec.to_string());
    }
    if bitrate > 0 {
        parts.push(res::str::station_bitrate(bitrate as f64).format());
    }
    parts.join(" · ")
}

/// `🇫🇷 France · MP3 · 128 kbps`: the second line of every station row.
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
    parts.join(" · ")
}

// --- the content-list pane -------------------------------------------------------------------

/// The middle column, built once per window and re-scoped by the section (the host keeps it
/// resident; `when` swaps what it shows). Until a catalog is attached, it shows the download
/// instead, whatever the section: there is nothing to list yet.
pub(crate) fn content_pane() -> impl Piece {
    let scene = Scene::ambient();
    let app = App::app();
    let ready = move || app.catalog.attached.get();
    column((
        when(move || !ready(), move || catalog_panel(app)),
        when(
            move || ready() && scene.section.get() == Section::Browse,
            move || {
                // The cut is the Browse root's control: drilling into a country pushes a
                // page that declares none, so it leaves with the list it belongs to
                // (https://daybrite.dev/docs/toolbars). It used to be a route test inside a
                // window-wide toolbar builder.
                browse_pane(scene).toolbar(move || {
                    if !scene.browse_path.get().is_empty() {
                        return Vec::new();
                    }
                    vec![
                        toolbar_segmented(
                            "tb-cut",
                            vec![
                                segment(res::str::browse_top()),
                                segment(res::str::browse_countries()),
                                segment(res::str::browse_genres()),
                                segment(res::str::browse_languages()),
                            ],
                            scene.browse_cut,
                        )
                        .placement(ToolbarPlacement::Principal),
                    ]
                })
            },
        ),
        when(
            move || ready() && scene.section.get() == Section::Search,
            move || search_pane(scene),
        ),
        when(
            move || ready() && scene.section.get() == Section::Library,
            move || library_pane(scene),
        ),
    ))
    .grow()
}

/// The first launch: the catalog is on its way, and this is how far along it is.
fn catalog_panel(app: App) -> impl Piece {
    let sync = app.sync;
    column((
        spacer(),
        vector(res::vectors::radio)
            .tint(ACCENT)
            .frame(72.0, 72.0)
            .padding(16.0)
            .background(tinted(ACCENT, 0.12))
            .corner_radius(20.0),
        label(res::str::catalog_fetching())
            .font(Font::Title3)
            .align(TextAlign::Center)
            .id("catalog-title"),
        label(move || sync_line(&sync.state.get()))
            .font(Font::Caption)
            .secondary()
            .align(TextAlign::Center)
            .max_width(320.0)
            .id("catalog-state"),
        when(
            move || matches!(sync.state.get(), SyncState::Downloading { .. }),
            move || {
                progress(move || sync_fraction(&sync.state.get()))
                    .frame(240.0, 8.0)
                    .id("catalog-progress")
            },
        ),
        when(
            move || matches!(sync.state.get(), SyncState::Failed(_)),
            move || {
                button(res::str::catalog_retry())
                    .prominent()
                    .action(move || sync.check(app.catalog))
                    .id("catalog-retry")
            },
        ),
        spacer(),
    ))
    .spacing(12.0)
    .align(HAlign::Center)
    .grow()
    .padding(24.0)
}

/// The sync, as a line for the download panel and Settings.
fn sync_line(state: &SyncState) -> String {
    match state {
        SyncState::Idle => res::str::catalog_idle().format(),
        SyncState::Checking => res::str::catalog_checking().format(),
        SyncState::Downloading { received, total } => {
            let mb = |b: u64| format!("{:.1}", b as f64 / 1_048_576.0);
            let progress = if *total > 0 {
                format!("{} / {} MB", mb(*received), mb(*total))
            } else {
                format!("{} MB", mb(*received))
            };
            res::str::catalog_downloading(progress).format()
        }
        SyncState::Verifying => res::str::catalog_verifying().format(),
        SyncState::UpToDate => res::str::catalog_up_to_date().format(),
        SyncState::Failed(reason) => res::str::catalog_failed(reason.clone()).format(),
    }
}

fn sync_fraction(state: &SyncState) -> f64 {
    match state {
        SyncState::Downloading { received, total } if *total > 0 => {
            (*received as f64 / *total as f64).clamp(0.0, 1.0)
        }
        _ => 0.0,
    }
}

/// Browse: a push stack (https://daybrite.dev/docs/navigation) whose root is the cut's
/// list (the top stations, or the countries, genres, and languages) and whose pushed page
/// is the stations of the country, genre, or language picked. On a phone the pushes ride the
/// tab's navigation controller with its back button; on a desktop the pane carries a back
/// header. The cut is chosen on the root page's chrome, on every platform.
fn browse_pane(scene: Scene) -> impl Piece {
    nav_stack(scene.browse_path, browse_root(scene))
        .destination(move |scope: &Scope| scope_page(scene, scope.clone()))
        .id("browse-stack")
}

/// The root of the Browse stack: the cut, and its list.
fn browse_root(scene: Scene) -> impl Piece {
    let app = App::app();
    column((
        when(
            move || scene.browse_cut.get() == 0,
            move || {
                let query = app.catalog.stations_for(|| Scope::Top);
                station_list(scene, query, "browse-list").grow()
            },
        ),
        when(
            move || scene.browse_cut.get() == 1,
            move || {
                facet_list(
                    scene,
                    app.catalog.countries,
                    |f| {
                        format!("{} {}", flag(&f.key), short_country(&f.name))
                            .trim()
                            .to_string()
                    },
                    Scope::Country,
                    "country-list",
                )
            },
        ),
        when(
            move || scene.browse_cut.get() == 2,
            move || {
                facet_list(
                    scene,
                    app.catalog.tags,
                    |f| tag_name(&f.key),
                    Scope::Tag,
                    "tag-list",
                )
            },
        ),
        when(
            move || scene.browse_cut.get() == 3,
            move || {
                facet_list(
                    scene,
                    app.catalog.languages,
                    |f| language_name(&f.key),
                    Scope::Language,
                    "language-list",
                )
            },
        ),
    ))
    .grow()
}

/// A pushed page of the Browse stack: the stations of one country, genre, or language.
fn scope_page(scene: Scene, scope: Scope) -> impl Piece {
    let app = App::app();
    let query = app.catalog.stations_for(move || scope.clone());
    station_list(scene, query, "browse-list").grow()
}

/// A `Scope` as a route segment of the Browse stack (https://daybrite.dev/docs/navigation):
/// `country-FR`, `tag-classic-rock`, `language-en`; the title is what the navigation bar and
/// the desktop back header show for the pushed page.
impl Route for Scope {
    fn key(&self) -> String {
        match self {
            Scope::Top => "top".into(),
            Scope::Country(c) => format!("country-{c}"),
            Scope::Tag(t) => format!("tag-{t}"),
            Scope::Language(l) => format!("language-{l}"),
            Scope::Search(q) => format!("search-{q}"),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        if key == "top" {
            return Some(Scope::Top);
        }
        if let Some(c) = key.strip_prefix("country-") {
            return Some(Scope::Country(c.to_string()));
        }
        if let Some(t) = key.strip_prefix("tag-") {
            return Some(Scope::Tag(t.to_string()));
        }
        if let Some(l) = key.strip_prefix("language-") {
            return Some(Scope::Language(l.to_string()));
        }
        key.strip_prefix("search-")
            .map(|q| Scope::Search(q.to_string()))
    }
    fn title(&self) -> String {
        pick_title(App::app(), Some(self.clone()))
    }
}

fn pick_title(app: App, pick: Option<Scope>) -> String {
    match pick {
        Some(Scope::Country(code)) => {
            let name = app.catalog.countries.with_untracked(|all| {
                all.iter()
                    .find(|f| f.key == code)
                    .map(|f| short_country(&f.name))
            });
            format!("{} {}", flag(&code), name.unwrap_or(code))
                .trim()
                .to_string()
        }
        Some(Scope::Tag(slug)) => tag_name(&slug),
        Some(Scope::Language(slug)) => language_name(&slug),
        _ => res::str::browse_top_title().format(),
    }
}

/// A language's display name: the catalog carries ISO codes and some plain names; both show
/// as they are, upper-cased when a code.
fn language_name(slug: &str) -> String {
    if slug.len() == 2 {
        slug.to_ascii_uppercase()
    } else {
        let mut c = slug.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => String::new(),
        }
    }
}

/// One list of countries, genres, or languages: the facets computed at attach, each row a
/// name and a count, a tap opening the pick.
fn facet_list(
    scene: Scene,
    source: Signal<Vec<Facet>>,
    name: impl Fn(&Facet) -> String + Copy + 'static,
    pick: impl Fn(String) -> Scope + 'static,
    id: &'static str,
) -> impl Piece {
    list(
        items(move || source.get(), |f: &Facet| f.key.clone()),
        move |slot: ItemSlot<Facet, String>| {
            facet_row(
                move || slot.with(|f| name(f)),
                move || slot.with(|f| f.stations),
            )
        },
    )
    .row_height(RowHeight::Uniform(44.0))
    .on_select(move |key: String| scene.browse_path.set(vec![pick(key)]))
    .id(id)
    .grow()
}

/// One country, genre, or language: its name and how many stations it opens onto.
fn facet_row(name: impl Fn() -> String + 'static, count: impl Fn() -> i64 + 'static) -> impl Piece {
    row((
        label(name).grow(),
        label(move || res::str::browse_count(count() as f64).format())
            .font(Font::Caption)
            .secondary(),
    ))
    .spacing(10.0)
    .padding(Insets {
        top: 6.0,
        leading: 14.0,
        bottom: 6.0,
        trailing: 14.0,
    })
}

/// Search: a field and the stations that match, live.
fn search_pane(scene: Scene) -> impl Piece {
    let app = App::app();
    let query = app
        .catalog
        .stations_for(move || Scope::Search(scene.query.get()));
    column((
        text_field(scene.query)
            .placeholder(res::str::search_hint())
            .id("search-field")
            .padding(Insets {
                top: 8.0,
                leading: 12.0,
                bottom: 4.0,
                trailing: 12.0,
            }),
        // The hint while the field is empty, the count of nothing when a search finds none.
        when(
            move || scene.query.get().trim().is_empty(),
            move || {
                label(move || {
                    res::str::search_empty(app.catalog.station_count.get() as f64).format()
                })
                .font(Font::Caption)
                .secondary()
                .padding(Insets {
                    top: 0.0,
                    leading: 14.0,
                    bottom: 4.0,
                    trailing: 14.0,
                })
                .id("search-hint")
            },
        ),
        {
            let q = query.clone();
            when(
                move || !scene.query.get().trim().is_empty() && q.count() == 0,
                move || {
                    label(move || res::str::search_none(scene.query.get()).format())
                        .secondary()
                        .padding(14.0)
                        .id("search-none")
                },
            )
        },
        station_list(scene, query, "search-list").grow(),
    ))
    .grow()
}

/// Library: favorites and recents, in the order they were added or played, filtered by one of
/// the listener's own tags when they have made any.
fn library_pane(scene: Scene) -> impl Piece {
    let app = App::app();
    let cuts = [
        res::str::library_favorites().format(),
        res::str::library_recents().format(),
    ];
    // The listener's tag vocabulary: the distinct names across every tagging, live. Shared
    // by the filter, the picker, and the queries, so it is one closure behind an `Rc`.
    let all = app.catalog.all_taggings();
    let tag_names: Rc<dyn Fn() -> Vec<String>> = Rc::new(move || {
        let mut names: Vec<String> = all
            .ids()
            .iter()
            .filter_map(|id| app.catalog.tagging_name(id.handle()))
            .collect();
        names.sort();
        names.dedup();
        names
    });
    // A vocabulary that shrinks under the filter drops the filter.
    let names_len = tag_names.clone();
    watch(
        move || names_len().len(),
        move |n, _| {
            if scene.library_tag.get_untracked() > *n {
                scene.library_tag.set(0);
            }
        },
    );
    let names_of = tag_names.clone();
    let tag_of: Rc<dyn Fn() -> Option<String>> = Rc::new(move || match scene.library_tag.get() {
        0 => None,
        i => names_of().get(i - 1).cloned(),
    });
    let names_shown = tag_names.clone();
    let names_options = tag_names.clone();
    let (tag_f, tag_r) = (tag_of.clone(), tag_of.clone());
    let favorites = app.catalog.favorites_tagged(move || tag_f());
    let recents = app.catalog.recents_tagged(move || tag_r());
    // The listed ids, by cut. A favorite and a recent of one station share an id
    // (`station_id`), so a row resolves the same way in either cut.
    let ids = {
        let (f, r) = (favorites.clone(), recents.clone());
        move || -> Vec<u64> {
            if scene.library_cut.get() == 0 {
                f.ids().iter().map(|id| id.handle()).collect()
            } else {
                r.ids().iter().map(|id| id.handle()).collect()
            }
        }
    };
    let ids_empty = ids.clone();
    let ids_rows = ids.clone();
    column((
        picker(cuts, scene.library_cut)
            .segmented()
            .id("library-cut")
            .padding(Insets {
                top: 8.0,
                leading: 12.0,
                bottom: 4.0,
                trailing: 12.0,
            }),
        when(
            move || !names_shown().is_empty(),
            move || {
                let names = names_options.clone();
                picker([res::str::library_all_tags().format()], scene.library_tag)
                    .options_reactive(move || {
                        let mut options = vec![res::str::library_all_tags().format()];
                        options.extend(names());
                        options
                    })
                    .id("library-tag")
                    .padding(Insets {
                        top: 0.0,
                        leading: 12.0,
                        bottom: 4.0,
                        trailing: 12.0,
                    })
            },
        ),
        when(
            move || ids_empty().is_empty(),
            move || {
                label(move || {
                    if scene.library_cut.get() == 0 {
                        res::str::library_no_favorites().format()
                    } else {
                        res::str::library_no_recents().format()
                    }
                })
                .secondary()
                .padding(14.0)
                .id("library-empty")
            },
        ),
        list(
            items(ids_rows, |id: &u64| *id),
            move |slot: ItemSlot<u64, u64>| library_row(app, move || slot.get()),
        )
        .row_height(RowHeight::Uniform(56.0))
        .on_selection({
            let ids = ids.clone();
            move |rows: Vec<u64>| match rows.first().and_then(|id| library_station(app, *id)) {
                Some(s) => {
                    let queue: Vec<Vec<u8>> = ids()
                        .iter()
                        .filter_map(|id| {
                            app.catalog
                                .favorite_uuid(*id)
                                .or_else(|| app.catalog.recent_uuid(*id))
                        })
                        .collect();
                    scene.open_from(s.id, queue);
                }
                None => scene.clear_selection(),
            }
        })
        .selected_rows({
            let ids = ids.clone();
            move || {
                scene
                    .selected
                    .get()
                    .and_then(|id| app.catalog.station(id))
                    .map(|s| station_id(&s.uuid))
                    .and_then(|id| ids().iter().position(|k| *k == id))
                    .into_iter()
                    .collect()
            }
        })
        .id("library-list")
        .grow(),
    ))
    .grow()
}

/// The station a library row names, if this catalog still carries it.
fn library_station(app: App, id: u64) -> Option<Station> {
    app.catalog
        .favorite_uuid(id)
        .or_else(|| app.catalog.recent_uuid(id))
        .and_then(|uuid| app.catalog.station_by_uuid(&uuid))
}

/// A library row: the station a favorite or recent names, read from the catalog on every bind;
/// the row set is small, and a station the catalog dropped still shows as such rather than
/// vanishing with its note and tags.
fn library_row(app: App, id: impl Fn() -> u64 + Copy + 'static) -> impl Piece {
    row((
        column((
            label(move || {
                library_station(app, id())
                    .map(|s| s.name)
                    .unwrap_or_else(|| res::str::library_missing().format())
            })
            .font(Font::Headline),
            label(move || {
                library_station(app, id())
                    .map(|s| station_meta(&s))
                    .unwrap_or_default()
            })
            .font(Font::Caption)
            .secondary(),
        ))
        .spacing(2.0)
        .align(HAlign::Leading)
        .grow(),
        when(
            move || {
                app.player
                    .current_uuid()
                    .is_some_and(|u| station_id(&u) == id())
            },
            move || vector(res::vectors::play).tint(ACCENT).frame(16.0, 16.0),
        ),
    ))
    .spacing(10.0)
    .padding(Insets {
        top: 8.0,
        leading: 14.0,
        bottom: 8.0,
        trailing: 14.0,
    })
}

// --- station lists and rows -------------------------------------------------------------------

/// The one station list every pane uses: a live query as the row source, rows faulting in as
/// they scroll, the selection two-way with the window's `selected`.
fn station_list(scene: Scene, query: Query<Station>, id: &'static str) -> impl Piece {
    let app = App::app();
    let positions = query.clone();
    let order = query.clone();
    list(query, move |slot: ModelSlot<Station>| {
        station_row(app, slot)
    })
    .row_height(RowHeight::Uniform(56.0))
    // `on_selection`, not `on_select`: only the full set can report a cleared selection. The
    // whole list rides along as the player's queue, so Next and Previous walk it.
    .on_selection(move |rows: Vec<Elem<Station>>| match rows.first() {
        Some(s) => {
            let ids: Vec<u64> = order.ids_untracked().iter().map(|id| id.handle()).collect();
            scene.open_from(s.key(), app.catalog.uuids_for(&ids));
        }
        None => scene.clear_selection(),
    })
    // Two-way: a station opened any other way highlights in the list rather than leaving
    // the page and the list disagreeing about what is open.
    .selected_rows(move || {
        scene
            .selected
            .get()
            .and_then(|id| positions.ids().iter().position(|k| k.handle() == id))
            .into_iter()
            .collect()
    })
    .separators(true)
    .id(id)
}

/// One station row: its name, where it is and how it streams, and a marker when it is on air.
///
/// Every read is a per-FIELD tracked read inside a reactive closure, so the recycling list can
/// rebind this physical row to another station with one write (https://daybrite.dev/docs/list).
fn station_row(app: App, slot: ModelSlot<Station>) -> impl Piece {
    let meta = move || {
        let mut parts: Vec<String> = Vec::new();
        let code = slot.countrycode().read();
        let country = slot.country().read();
        if !country.is_empty() {
            parts.push(
                format!("{} {}", flag(&code), short_country(&country))
                    .trim()
                    .to_string(),
            );
        }
        let stream = stream_line(
            &slot.codec().read(),
            slot.bitrate().read(),
            slot.hls().read(),
        );
        if !stream.is_empty() {
            parts.push(stream);
        }
        let tags: Vec<String> = app
            .catalog
            .station_tags(&slot.uuid().read())
            .iter()
            .take(2)
            .map(|t| tag_name(t))
            .collect();
        if !tags.is_empty() {
            parts.push(tags.join(", "));
        }
        parts.join(" · ")
    };
    row((
        column((
            label(move || slot.name().read()).font(Font::Headline),
            label(meta).font(Font::Caption).secondary(),
        ))
        .spacing(2.0)
        .align(HAlign::Leading)
        .grow(),
        when(
            move || app.player.is_current(&slot.uuid().read()),
            move || vector(res::vectors::play).tint(ACCENT).frame(16.0, 16.0),
        ),
    ))
    .spacing(10.0)
    .padding(Insets {
        top: 8.0,
        leading: 14.0,
        bottom: 8.0,
        trailing: 14.0,
    })
}

// --- the station page --------------------------------------------------------------------------

/// The detail for Browse, Search, and Library: the selected station's page, or the empty state.
///
/// `each` over a nought-or-one list rather than a conditional, because the page has to be
/// rebuilt when the selection changes, not merely shown and hidden.
pub(crate) fn station_page() -> impl Piece {
    let scene = Scene::ambient();
    let app = App::app();
    column((
        when(
            move || {
                scene
                    .selected
                    .get()
                    .is_none_or(|id| app.catalog.station(id).is_none())
            },
            || {
                column((
                    spacer(),
                    label(res::str::station_none())
                        .font(Font::Title3)
                        .secondary()
                        .align(TextAlign::Center)
                        .id("station-none"),
                    spacer(),
                ))
                .align(HAlign::Center)
                .grow()
                .padding(24.0)
                .grow()
            },
        ),
        each(
            items(
                move || {
                    scene
                        .selected
                        .get()
                        .filter(|id| app.catalog.station(*id).is_some())
                        .into_iter()
                        .collect::<Vec<u64>>()
                },
                |id: &u64| *id,
            ),
            move |slot: ItemSlot<u64, u64>| station_detail(app, scene, slot.key()),
        ),
    ))
    .grow()
}

/// One station's page: what it is, how to play it, what the catalog knows about it, and what
/// the listener has written about it.
fn station_detail(app: App, scene: Scene, id: u64) -> impl Piece {
    let Some(s) = app.catalog.station(id) else {
        return column(()).any();
    };
    let player = app.player;
    let uuid = s.uuid.clone();
    // Mirrors of one stream (same codec, bitrate, and kind) are one button: the listener picks
    // a quality, not a host.
    let mut streams: Vec<StreamRow> = Vec::new();
    for st in app.catalog.streams(&uuid) {
        if !streams
            .iter()
            .any(|s| s.codec == st.codec && s.bitrate == st.bitrate && s.hls == st.hls)
        {
            streams.push(st);
        }
    }
    let main_url = stream_url(&s);
    let place = {
        let mut parts = Vec::new();
        let f = flag(&s.countrycode);
        if !f.is_empty() {
            parts.push(f);
        }
        if !s.country.is_empty() {
            parts.push(short_country(&s.country));
        }
        if !s.state.is_empty() {
            parts.push(s.state.clone());
        }
        parts.join(" · ")
    };
    let genres = app.catalog.station_tags(&uuid);
    let languages: Vec<String> = s
        .languagecodes
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(language_name)
        .collect();

    // One Play per stream variant when the station publishes several, else one for the station.
    let play_buttons: Vec<AnyPiece> = if streams.len() > 1 {
        streams
            .iter()
            .enumerate()
            .map(|(i, st)| {
                let url = if st.url_resolved.is_empty() {
                    st.url.clone()
                } else {
                    st.url_resolved.clone()
                };
                let label_text = if st.label.is_empty() {
                    stream_line(&st.codec, st.bitrate, st.hls)
                } else {
                    st.label.clone()
                };
                let station = s.clone();
                // The first variant is the station's Play for scripts and shortcuts alike.
                let b = button(res::str::station_play_variant(label_text))
                    .action(move || {
                        player.play_station(
                            station.clone(),
                            url.clone(),
                            scene.queue.get_untracked(),
                        )
                    })
                    .id(if i == 0 {
                        "station-play".to_string()
                    } else {
                        format!("station-play-{i}")
                    });
                if i == 0 {
                    b.prominent().any()
                } else {
                    b.bordered().any()
                }
            })
            .collect()
    } else {
        let station = s.clone();
        vec![
            button(res::str::station_play())
                .prominent()
                .action(move || {
                    player.play_station(
                        station.clone(),
                        main_url.clone(),
                        scene.queue.get_untracked(),
                    )
                })
                .id("station-play")
                .any(),
        ]
    };
    let fav_label = {
        let uuid = uuid.clone();
        move || {
            if app.catalog.is_favorite(&uuid) {
                res::str::cmd_unfavorite().format()
            } else {
                res::str::cmd_favorite().format()
            }
        }
    };
    let fav_uuid = uuid.clone();
    let state_uuid = uuid.clone();

    scroll(
        column((
            label(place)
                .font(Font::Caption)
                .secondary()
                .id("station-place"),
            label(s.name.clone()).font(Font::Title).id("station-name"),
            // What the player says about this station while it is the one on air.
            when(
                move || player.is_current(&state_uuid),
                move || {
                    label(move || state_line(player.state.get()))
                        .font(Font::Caption)
                        .color(ACCENT)
                        .id("station-state")
                },
            ),
            row(PieceVec(play_buttons))
                .spacing(8.0)
                .fit(RowFit::Wrap { run_spacing: 8.0 }),
            row((
                button(fav_label)
                    .bordered()
                    .action(move || app.catalog.toggle_favorite(&fav_uuid))
                    .id("station-favorite"),
                when(
                    {
                        let has = !s.homepage.is_empty();
                        move || has
                    },
                    {
                        let home = s.homepage.clone();
                        move || {
                            link(res::str::station_homepage(), home.clone()).id("station-homepage")
                        }
                    },
                ),
            ))
            .spacing(12.0)
            .fit(RowFit::Wrap { run_spacing: 8.0 }),
            chips(genres),
            form((
                section((
                    labeled(
                        res::str::station_stream(),
                        label(stream_line(&s.codec, s.bitrate, s.hls)),
                    ),
                    labeled(
                        res::str::station_status(),
                        label(if s.online {
                            res::str::station_online().format()
                        } else {
                            res::str::station_offline().format()
                        }),
                    ),
                    labeled(res::str::station_votes(), label(grouped(s.votes)).tabular()),
                ))
                .title(res::str::station_stream()),
                section((
                    labeled(res::str::station_languages(), label(languages.join(", "))),
                    labeled(res::str::station_nature(), label(s.nature.clone())),
                ))
                .title(res::str::station_about()),
            )),
            // The listener's own words: a note, and tags the Library filters by. Both live in
            // the app's store, keyed by the catalog's uuid, and outlive every catalog update.
            label(res::str::station_notes())
                .font(Font::Headline)
                .id("station-notes-title"),
            text_area(app.catalog.note(&uuid).text())
                .placeholder(res::str::station_notes_hint())
                .min_lines(3)
                .max_lines(8)
                .id("station-note"),
            label(res::str::station_tags_title()).font(Font::Headline),
            user_tags(app, scene, uuid.clone()),
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(20.0),
    )
    .grow()
    .any()
}

/// The listener's tags on one station: a chip per tag that removes itself, and a field to add
/// one. The chips are `each` over the live tagging query, so a tag added here shows at once.
fn user_tags(app: App, scene: Scene, uuid: Vec<u8>) -> impl Piece {
    let taggings = app.catalog.taggings(&uuid);
    let names = move || -> Vec<String> {
        taggings
            .ids()
            .iter()
            .filter_map(|id| app.catalog.tagging_name(id.handle()))
            .collect()
    };
    let chip_uuid = uuid.clone();
    let add_uuid = uuid.clone();
    column((
        row((each(
            items(names, |t: &String| t.clone()),
            move |slot: ItemSlot<String, String>| {
                let tag = slot.key();
                let uuid = chip_uuid.clone();
                let remove_tag = tag.clone();
                button(format!("{tag} ×"))
                    .bordered()
                    .action(move || app.catalog.remove_tag(&uuid, &remove_tag))
                    .id(format!("station-user-tag-{}", tag.replace(' ', "-")))
            },
        ),))
        .spacing(6.0)
        .fit(RowFit::Wrap { run_spacing: 6.0 })
        .id("station-user-tags"),
        row((
            text_field(scene.new_tag)
                .placeholder(res::str::station_tag_hint())
                .id("station-tag-field")
                .grow(),
            button(res::str::station_tag_add())
                .bordered()
                .action(move || {
                    let tag = scene.new_tag.get_untracked();
                    app.catalog.add_tag(&add_uuid, &tag);
                    // Cleared a beat later: the field commits its text as the button takes
                    // the focus, and a clear in the same turn loses to that write-back.
                    day::task(async move {
                        day::sleep(80).await;
                        scene.new_tag.set(String::new());
                    });
                })
                .id("station-tag-add"),
        ))
        .spacing(8.0)
        .max_width(360.0),
    ))
    .spacing(8.0)
    .align(HAlign::Leading)
}

/// The genre chips: one rounded label per tag, tinted with the app's accent.
fn chips(tags: Vec<String>) -> impl Piece {
    let pieces: Vec<AnyPiece> = tags
        .iter()
        .map(|t| {
            label(tag_name(t))
                .font(Font::Caption)
                .color(ACCENT)
                .padding(Insets {
                    top: 3.0,
                    leading: 10.0,
                    bottom: 3.0,
                    trailing: 10.0,
                })
                .background(tinted(ACCENT, 0.14))
                .corner_radius(11.0)
                .any()
        })
        .collect();
    row(PieceVec(pieces))
        .spacing(6.0)
        .fit(RowFit::Wrap { run_spacing: 6.0 })
        .id("station-tags")
}

/// What the player says, as a line: `Live`, `Connecting…`, or why it could not play.
fn state_line(state: PlaybackState) -> String {
    match state {
        PlaybackState::Idle => res::str::playing_state_idle().format(),
        PlaybackState::Loading => res::str::playing_state_loading().format(),
        PlaybackState::Playing => res::str::playing_state_playing().format(),
        PlaybackState::Paused => res::str::playing_state_paused().format(),
        PlaybackState::Ended => res::str::playing_state_ended().format(),
        PlaybackState::Error(reason) => res::str::playing_state_error(reason).format(),
    }
}

// --- now playing ---------------------------------------------------------------------------------

/// The Now Playing section, laid out the way a music app's is: the station's art, its name
/// and the track the stream names, and the transport (Previous, Play/Pause, Next) with the
/// favorite star and the volume below. On a desktop the same commands also ride the window
/// toolbar; here is where a phone has them.
pub(crate) fn playing_page() -> impl Piece {
    let scene = Scene::ambient();
    let app = App::app();
    let player = app.player;
    let station = move || player.current.get();
    let on_air = move || player.current_uuid().is_some();
    column((
        spacer(),
        // The art, rebuilt per station: the catalog's favicon when the station has one, the
        // app's own tile when not or until it loads.
        each(
            items(
                move || {
                    station()
                        .map(|s| s.uuid)
                        .into_iter()
                        .collect::<Vec<Vec<u8>>>()
                },
                |u: &Vec<u8>| u.clone(),
            ),
            move |_slot: ItemSlot<Vec<u8>, Vec<u8>>| artwork(player),
        ),
        when(
            move || station().is_none(),
            || art_tile().id("playing-art-empty"),
        ),
        label(move || {
            station()
                .map(|s| s.name)
                .unwrap_or_else(|| res::str::playing_nothing().format())
        })
        .font(Font::Title2)
        .align(TextAlign::Center)
        .max_width(480.0)
        .id("playing-name"),
        // The track: the title the stream names, the artist and album under it, else what
        // the player is doing.
        label(move || match (station(), player.track.get()) {
            (Some(_), Some(t)) if !t.title.is_empty() => t.title,
            (Some(_), _) => state_line(player.state.get()),
            (None, _) => res::str::playing_pick().format(),
        })
        .font(Font::Headline)
        .align(TextAlign::Center)
        .max_width(480.0)
        .id("playing-track"),
        label(move || match (station(), player.track.get()) {
            (Some(_), Some(t)) => {
                let mut parts = Vec::new();
                if !t.artist.is_empty() {
                    parts.push(t.artist);
                }
                if !t.album.is_empty() {
                    parts.push(t.album);
                }
                if parts.is_empty() {
                    state_line(player.state.get())
                } else {
                    parts.join(" · ")
                }
            }
            (Some(s), None) => {
                if player.is_on_air() {
                    res::str::playing_track_none().format()
                } else {
                    station_meta(&s)
                }
            }
            (None, _) => String::new(),
        })
        .font(Font::Caption)
        .secondary()
        .align(TextAlign::Center)
        .max_width(480.0)
        .id("playing-artist"),
        // The transport: Previous | Play/Pause | Next, as tinted vector glyphs.
        row((
            transport_glyph(res::vectors::skip_previous, 40.0, move || {
                player.has_previous()
            })
            .on_tap(move || player.step(-1))
            .id("playing-previous"),
            column((when(
                move || player.is_on_air(),
                || {
                    vector(res::vectors::pause)
                        .tint(Color::hex(0xFFFFFF))
                        .frame(44.0, 44.0)
                },
            )
            .otherwise(|| {
                vector(res::vectors::play_arrow)
                    .tint(Color::hex(0xFFFFFF))
                    .frame(44.0, 44.0)
            }),))
            .padding(16.0)
            .background(move || {
                if on_air() {
                    ACCENT
                } else {
                    tinted(ACCENT, 0.45)
                }
            })
            .corner_radius(38.0)
            .on_tap(move || player.toggle())
            .id("playing-toggle"),
            transport_glyph(res::vectors::skip_next, 40.0, move || player.has_next())
                .on_tap(move || player.step(1))
                .id("playing-next"),
        ))
        .spacing(28.0)
        .align(VAlign::Center),
        // The star and the volume.
        row((
            column((when(
                move || player.is_favorite(),
                || {
                    vector(res::vectors::star)
                        .tint(Color::hex(0xF59E0B))
                        .frame(28.0, 28.0)
                },
            )
            .otherwise(move || {
                vector(res::vectors::star_outline)
                    .tint(move || {
                        if on_air() {
                            Color::hex(0x9CA3AF)
                        } else {
                            tinted(Color::hex(0x9CA3AF), 0.4)
                        }
                    })
                    .frame(28.0, 28.0)
            }),))
            .on_tap(move || player.toggle_favorite())
            .id("playing-favorite"),
            vector(res::vectors::volume_down)
                .tint(Color::hex(0x9CA3AF))
                .frame(22.0, 22.0),
            // Tall enough for Material 3's slider, whose handle stands above its track.
            slider(player.volume)
                .range(0.0..=1.0)
                .id("playing-volume")
                .frame(200.0, 44.0),
            vector(res::vectors::volume_up)
                .tint(Color::hex(0x9CA3AF))
                .frame(22.0, 22.0),
        ))
        .spacing(12.0)
        .align(VAlign::Center),
        // Where the station is and how it streams: the row's second line, here as a footnote.
        label(move || station().map(|s| station_meta(&s)).unwrap_or_default())
            .font(Font::Caption)
            .secondary()
            .align(TextAlign::Center)
            .max_width(420.0)
            .id("playing-meta"),
        when(
            move || on_air() && !has_toolbar(),
            move || {
                button(res::str::playing_open())
                    .bordered()
                    .action(move || {
                        // The page is keyed by the current catalog's rowid: look the station
                        // up again rather than trusting the copy the player holds.
                        if let Some(s) = player
                            .current_uuid()
                            .and_then(|u| app.catalog.station_by_uuid(&u))
                        {
                            scene.show(s.id);
                        }
                    })
                    .id("playing-open")
            },
        ),
        button(res::str::cmd_stop())
            .bordered()
            .enabled(on_air)
            .action(move || player.stop.notify())
            .id("playing-stop"),
        spacer(),
    ))
    .spacing(16.0)
    .align(HAlign::Center)
    .grow()
    .padding(24.0)
}

/// A transport glyph, dimmed while its command has nowhere to go.
fn transport_glyph(
    name: day::VectorName,
    size: f64,
    enabled: impl Fn() -> bool + Copy + 'static,
) -> impl Piece {
    vector(name)
        .tint(move || {
            if enabled() {
                Color::hex(0x9CA3AF)
            } else {
                tinted(Color::hex(0x9CA3AF), 0.35)
            }
        })
        .frame(size, size)
}

/// The station's logo, from the catalog's favicon URL, a rounded tile the size of album art.
/// Built once per station (`each` above), since the fetch is one per build.
fn artwork(player: crate::Player) -> impl Piece {
    let favicon = player
        .current
        .with_untracked(|s| s.as_ref().map(|s| s.favicon.clone()))
        .unwrap_or_default();
    if !cfg!(any(feature = "dom", feature = "arkui"))
        && (favicon.starts_with("http://") || favicon.starts_with("https://"))
    {
        remote_image_url(favicon)
            .rounded(24.0)
            .placeholder_color(tinted(ACCENT, 0.12))
            .frame(240.0, 240.0)
            .id("playing-art")
            .any()
    } else {
        art_tile().id("playing-art-tile").any()
    }
}

/// The app's own art, for a station with no logo and for the empty state.
fn art_tile() -> impl Piece {
    vector(res::vectors::radio)
        .tint(ACCENT)
        .frame(120.0, 120.0)
        .padding(60.0)
        .background(tinted(ACCENT, 0.12))
        .corner_radius(24.0)
}

// --- settings ------------------------------------------------------------------------------------

/// Appearance and language, from `day-piece-settings`: persisted, applied live, and labeled
/// from Day's catalog (https://daybrite.dev/docs/localization), and the station catalog:
/// what is installed, whether it is current, and where it comes from.
pub(crate) fn settings_body() -> impl Piece {
    let app = App::app();
    let sync = app.sync;
    // The publisher override, edited here and read at the next check.
    let base = Signal::new(day::prefs::get(crate::sync::BASE_KEY).unwrap_or_default());
    watch(
        move || base.get(),
        move |b, previous| {
            if previous.is_some() {
                sync.set_base(b);
            }
        },
    );
    form((
        day_piece_settings::settings_sections(
            crate::THEME_KEY,
            crate::LOCALE_KEY,
            res::locales::ALL,
        ),
        section((
            labeled(
                res::str::catalog_installed(),
                label(move || installed_line(app)).id("catalog-installed"),
            ),
            labeled(
                res::str::catalog_status(),
                label(move || sync_line(&sync.state.get())).id("catalog-status"),
            ),
            labeled(
                res::str::catalog_source(),
                text_field(base)
                    .placeholder(DEFAULT_BASE)
                    .id("catalog-source"),
            ),
            button(res::str::catalog_check())
                .bordered()
                .action(move || sync.check(app.catalog))
                .id("catalog-check"),
        ))
        .title(res::str::catalog_section()),
    ))
}

/// `48,205 stations, published 2026-06-23`, from the manifest beside the copy.
fn installed_line(app: App) -> String {
    match app.sync.installed.get() {
        Some(m) => {
            let date = m.generated_at.chars().take(10).collect::<String>();
            res::str::catalog_installed_line(grouped(m.count as i64), date).format()
        }
        None if app.catalog.attached.get() => {
            res::str::browse_count(app.catalog.station_count.get() as f64).format()
        }
        None => res::str::catalog_none().format(),
    }
}

/// The same body as a navigable section, for the platforms with no menu bar.
pub(crate) fn settings_page() -> impl Piece {
    column((
        label(res::str::nav_settings())
            .font(Font::Title)
            .id("settings-title"),
        settings_body(),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
}

/// The catalog source's hint is a Fluent string the field does not use: the URL itself is the
/// placeholder, which reads better than prose in a URL field. Named so `day lint` sees it used.
#[allow(dead_code)]
fn _source_hint() -> LocalizedText {
    res::str::catalog_source_hint()
}
