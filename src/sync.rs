//! Keeping the local catalog copy current (https://daybrite.dev/docs/http).
//!
//! The Tune Out catalog is published as a manifest beside the database it describes:
//!
//! ```text
//! <base>/manifest.json     { "generated_at", "count", "artifacts": { "sqlite": { "path", "size", "sha256" } } }
//! <base>/stations.sqlite   the file the manifest hashes
//! ```
//!
//! On every launch the app reads the manifest, and when its sha256 differs from the copy on
//! disk it streams the file to a temp path — hashing as it lands — then renames it into place
//! and re-attaches it, so the old copy keeps answering until the new one is whole. The base is
//! a constant with a Settings override; a `file:` or `asset:` base reads the same two files
//! from disk or from the app bundle, which is what the walkthrough uses.

use crate::catalog::Catalog;
use day::prelude::*;
use day_part_http::Request;
#[cfg(not(target_arch = "wasm32"))]
use day_part_http::{Download, HttpError, StreamSink, fetch_streamed};
use serde::Deserialize;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};

/// Where the catalog is published.
pub const DEFAULT_BASE: &str = "https://tune-out.app/data/";
/// The Settings override's `day::prefs` key (empty = the default).
pub const BASE_KEY: &str = "catalog.base";
/// The launch-time override, for scripts: `TUNES_CATALOG_URL=asset:catalog-fixture`.
pub const BASE_ENV: &str = "TUNES_CATALOG_URL";

/// What the publisher says about the current catalog.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub artifacts: Artifacts,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Artifacts {
    #[serde(default)]
    pub sqlite: Artifact,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub sha256: String,
}

impl Manifest {
    fn parse(text: &str) -> Result<Manifest, String> {
        serde_json::from_str(text).map_err(|e| format!("the manifest is not readable: {e}"))
    }
}

/// Where the sync is, for the UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SyncState {
    #[default]
    Idle,
    Checking,
    Downloading {
        received: u64,
        total: u64,
    },
    Verifying,
    /// The installed copy matches the published one.
    UpToDate,
    Failed(String),
}

/// The app-wide sync: its state and the copy on disk. `Copy`, because every field is a handle.
#[derive(Clone, Copy)]
pub struct Sync {
    pub state: Signal<SyncState>,
    /// The manifest of the copy on disk, once one is attached.
    pub installed: Signal<Option<Manifest>>,
    /// The base URL in use (the override, else the default).
    pub base: Signal<String>,
    /// Whether a check or download is in flight, so a second "Check now" waits.
    busy: Signal<bool>,
}

/// Where the copy lives: a directory of its own under the app's data root (which the
/// platform convention shares between Day apps, hence the app's name in it).
#[cfg(not(target_arch = "wasm32"))]
struct Paths {
    dir: PathBuf,
    db: PathBuf,
    manifest: PathBuf,
    part: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
fn paths() -> Result<Paths, String> {
    let root = day::persistence::Sqlite::app_data_dir().map_err(|e| e.to_string())?;
    let dir = root.join("tunes-catalog");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(Paths {
        db: dir.join("stations.sqlite"),
        manifest: dir.join("manifest.json"),
        part: dir.join("stations.sqlite.part"),
        dir,
    })
}

impl Sync {
    pub fn new() -> Sync {
        let base = day::env(BASE_ENV)
            .filter(|s| !s.is_empty())
            .or_else(|| day::prefs::get(BASE_KEY).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| DEFAULT_BASE.to_string());
        Sync {
            state: Signal::new(SyncState::Idle),
            installed: Signal::new(None),
            base: Signal::new(base),
            busy: Signal::new(false),
        }
    }

    /// Point at another publisher, from Settings. Empty restores the default. Takes effect on
    /// the next check.
    pub fn set_base(self, base: &str) {
        let base = base.trim();
        day::prefs::set(BASE_KEY, base);
        self.base.set(if base.is_empty() {
            DEFAULT_BASE.to_string()
        } else {
            base.to_string()
        });
    }

    /// Attach the copy on disk, if there is one — the launch path that needs no network.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn attach_installed(self, catalog: &'static Catalog) {
        let Ok(p) = paths() else {
            return;
        };
        if !p.db.is_file() {
            return;
        }
        let manifest = std::fs::read_to_string(&p.manifest)
            .ok()
            .and_then(|t| Manifest::parse(&t).ok());
        match catalog.attach(&p.db) {
            Ok(()) => self.installed.set(manifest),
            Err(e) => {
                // A copy that will not open is one to replace: the next check downloads it
                // again rather than trusting the manifest beside it.
                error!("the catalog copy could not be attached: {e}");
                let _ = std::fs::remove_file(&p.manifest);
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn attach_installed(self, catalog: &'static Catalog) {
        let Some(manifest) =
            day::prefs::get("catalog.installed").and_then(|text| Manifest::parse(&text).ok())
        else {
            return;
        };
        let name = web_catalog_name(&manifest);
        if day::persistence::Sqlite::web_storage().is_ok_and(|storage| storage.exists(&name)) {
            match catalog.attach(std::path::Path::new(&name)) {
                Ok(()) => self.installed.set(Some(manifest)),
                Err(error) => error!("could not attach the browser catalog: {error}"),
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    async fn run(self, catalog: &'static Catalog, source: Source) -> Result<(), String> {
        let manifest = Manifest::parse(&source.text("manifest.json").await?)?;
        let wanted = manifest.artifacts.sqlite.sha256.to_ascii_lowercase();
        if wanted.len() != 64 || !wanted.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("the manifest has no valid SHA-256 for the catalog".into());
        }
        let name = web_catalog_name(&manifest);
        let storage = day::persistence::Sqlite::web_storage().map_err(|e| e.to_string())?;
        if self
            .installed
            .get_untracked()
            .as_ref()
            .is_some_and(|m| web_catalog_name(m) == name)
            && storage.exists(&name)
        {
            return Ok(());
        }
        let artifact = manifest
            .artifacts
            .sqlite
            .path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("stations.sqlite");
        self.state.set(SyncState::Downloading {
            received: 0,
            total: manifest.artifacts.sqlite.size,
        });
        let bytes = source.bytes(artifact).await?;
        self.state.set(SyncState::Verifying);
        if sha256_hex(&bytes) != wanted {
            return Err("the download does not match the manifest".into());
        }
        // Each revision gets a separate OPFS file, so an existing attachment stays valid until
        // the replacement has been verified and imported.
        if !storage.exists(&name) {
            storage
                .import_db(&name, &bytes)
                .map_err(|e| e.to_string())?;
        }
        catalog
            .attach(std::path::Path::new(&name))
            .map_err(|e| e.to_string())?;
        let old = self
            .installed
            .get_untracked()
            .as_ref()
            .map(web_catalog_name);
        day::prefs::set("catalog.installed", &manifest_json(&manifest).to_string());
        self.installed.set(Some(manifest));
        if let Some(old) = old
            && old != name
        {
            storage.delete_db(&old);
        }
        Ok(())
    }

    /// Read the manifest, and download the catalog when the copy on disk is not the
    /// published one. The download streams on a worker thread; the UI follows `state`.
    pub fn check(self, catalog: &'static Catalog) {
        if self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.state.set(SyncState::Checking);
        let base = self.base.get_untracked();
        let source = Source::parse(&base);
        day::task(async move {
            let outcome = self.run(catalog, source).await;
            match outcome {
                Ok(()) => self.state.set(SyncState::UpToDate),
                Err(e) => {
                    error!("catalog sync failed: {e}");
                    self.state.set(SyncState::Failed(e));
                }
            }
            self.busy.set(false);
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    async fn run(self, catalog: &'static Catalog, source: Source) -> Result<(), String> {
        let p = paths()?;
        let manifest = Manifest::parse(&source.text("manifest.json").await?)?;
        let wanted = manifest.artifacts.sqlite.sha256.to_ascii_lowercase();
        if wanted.is_empty() {
            return Err("the manifest names no sqlite artifact".into());
        }
        let have = self
            .installed
            .get_untracked()
            .map(|m| m.artifacts.sqlite.sha256.to_ascii_lowercase());
        if have.as_deref() == Some(wanted.as_str()) && p.db.is_file() {
            return Ok(());
        }
        // The artifact path is relative to the manifest's own directory.
        let name = manifest
            .artifacts
            .sqlite
            .path
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("stations.sqlite")
            .to_string();
        let total = manifest.artifacts.sqlite.size;
        self.state
            .set(SyncState::Downloading { received: 0, total });
        let progress = source.download(&name, &p.part, total, self).await?;
        self.state.set(SyncState::Verifying);
        if progress.sha256 != wanted {
            let _ = std::fs::remove_file(&p.part);
            return Err(format!(
                "the download does not match the manifest (sha256 {}, wanted {wanted})",
                progress.sha256
            ));
        }
        // Into place: on Windows a rename over an existing file fails, so the old copy goes
        // first — after the download is whole and verified, never before.
        let _ = std::fs::remove_file(&p.db);
        std::fs::rename(&p.part, &p.db).map_err(|e| format!("{}: {e}", p.dir.display()))?;
        catalog.attach(&p.db).map_err(|e| e.to_string())?;
        std::fs::write(
            &p.manifest,
            serde_json::to_string_pretty(&manifest_json(&manifest)).unwrap_or_default(),
        )
        .map_err(|e| format!("{}: {e}", p.manifest.display()))?;
        self.installed.set(Some(manifest));
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
fn web_catalog_name(manifest: &Manifest) -> String {
    format!(
        "tunes-catalog-{}.sqlite",
        manifest.artifacts.sqlite.sha256.to_ascii_lowercase()
    )
}

impl Default for Sync {
    fn default() -> Self {
        Sync::new()
    }
}

/// The manifest as we store it beside the copy: only what a later check compares.
fn manifest_json(m: &Manifest) -> serde_json::Value {
    serde_json::json!({
        "generated_at": m.generated_at,
        "count": m.count,
        "artifacts": { "sqlite": {
            "path": m.artifacts.sqlite.path,
            "size": m.artifacts.sqlite.size,
            "sha256": m.artifacts.sqlite.sha256,
        } },
    })
}

/// Where the two files come from.
enum Source {
    Http(String),
    /// A directory on disk (`file:///…/`), for a developer's own build of the catalog.
    File(std::path::PathBuf),
    /// A directory under `resource/assets/` (`asset:name`), for the walkthrough.
    Asset(String),
}

impl Source {
    fn parse(base: &str) -> Source {
        let base = base.trim();
        if let Some(rest) = base.strip_prefix("asset:") {
            return Source::Asset(rest.trim_matches('/').to_string());
        }
        if let Some(rest) = base.strip_prefix("file://") {
            return Source::File(std::path::PathBuf::from(rest));
        }
        let mut url = base.to_string();
        if !url.ends_with('/') {
            url.push('/');
        }
        Source::Http(url)
    }

    /// A small text file (the manifest).
    async fn text(&self, name: &str) -> Result<String, String> {
        match self {
            Source::Http(base) => {
                let req = Request::get(format!("{base}{name}"))
                    .header("Accept", "application/json")
                    .timeout(std::time::Duration::from_secs(30));
                let response = day_part_http::fetch_future(req)
                    .await
                    .map_err(|e| format!("{name}: {e:?}"))?;
                if response.status != 200 {
                    return Err(format!("{name}: HTTP {}", response.status));
                }
                Ok(response.text().into_owned())
            }
            Source::File(dir) => {
                let path = dir.join(name);
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
            }
            Source::Asset(dir) => {
                #[cfg(target_arch = "wasm32")]
                {
                    let _ = dir;
                    let bytes = self.bytes(name).await?;
                    String::from_utf8(bytes).map_err(|e| e.to_string())
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let name = format!("{dir}/{name}");
                    let res = day::resource(day::AssetName::dynamic(name.clone()))
                        .ok_or_else(|| format!("no bundled asset {name}"))?;
                    Ok(String::from_utf8_lossy(res.as_slice()).into_owned())
                }
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    async fn bytes(&self, name: &str) -> Result<Vec<u8>, String> {
        match self {
            Source::Http(base) => {
                let response = day_part_http::fetch_future(Request::get(format!("{base}{name}")))
                    .await
                    .map_err(|e| format!("{name}: {e:?}"))?;
                if response.status != 200 {
                    return Err(format!("{name}: HTTP {}", response.status));
                }
                Ok(response.body)
            }
            Source::Asset(dir) => {
                let url = format!("assets/data/{dir}/{name}");
                let response = day_part_http::fetch_future(Request::get(url))
                    .await
                    .map_err(|e| format!("{name}: {e:?}"))?;
                if response.status != 200 {
                    return Err(format!("{name}: HTTP {}", response.status));
                }
                Ok(response.body)
            }
            Source::File(_) => Err(
                "the browser cannot read a local filesystem path; use an HTTPS or asset source"
                    .into(),
            ),
        }
    }

    /// The database, to `dest`, hashed as it lands. `total` sizes the progress; a server that
    /// says otherwise wins.
    #[cfg(not(target_arch = "wasm32"))]
    async fn download(
        &self,
        name: &str,
        dest: &Path,
        total: u64,
        sync: Sync,
    ) -> Result<Progress, String> {
        match self {
            Source::Http(base) => {
                let url = format!("{base}{name}");
                let shared = Arc::new(Mutex::new(Progress {
                    received: 0,
                    total,
                    done: None,
                    sha256: String::new(),
                }));
                let dest = dest.to_path_buf();
                let worker = shared.clone();
                // A blocking stream on its own thread; the task below relays its progress to
                // the signal a few times a second. The browser uses async fetch and OPFS instead.
                spawn_download(url, dest, worker);
                loop {
                    day::sleep(200).await;
                    let snapshot = shared.lock().map_err(|_| "download state poisoned")?;
                    sync.state.set(SyncState::Downloading {
                        received: snapshot.received,
                        total: snapshot.total,
                    });
                    if let Some(done) = &snapshot.done {
                        return done.clone().map(|()| Progress {
                            received: snapshot.received,
                            total: snapshot.total,
                            done: None,
                            sha256: snapshot.sha256.clone(),
                        });
                    }
                }
            }
            Source::File(dir) => {
                let bytes = std::fs::read(dir.join(name))
                    .map_err(|e| format!("{}: {e}", dir.join(name).display()))?;
                write_whole(dest, &bytes)
            }
            Source::Asset(dir) => {
                let asset = format!("{dir}/{name}");
                let res = day::resource(day::AssetName::dynamic(asset.clone()))
                    .ok_or_else(|| format!("no bundled asset {asset}"))?;
                write_whole(dest, res.as_slice())
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_whole(dest: &Path, bytes: &[u8]) -> Result<Progress, String> {
    std::fs::write(dest, bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(Progress {
        received: bytes.len() as u64,
        total: bytes.len() as u64,
        done: None,
        sha256: sha256_hex(bytes),
    })
}

/// What the worker thread reports.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct Progress {
    received: u64,
    total: u64,
    /// `Some` once the transfer ended, with why it failed.
    done: Option<Result<(), String>>,
    sha256: String,
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_download(url: String, dest: PathBuf, shared: Arc<Mutex<Progress>>) {
    std::thread::spawn(move || {
        let outcome = stream_to_file(&url, &dest, &shared);
        if let Ok(mut p) = shared.lock() {
            p.done = Some(outcome);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn stream_to_file(url: &str, dest: &Path, shared: &Arc<Mutex<Progress>>) -> Result<(), String> {
    struct Sink<'a> {
        file: std::fs::File,
        hasher: Sha256,
        shared: &'a Arc<Mutex<Progress>>,
    }
    impl StreamSink for Sink<'_> {
        fn head(&mut self, status: u16, headers: &[(String, String)]) -> bool {
            if status != 200 {
                return false;
            }
            // Content-Length sizes the WIRE, and a server that compresses the file on the
            // way sends fewer bytes than land here; the manifest's size is the file's. Fall
            // back on the header only when the manifest gave none.
            let length = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<u64>().ok());
            if let (Some(length), Ok(mut p)) = (length, self.shared.lock())
                && p.total == 0
            {
                p.total = length;
            }
            true
        }
        fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
            self.file
                .write_all(data)
                .map_err(|e| HttpError::Io(e.to_string()))?;
            self.hasher.update(data);
            if let Ok(mut p) = self.shared.lock() {
                p.received += data.len() as u64;
            }
            Ok(())
        }
    }
    let file = std::fs::File::create(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut sink = Sink {
        file,
        hasher: Sha256::new(),
        shared,
    };
    let req = Request::get(url)
        .timeout(std::time::Duration::from_secs(600))
        .allow_expensive(true);
    let result: Result<Download, HttpError> = fetch_streamed(&req, &mut sink);
    let Sink { file, hasher, .. } = sink;
    drop(file);
    match result {
        Ok(d) if d.status == 200 => {
            if let Ok(mut p) = shared.lock() {
                p.sha256 = hasher.finish_hex();
            }
            Ok(())
        }
        Ok(d) => {
            let _ = std::fs::remove_file(dest);
            Err(format!("HTTP {}", d.status))
        }
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            Err(format!("{e:?}"))
        }
    }
}

// --- SHA-256 -------------------------------------------------------------------------------
// The manifest's hash, so a download is trusted only when it is the published file. Written
// out rather than taken from a crate: sixty lines, no dependency, and no ambition beyond
// matching the publisher's `sha256sum`.

pub struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: Vec::with_capacity(64),
            length: 0,
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        let mut data = data;
        if !self.buffer.is_empty() {
            let take = (64 - self.buffer.len()).min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buffer.len() == 64 {
                let block: [u8; 64] = self.buffer[..].try_into().unwrap_or([0; 64]);
                self.block(&block);
                self.buffer.clear();
            }
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            self.block(block);
        }
        self.buffer.extend_from_slice(rest);
    }

    pub fn finish_hex(mut self) -> String {
        let bits = self.length.wrapping_mul(8);
        let mut tail = std::mem::take(&mut self.buffer);
        tail.push(0x80);
        while tail.len() % 64 != 56 {
            tail.push(0);
        }
        tail.extend_from_slice(&bits.to_be_bytes());
        for block in tail.as_chunks::<64>().0 {
            self.block(block);
        }
        self.state.iter().map(|w| format!("{w:08x}")).collect()
    }

    fn block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finish_hex()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_reference_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Across chunk boundaries, in odd pieces.
        let mut h = Sha256::new();
        let text = b"The quick brown fox jumps over the lazy dog";
        for piece in text.chunks(7) {
            h.update(piece);
        }
        assert_eq!(
            h.finish_hex(),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
        let big = vec![0x61u8; 1000];
        assert_eq!(
            sha256_hex(&big),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn manifests_parse_and_bases_resolve() {
        let m = Manifest::parse(
            r#"{"generated_at":"2026-06-23T18:14:45.829Z","count":48205,
                "artifacts":{"sqlite":{"path":"data/stations.sqlite","size":73068544,"sha256":"73b6"}}}"#,
        )
        .expect("parses");
        assert_eq!(m.count, 48205);
        assert_eq!(m.artifacts.sqlite.sha256, "73b6");
        assert!(
            matches!(Source::parse("asset:catalog-fixture"), Source::Asset(a) if a == "catalog-fixture")
        );
        assert!(
            matches!(Source::parse("file:///tmp/cat/"), Source::File(p) if p == std::path::Path::new("/tmp/cat/"))
        );
        assert!(
            matches!(Source::parse("https://x.example/data"), Source::Http(u) if u == "https://x.example/data/")
        );
    }
}
