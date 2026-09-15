# Day Tunes

An internet radio player for browsing stations around the world and keeping the ones you like.
Built with [Day](https://daybrite.dev), a Rust framework that renders native widgets on Mac,
iPhone, Android, Windows, Linux, and HarmonyOS, and DOM elements on the web.

## Run it in one command

Install the `day` CLI, then let it clone, build, and launch the app for your desktop:

```sh
cargo install day-cli
day launch --git https://github.com/daybrite/Day-Tunes.git
```

`day doctor` lists the toolchain requirements and install commands for anything missing.
The launch prints the checkout location so you can open the code and change it.

## What you get

- Browse the [Tune Out catalog](https://github.com/Tune-Out/stations) by country, genre,
  language, or top stations. Search names, genres, and countries as you type.
- Play stations and choose among the available streams; inspect their formats or visit their
  homepages.
- Keep favorites, listening history, notes, and tags. Your library stays on the device and
  survives catalog updates.
- Browse while listening. Desktop toolbar controls and a separate Now Playing destination
  provide playback, volume, favorites, and Previous/Next through the station list.
- Select a station to open its details. Track details appear when the stream and platform
  provide them.
- Change the appearance and language, check for catalog updates, or use another catalog URL
  in Settings.

The catalog downloads on first launch and updates in the background. Once downloaded, it is
available for offline browsing and search; listening still needs a network connection.
Navigation adapts to the window size, with tabs, a rail, or a sidebar. Audio is hosted in the
first window's piece tree; it survives page changes, but is not an application-owned background
service. Activity recreation can restart the player. Lock-screen controls and guaranteed
background playback are not implemented.

Playback uses [day-piece-media](https://github.com/daybrite/day-piece-media) and the platform's
media engine. Available codecs and plugins determine which streams work. GTK installations
without a media backend and OpenHarmony emulators without media plugins can browse stations
but cannot play them.

On the web, browser codec and cross-origin policies can prevent some stations from playing,
and stream metadata is unavailable. Web and HarmonyOS use bundled radio artwork in place of
station logos.

## Build from a clone

Day compiles one toolkit backend per binary. Choose a target from [Day.toml](Day.toml) when
you build or launch:

```sh
day doctor
day launch -p macos-appkit
day launch -p ios-uikit          # needs a booted Simulator
day launch -p android-mdc        # needs a JDK and a running emulator or device
day launch -p web-dom            # serves the WebAssembly build locally
cargo test                      # catalog and manifest unit tests
```

Bare Cargo commands use the default `mock` backend for tests and editor support. To build a
native desktop backend directly, disable the default and select a feature, for example
`cargo build --no-default-features --features appkit`.

To work against sibling checkouts of Day and the media crate, generate local Cargo overrides:

```sh
day patch --local ../day --local ../day-piece-media --check
```

The [walkthrough](dayscript/walkthrough.yaml) exercises browsing, search, the library, settings,
and playback controls, and captures screenshots. Run it with the bundled catalog fixture:

```sh
day launch -p macos-appkit --env TUNES_CATALOG_URL=asset:catalog-fixture \
  --script dayscript/walkthrough.yaml
```

The fixture makes catalog tests independent of the download server. Its stream URLs still
refer to public stations; the walkthrough does not assert successful live playback. Run desktop
walkthroughs sequentially, or pass a separate `--env DAY_DATA_DIR=/path/to/test-data` to each
instance so they do not modify the same library.

`day lint --strict` checks the app's declarations and locale coverage. The browser storage test
checks that the catalog, a favorite, and a note survive a reload with catalog requests blocked:

```sh
day build -p web-dom
node scripts/test-web-storage.mjs
```

The storage test needs Playwright and Chromium. If Playwright is installed outside the project,
set `DAY_WEB_DRIVER_PLAYWRIGHT` to the directory containing its `node_modules`.

## Inside the code

- [src/lib.rs](src/lib.rs) sets up navigation, menus, overlays, and the media player.
  `App` holds the shared catalog, sync, and playback state; `Scene` holds each window's
  navigation and selection. An application-owned audio service keeps playback independent of
  individual windows.
- [src/catalog.rs](src/catalog.rs) defines the station queries and listener data. A read-only
  catalog is attached to the listener's SQLite database, with favorites, notes, and tags linked
  by station UUID. Search uses SQLite's full-text index.
- [src/sync.rs](src/sync.rs) checks the publisher's manifest, downloads the catalog, verifies its
  SHA-256 hash, and replaces the installed revision.
- [src/pages.rs](src/pages.rs) contains Browse, Search, Library, station details, Now Playing,
  and Settings.
- [resource/locales/](resource/locales/) contains the Fluent translations.
- [resource/assets/catalog-fixture/](resource/assets/catalog-fixture/) holds the test catalog.
  [scripts/build-fixture.py](scripts/build-fixture.py) rebuilds it from a full `stations.sqlite`.
- [platform/](platform/) contains the native host projects for mobile targets.

The app uses Day's persistence support, `day-part-http` for catalog downloads,
`day-piece-settings` for preferences, and `day-piece-remote-image` for station artwork.
`day-piece-media` supplies playback; Serde parses the catalog manifest, and `day-build`
generates resource bindings. [Cargo.toml](Cargo.toml) lists the dependencies and backend features.

In the browser, Day's SQLite worker stores both databases in the origin private file system
(OPFS). Catalog revisions use separate filenames so a replacement can be attached before the
old file is removed. Hosting requires cross-origin isolation headers, which Day's development
server supplies; remote catalog servers must also allow CORS.

Day Tunes is open source under the Apache-2.0 license. The
[Tune Out catalog](https://github.com/Tune-Out/stations) and the bundled catalog fixture are
public domain (CC0).
