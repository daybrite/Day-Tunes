#!/usr/bin/env python3
"""Build the walkthrough's catalog fixture from a real Tune Out `stations.sqlite`.

The fixture is a small catalog with the SAME schema as the published one — every table,
index, and the FTS5 index rebuilt over the rows kept — so the app reads it through exactly
the code path the download takes. It is served to the walkthrough as `asset:catalog-fixture`
(see src/sync.rs) with a manifest that hashes it, which is what a real publisher's does.

    python3 scripts/build-fixture.py /opt/src/github/Tune-Out/stations/public/data/stations.sqlite

Keeps: the 200 most-voted online stations, plus every station whose name contains a
name from KEEP (so the walkthrough's searches find something), and every tag/language/stream
row that refers to a kept station.
"""
import hashlib
import json
import os
import shutil
import sqlite3
import sys
import time

KEEP = ["France Culture", "BBC Radio 6", "KEXP", "FIP", "Jazz24", "WQXR"]
TOP = 200

def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    here = os.path.dirname(os.path.abspath(__file__))
    out_dir = os.path.join(here, "..", "resource", "assets", "catalog-fixture")
    os.makedirs(out_dir, exist_ok=True)
    tmp = os.path.join(out_dir, "stations.sqlite.tmp")
    if os.path.exists(tmp):
        os.remove(tmp)
    shutil.copyfile(src, tmp)
    db = sqlite3.connect(tmp)
    db.execute("PRAGMA foreign_keys = OFF")
    keep_names = " OR ".join("name LIKE ?" for _ in KEEP)
    db.execute(
        f"""CREATE TEMP TABLE keep AS
            SELECT uuid FROM stations WHERE lastcheckok = 1 AND url <> ''
              AND ({keep_names})
            UNION
            SELECT uuid FROM (SELECT uuid FROM stations WHERE lastcheckok = 1 AND url <> ''
                               ORDER BY votes DESC LIMIT {TOP})""",
        [f"%{n}%" for n in KEEP],
    )
    (n,) = db.execute("SELECT COUNT(*) FROM keep").fetchone()
    db.execute("DELETE FROM stations WHERE uuid NOT IN (SELECT uuid FROM keep)")
    for table, col in [("station_tags", "station_uuid"), ("station_languages", "station_uuid"),
                       ("station_streams", "station_uuid")]:
        db.execute(f"DELETE FROM {table} WHERE {col} NOT IN (SELECT uuid FROM keep)")
    db.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM station_tags)")
    db.execute("DELETE FROM languages WHERE id NOT IN (SELECT language_id FROM station_languages)")
    # The FTS index is contentless: rebuild it from the kept rows, the way the publisher does.
    db.execute("INSERT INTO stations_fts(stations_fts) VALUES('delete-all')")
    db.execute(
        """INSERT INTO stations_fts(rowid, name, tags_text, languages_text, research_text)
           SELECT s.rowid, s.name,
                  (SELECT group_concat(t.slug, ' ') FROM station_tags st JOIN tags t ON t.id = st.tag_id
                    WHERE st.station_uuid = s.uuid),
                  (SELECT group_concat(l.slug, ' ') FROM station_languages sl JOIN languages l ON l.id = sl.language_id
                    WHERE sl.station_uuid = s.uuid),
                  trim(coalesce(s.r_nature, '') || ' ' || coalesce(s.r_operator, '') || ' '
                       || coalesce(s.r_format, '') || ' ' || coalesce(s.r_notes, ''))
           FROM stations s"""
    )
    db.execute("DROP TABLE keep")
    db.commit()
    db.execute("VACUUM")
    db.close()
    dest = os.path.join(out_dir, "stations.sqlite")
    os.replace(tmp, dest)
    data = open(dest, "rb").read()
    manifest = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(os.path.getmtime(src))),
        "count": n,
        "source": os.path.basename(src),
        "artifacts": {"sqlite": {"path": "data/stations.sqlite", "size": len(data),
                                 "sha256": hashlib.sha256(data).hexdigest()}},
    }
    with open(os.path.join(out_dir, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"{n} stations, {len(data)} bytes -> {dest}")

if __name__ == "__main__":
    main()
