# Day Tunes

A music player for the songs you own, the world's radio, and the podcasts you follow. Built with
[Day](https://daybrite.dev), a Rust framework that renders native widgets on Mac, iPhone,
Android, Windows, Linux, and HarmonyOS, and DOM elements on the web.

## Run it in one command

Install the `day` CLI, then let it clone, build, and launch the app for your desktop:

```sh
cargo install day-cli
day launch --git https://github.com/daybrite/Day-Tunes.git
```

`day doctor` lists the toolchain requirements and install commands for anything missing.
The launch prints the checkout location so you can open the code and change it.

## What you get

- **A library.** Add a folder of music (or import files on a phone) and the app reads each
  file's tags and cover art: MP3, AAC, FLAC, Ogg Vorbis, Opus, WAV and AIFF. Songs, albums,
  artists and genres, recently added and recently played, and playlists you make and reorder.
- **Up Next.** Play an album, a playlist, a search, or a station list, and step through it.
  Shuffle, repeat, play next, add to the queue, and drag the queue into the order you want.
- **Internet radio.** Browse the community-run [Radio Browser](https://www.radio-browser.info)
  index by top stations, trending, country, tag or language, and search it by name. Keep the
  stations you like; the ones you play are remembered. The stream's own song title shows while
  it plays. The index is one implementation of a small trait, so another directory can take
  its place, and Settings lets you name a mirror of your own.
- **Podcasts.** Discover shows from the Apple Podcasts chart and search, or add any feed by
  URL. Episodes refresh on their own, resume where you stopped, and download for the road. The
  directory is one implementation of a trait too; the feed parser is the app's own.
- **Scrobbling.** Sign in to Last.fm, Libre.fm, ListenBrainz, or any GNU FM server, and every
  listen goes out, including the songs a radio station names, queued while offline.
- **The system's controls.** The lock screen, the media keys, headset buttons and MPRIS
  clients show what is on and drive the transport.
- **Every platform.** A three-pane window with a player bar on the desktops and the web; tabs,
  stacks and a mini player on the phones. Appearance and language follow Settings.

Playback uses [day-piece-media](https://github.com/daybrite/day-piece-media) and the platform's
media engine, so codecs and streaming formats are the platform's. The web build browses radio
and podcasts and plays what the browser's codecs and cross-origin rules allow; it has no local
library.

## Build from a clone

Day compiles one toolkit backend per binary. Choose a target from [Day.toml](Day.toml) when
you build or launch:

```sh
day doctor
day launch -t macos-appkit
day launch -t ios-uikit          # needs a booted Simulator
day launch -t android-mdc        # needs a JDK and a running emulator or device
day launch -t web-dom            # serves the WebAssembly build locally
cargo test                      # the parsers, the store, the queue
```

Bare Cargo commands use the default `mock` backend for tests and editor support.

To work against sibling checkouts of Day and the media crate, generate local Cargo overrides:

```sh
day patch --local ../day --local ../day-piece-media --local ../day-persistence --check
```

The [walkthrough](dayscript/walkthrough.yaml) exercises the library, playlists, radio, podcasts,
the queue and settings, and captures screenshots. Run it on the bundled fixtures:

```sh
day launch -t macos-appkit \
  --env TUNES_LIBRARY_IMPORT=asset:library-fixture \
  --env TUNES_RADIO_INDEX=asset:radio-fixture \
  --env TUNES_PODCAST_DIRECTORY=asset:podcast-fixture \
  --script dayscript/walkthrough.yaml
```

The fixtures live under [resource/assets/](resource/assets/) and are made by
[scripts/build-fixtures.py](scripts/build-fixtures.py): a dozen tagged WAV tones, the index's
JSON for a handful of stations, and two podcast feeds. The script asserts nothing about live
playback. Run desktop walkthroughs sequentially, or pass a separate `--env DAY_DATA_DIR=…` to
each instance so they do not share one library.

`day lint --strict` checks the app's declarations and locale coverage.

## Inside the code

- [src/lib.rs](src/lib.rs): the app's sections and routes, `App` (the store, the sources, the
  transport) and `Scene` (what one window shows), startup, the menu bar.
- [src/library.rs](src/library.rs): the SQLite store (tracks, folders, playlists, stations,
  podcasts, episodes, history, pending scrobbles), the grouped album, artist and genre views,
  the folder scanner and the file importer.
- [src/transport.rs](src/transport.rs): the app-owned audio session, the queue, shuffle and
  repeat, resume positions, listens, and the system's now-playing display.
- [src/radio.rs](src/radio.rs): the `RadioIndex` trait, the Radio Browser client, the fixture
  index, and the per-browse cache.
- [src/podcasts.rs](src/podcasts.rs): the `PodcastDirectory` trait, the Apple directory, the
  RSS parser, feed refreshes and episode downloads.
- [src/scrobble.rs](src/scrobble.rs): the `Scrobbler` trait, the Audioscrobbler 2.0 and
  ListenBrainz clients, sign-in and the send queue.
- [src/pages.rs](src/pages.rs): every surface, shared by the desktop and phone shells.
- [resource/locales/](resource/locales/): the Fluent translations.

The app uses Day's persistence extension for its store, `day-part-http` for every source,
`day-part-fs` for imported media, artwork and downloads, `day-part-keychain` for scrobbler
sessions, `day-piece-settings` for preferences, and `day-piece-media` for playback and tag
reading. [Cargo.toml](Cargo.toml) lists the dependencies and backend features.

Day Tunes is open source under the Apache-2.0 license.

## Generated native projects

Use a Day CLI with automatic native project generation; Apple builds require **Xcode 27
or newer**. Xcode project documents, Android Gradle files, and HarmonyOS hvigor files are
created under `build/` by `day prepare`, `day build`, and `day launch`. Open the selected
project with `day open -t <target>`; edits to generated files are disposable. Apple Swift
sources, plists and `.xcconfig` files, and any Android resource overlays, remain app-owned
under `platform/`. For additional native configuration, use
`day project add-target <target> --persist-native-project` to create a persistent override.
