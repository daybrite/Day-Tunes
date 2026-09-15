# Day Tunes

An internet radio player built with [Day](https://daybrite.dev) in one Rust codebase and
rendered with native widgets on iPhone, Android, Mac, Windows, Linux, HarmonyOS,
and the web.

## Run it in one command

Install the `day` CLI, then let it clone, build, and launch the app for your desktop:

```sh
cargo install day-cli
day launch --git https://github.com/daybrite/Day-Tunes.git
```

`day doctor` lists what your platform's toolkit needs and prints the install command for anything
missing. The launch prints where it put the checkout, so you can open the code and change it.

## What you get

- The [Tune Out catalog](https://github.com/Tune-Out/stations): 48,205 stations from
  radio-browser.info with duplicates folded, genres reduced to a closed vocabulary, and an
  editorial score per station. The app downloads the published SQLite file on first launch,
  checks the publisher's manifest on every launch after that, and swaps in a newer file in
  the background. Once the copy is on the device, browsing and search work offline.
- Browse by top stations, country, genre, or language. Search matches names, genres, and
  countries as you type, through SQLite's full-text index.
- A station page with one Play button per stream the station publishes, its genres, stream
  format, and homepage.
- Favorites, recents, a note per station, and your own tags, kept in a separate database on
  the device and joined to the catalog by station id, so they survive every catalog update.
  The Library filters by tag.
- Settings shows which catalog is installed, lets you check for an update, and takes another
  publisher's URL for a mirror or a local build of the catalog.
- A Now Playing screen in the middle of the tab bar, laid out like a music app's: the
  station's logo, the track the stream names (title, artist, and album, kept for a later
  lookup), Previous and Next through the list the station came from, the favorite star, and
  the volume. On the desktop the same transport rides the window toolbar and the Playback menu.
- Browse drills down as real navigation: Genres, then Pop, then the station, each a page with
  the platform's own back. The cut (Top, Countries, Genres, Languages) sits in the window
  toolbar on the desktop and in the navigation bar on iOS.
- Playback through [day-piece-media](https://github.com/daybrite/day-piece-media): AVPlayer,
  Android MediaPlayer, GTK's media backend, Qt Multimedia, Windows Media, ArkUI AVPlayer, and the browser's
  `<audio>`. The desktop transport rides the window toolbar and the Playback menu; phones get a
  Now Playing tab.

What the stream says it is playing comes from AVFoundation's timed metadata on Apple platforms
and from a short ICY probe of the stream elsewhere; the web cannot read it.

The browser keeps the catalog and listener database in OPFS through Day's SQLite worker. It
fetches the catalog asynchronously, verifies the SHA-256, and imports each revision under a
separate name before replacing the attachment. Favorites, notes, and tags survive page reloads.
The server must provide Day's cross-origin isolation headers; remote catalog servers must
allow CORS. Station playback also depends on the browser's codec and cross-origin policies.
Some stations cannot play in an isolated page.

Homebrew GTK has no media backend, and the local OpenHarmony emulator lacks media plugins.
Both can browse the catalog, but playback may be unavailable. On web and HarmonyOS, the app
uses its radio artwork because the remote-image piece has no renderer for those toolkits.

## Work with local Day checkouts

Keep `day/`, `day-piece-media/`, and `Day-Tunes/` beside one another. Use the rebuilt Day CLI
and local Cargo overrides for both dependencies:

```sh
day patch --local ../day --local ../day-piece-media --check
day launch -p macos-appkit --env TUNES_CATALOG_URL=asset:catalog-fixture \
  --script dayscript/walkthrough.yaml
```

The app and media crate use the same bare Day Git URL; adding `branch = "main"` to only one
can make Cargo resolve separate copies of the framework. The local configuration is ignored
by Git. An unpublished media checkout must remain patched locally.

`cargo test` checks the catalog and manifest helpers. After a web build,
`node scripts/test-web-storage.mjs` checks that the catalog, a favorite, and a note survive a
reload with catalog requests blocked. It uses Chromium through Playwright; set
`DAY_WEB_DRIVER_PLAYWRIGHT` to its installation directory if it is not installed locally.

The fixture supplies catalog data. The walkthrough exercises stream controls but does not
assert that a public radio server is reachable. Playback errors remain visible in the app.

## Layout

- `src/lib.rs` — the window: navigation, toolbar, menus, and the player piece.
- `src/catalog.rs` — the station model over the downloaded file, the listener's own models
  linked to it by station id, and the queries.
- `src/sync.rs` — the manifest check, the download with its hash check, and the swap.
- `src/pages.rs` — the Browse, Search, and Library panes, the station page, Now Playing, and
  Settings.
- `resource/assets/catalog-fixture/` — a small catalog with the published file's schema, for
  the walkthrough; `scripts/build-fixture.py` rebuilds it from a full `stations.sqlite`.
- `dayscript/walkthrough.yaml` — the walkthrough that doubles as the UI test:
  `day launch -p macos-appkit --env TUNES_CATALOG_URL=asset:catalog-fixture --script dayscript/walkthrough.yaml`.

## Targets

Every platform Day supports is in `Day.toml`. Build or launch one with `day build -p <target>`
or `day launch -p <target>`.

## Local validation

The fixture walkthrough passes 145/145 steps on macos-appkit, macos-qt, macos-gtk, ios-uikit,
android-mdc, web-dom (Chromium), and harmony-arkui. Windows and Linux require separate hosts.
These are UI and catalog checks; they do not guarantee that a public station can play.

Run desktop variants sequentially, or give each a separate `DAY_DATA_DIR` through `--env`,
so their walkthroughs do not change the same favorites and tags concurrently. Select a single
mobile device with `--ios-simulator` or `--android-device`; set `ANDROID_SERIAL` as well when
multiple Android emulators are running. The local OpenHarmony SDK may need
`OHOS_BASE_SDK_HOME` and `NODE_PATH` configured for Hvigor, plus `DAY_OHOS_TARGET` to select the
emulator.

Formatting, strict Day lint, Clippy for the available backends, the six Rust tests, and the
browser persistence regression test pass. Web Clippy needs a WebAssembly-capable Clang; on
macOS use Homebrew LLVM via `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown`.

## License

Apache-2.0. The station data comes from the [Tune Out catalog](https://github.com/Tune-Out/stations)
and is public domain (CC0); the fixture under `resource/assets/catalog-fixture/` is a cut of it.
