#!/usr/bin/env python3
"""Generate the bundled fixtures the walkthrough runs on, under resource/assets/.

    python3 scripts/build-fixtures.py

Three directories come out, each a stand-in for a network source so a scripted run needs no
network and gets the same answer every time:

- library-fixture/: a dozen short WAV tones with RIFF INFO tags (title, artist, album, genre,
  year, track) and a manifest.json listing them. `TUNES_LIBRARY_IMPORT=asset:library-fixture`
  imports them at launch, the way a listener's Import Files… does.
- radio-fixture/: the Radio Browser JSON shapes (stations, countries, tags, languages) for
  `TUNES_RADIO_INDEX=asset:radio-fixture`. Stream URLs point at public stations; the script
  asserts nothing about live playback.
- podcast-fixture/: the directory's lookup JSON (top.json, search.json) and two RSS feeds for
  `TUNES_PODCAST_DIRECTORY=asset:podcast-fixture`; the feeds' enclosures are public MP3 URLs.

Everything is deterministic: the same script makes byte-identical files.
"""

import json
import math
import os
import struct

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "resource", "assets")


def wav_with_info(path, seconds, freq, info):
    """A mono 8 kHz 16-bit WAV: a `fmt ` chunk, the samples, and a LIST INFO chunk of tags."""
    rate = 8000
    n = int(seconds * rate)
    samples = bytearray()
    for i in range(n):
        env = min(1.0, i / 200.0, (n - i) / 200.0)
        v = int(6000 * env * math.sin(2 * math.pi * freq * i / rate))
        samples += struct.pack("<h", v)
    fmt = struct.pack("<HHIIHH", 1, 1, rate, rate * 2, 2, 16)
    chunks = [(b"fmt ", fmt), (b"data", bytes(samples))]
    info_body = bytearray(b"INFO")
    for key, value in info.items():
        v = value.encode("utf-8") + b"\0"
        if len(v) % 2:
            v += b"\0"
        info_body += key.encode("ascii") + struct.pack("<I", len(v)) + v
    chunks.append((b"LIST", bytes(info_body)))
    body = bytearray(b"WAVE")
    for tag, data in chunks:
        body += tag + struct.pack("<I", len(data)) + data
        if len(data) % 2:
            body += b"\0"
    with open(path, "wb") as f:
        f.write(b"RIFF" + struct.pack("<I", len(body)) + body)


def library():
    d = os.path.join(ROOT, "library-fixture")
    os.makedirs(d, exist_ok=True)
    albums = [
        ("Morning Light", "Aurora Fields", "Ambient", 2021, ["First Light", "Dew", "Slow Hours"]),
        ("Backroads", "The Quiet Engines", "Rock", 2019, ["Ignition", "Mile Marker", "Dust", "Home Again"]),
        ("Night Signals", "Vela Nine", "Electronic", 2023, ["Pulse", "Afterglow", "Orbit"]),
        ("Kitchen Table", "Mara Lindqvist", "Folk", 2017, ["Coffee Song", "Letters"]),
    ]
    files = []
    n = 0
    for album, artist, genre, year, tracks in albums:
        for i, title in enumerate(tracks, start=1):
            n += 1
            name = f"{n:02d}.wav"
            wav_with_info(
                os.path.join(d, name),
                0.6 + 0.1 * i,
                220 + 30 * n,
                {
                    "INAM": title,
                    "IART": artist,
                    "IPRD": album,
                    "IGNR": genre,
                    "ICRD": str(year),
                    "ITRK": str(i),
                },
            )
            files.append(name)
    with open(os.path.join(d, "manifest.json"), "w") as f:
        json.dump({"files": files}, f, indent=2)
        f.write("\n")
    print(f"library-fixture: {n} files")


def station(uuid, name, url, country, code, tags, codec="MP3", bitrate=128, votes=100, lang="english", hp=""):
    return {
        "stationuuid": uuid,
        "name": name,
        "url": url,
        "url_resolved": url,
        "homepage": hp,
        "favicon": "",
        "tags": tags,
        "country": country,
        "countrycode": code,
        "state": "",
        "language": lang,
        "votes": votes,
        "codec": codec,
        "bitrate": bitrate,
        "hls": 0,
        "lastcheckok": 1,
        "clickcount": votes * 3,
    }


def radio():
    d = os.path.join(ROOT, "radio-fixture")
    os.makedirs(d, exist_ok=True)
    stations = [
        station("f1", "FIP", "https://icecast.radiofrance.fr/fip-midfi.mp3", "France", "FR", "jazz,eclectic,public radio", votes=900, lang="french", hp="https://www.radiofrance.fr/fip"),
        station("f2", "Radio Paradise", "https://stream.radioparadise.com/mp3-128", "United States Of America", "US", "eclectic,rock,indie", votes=1200, hp="https://radioparadise.com"),
        station("f3", "SomaFM Groove Salad", "https://ice1.somafm.com/groovesalad-128-mp3", "United States Of America", "US", "ambient,downtempo,chillout", votes=800, hp="https://somafm.com"),
        station("f4", "BBC Radio 3", "http://stream.live.vc.bbcmedia.co.uk/bbc_radio_three", "United Kingdom Of Great Britain And Northern Ireland", "GB", "classical,public radio", codec="AAC", bitrate=96, votes=700),
        station("f5", "Radio Swiss Jazz", "https://stream.srg-ssr.ch/m/rsj/mp3_128", "Switzerland", "CH", "jazz", votes=650, lang="german"),
        station("f6", "Deutschlandfunk Kultur", "https://st02.sslstream.dlf.de/dlf/02/128/mp3/stream.mp3", "Germany", "DE", "culture,talk,public radio", votes=500, lang="german"),
        station("f7", "NTS Radio 1", "https://stream-relay-geo.ntslive.net/stream", "United Kingdom Of Great Britain And Northern Ireland", "GB", "eclectic,electronic", votes=450),
        station("f8", "Radio Nova", "https://novazz.ice.infomaniak.ch/novazz-128.mp3", "France", "FR", "eclectic,indie", votes=400, lang="french"),
    ]
    by_votes = sorted(stations, key=lambda s: -s["votes"])
    dump = lambda name, rows: json.dump(rows, open(os.path.join(d, name), "w"), indent=1)
    dump("top.json", by_votes)
    dump("trending.json", sorted(stations, key=lambda s: -s["clickcount"]))
    dump("search.json", by_votes)
    countries = {}
    tags = {}
    langs = {}
    for s in stations:
        countries.setdefault((s["countrycode"], s["country"]), 0)
        countries[(s["countrycode"], s["country"])] += 1
        for t in s["tags"].split(","):
            tags[t] = tags.get(t, 0) + 1
        langs[s["language"]] = langs.get(s["language"], 0) + 1
    dump("countries.json", [{"name": n, "iso_3166_1": c, "stationcount": k} for (c, n), k in sorted(countries.items(), key=lambda e: -e[1])])
    dump("tags.json", [{"name": t, "stationcount": k} for t, k in sorted(tags.items(), key=lambda e: (-e[1], e[0]))])
    dump("languages.json", [{"name": l, "stationcount": k} for l, k in sorted(langs.items(), key=lambda e: -e[1])])
    for (code, _), _ in countries.items():
        dump(f"country-{code}.json", [s for s in by_votes if s["countrycode"] == code])
    for t in tags:
        slug = "".join(c if c.isalnum() else "-" for c in t.lower())
        dump(f"tag-{slug}.json", [s for s in by_votes if t in s["tags"].split(",")])
    for l in langs:
        dump(f"language-{l}.json", [s for s in by_votes if s["language"] == l])
    print(f"radio-fixture: {len(stations)} stations, {len(countries)} countries, {len(tags)} tags")


def podcasts():
    d = os.path.join(ROOT, "podcast-fixture")
    os.makedirs(d, exist_ok=True)
    shows = [
        {
            "kind": "podcast", "collectionId": 1001, "collectionName": "Morning Signal", "artistName": "Ada Lumen",
            "feedUrl": "asset:podcast-fixture/feed-1.xml", "artworkUrl600": "", "primaryGenreName": "Technology",
            "trackCount": 3, "collectionViewUrl": "https://example.org/signal",
        },
        {
            "kind": "podcast", "collectionId": 1002, "collectionName": "The Long Walk", "artistName": "Field Notes Collective",
            "feedUrl": "asset:podcast-fixture/feed-2.xml", "artworkUrl600": "", "primaryGenreName": "Society & Culture",
            "trackCount": 2, "collectionViewUrl": "https://example.org/walk",
        },
    ]
    json.dump({"resultCount": len(shows), "results": shows}, open(os.path.join(d, "top.json"), "w"), indent=1)
    json.dump({"resultCount": len(shows), "results": shows}, open(os.path.join(d, "search.json"), "w"), indent=1)
    feed1 = """<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd" version="2.0">
<channel>
  <title>Morning Signal</title>
  <link>https://example.org/signal</link>
  <description>Short conversations about the tools that shape a day's work.</description>
  <itunes:author>Ada Lumen</itunes:author>
  <item>
    <title>Episode 3: Steam and Syntax</title>
    <guid isPermaLink="false">signal-3</guid>
    <pubDate>Tue, 10 Jun 2025 08:00:00 GMT</pubDate>
    <enclosure url="https://www.soundhelix.com/examples/mp3/SoundHelix-Song-1.mp3" length="8000000" type="audio/mpeg"/>
    <itunes:duration>6:12</itunes:duration>
    <description>Why a good morning starts with a quiet terminal.</description>
  </item>
  <item>
    <title>Episode 2: The Second Cup</title>
    <guid isPermaLink="false">signal-2</guid>
    <pubDate>Tue, 03 Jun 2025 08:00:00 GMT</pubDate>
    <enclosure url="https://www.soundhelix.com/examples/mp3/SoundHelix-Song-2.mp3" length="7000000" type="audio/mpeg"/>
    <itunes:duration>5:40</itunes:duration>
    <description>On habits, and how to keep the ones that matter.</description>
  </item>
  <item>
    <title>Episode 1: First Light</title>
    <guid isPermaLink="false">signal-1</guid>
    <pubDate>Tue, 27 May 2025 08:00:00 GMT</pubDate>
    <enclosure url="https://www.soundhelix.com/examples/mp3/SoundHelix-Song-3.mp3" length="7500000" type="audio/mpeg"/>
    <itunes:duration>5:55</itunes:duration>
    <description>The pilot.</description>
  </item>
</channel>
</rss>
"""
    feed2 = """<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd" version="2.0">
<channel>
  <title>The Long Walk</title>
  <link>https://example.org/walk</link>
  <description>Stories from people who went somewhere on foot.</description>
  <itunes:author>Field Notes Collective</itunes:author>
  <item>
    <title>Across the Fells</title>
    <guid>walk-2</guid>
    <pubDate>Sun, 08 Jun 2025 06:00:00 GMT</pubDate>
    <enclosure url="https://www.soundhelix.com/examples/mp3/SoundHelix-Song-4.mp3" length="9000000" type="audio/mpeg"/>
    <itunes:duration>7:30</itunes:duration>
    <description>Three days in the Lake District.</description>
  </item>
  <item>
    <title>The Coast Path</title>
    <guid>walk-1</guid>
    <pubDate>Sun, 25 May 2025 06:00:00 GMT</pubDate>
    <enclosure url="https://www.soundhelix.com/examples/mp3/SoundHelix-Song-5.mp3" length="8800000" type="audio/mpeg"/>
    <itunes:duration>7:05</itunes:duration>
    <description>Salt, wind, and a tide table.</description>
  </item>
</channel>
</rss>
"""
    open(os.path.join(d, "feed-1.xml"), "w").write(feed1)
    open(os.path.join(d, "feed-2.xml"), "w").write(feed2)
    print("podcast-fixture: 2 shows")


if __name__ == "__main__":
    library()
    radio()
    podcasts()
