//! Scrobbling: telling a listening service what played, so a profile there fills in.
//!
//! Two protocols cover the services people use. [`Audioscrobbler`] is the Last.fm web API 2.0,
//! which Libre.fm and every GNU FM server speak as well: a session key from the listener's
//! name and password, then `track.updateNowPlaying` and `track.scrobble` calls signed with
//! MD5. [`ListenBrainz`] is the MetaBrainz service's JSON API with a user token. Both sit
//! behind [`Scrobbler`], so a third service is one more impl.
//!
//! Listens queue in the store (src/library.rs) and go out in batches, so a listen made
//! offline reaches the service when the network does.

use crate::library::Library;
use day::prelude::*;
use day_part_http::Request;
use day_piece_settings::{Pref, pref_enum};
use std::future::Future;
use std::pin::Pin;

type Fut<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// One thing that played, as a service wants to hear it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Listen {
    pub artist: String,
    pub title: String,
    pub album: String,
    /// Seconds; `0.0` when unknown.
    pub duration: f64,
    /// When it started, seconds since the epoch.
    pub started: i64,
}

/// A listening service.
pub trait Scrobbler {
    fn name(&self) -> String;
    /// What is on right now.
    fn now_playing(&self, listen: &Listen) -> Fut<()>;
    /// What finished (up to 50 at a time).
    fn scrobble(&self, listens: &[Listen]) -> Fut<()>;
}

pref_enum! {
    /// Which service the listener uses.
    pub enum Service { Off = "off", LastFm = "lastfm", LibreFm = "librefm", ListenBrainz = "listenbrainz", Custom = "custom" }
}

pub static SERVICE: Pref<Service> = Pref::new("scrobble.service", || Service::Off);
pub static USERNAME: Pref<String> = Pref::new("scrobble.username", String::new);
/// The Last.fm API key and secret the listener registered (Libre.fm takes any key).
pub static API_KEY: Pref<String> = Pref::new("scrobble.api_key", String::new);
pub static API_SECRET: Pref<String> = Pref::new("scrobble.api_secret", String::new);
/// A GNU FM server's `2.0/` endpoint, for `Service::Custom`.
pub static CUSTOM_BASE: Pref<String> = Pref::new("scrobble.base", String::new);
pub static SCROBBLE_MUSIC: Pref<bool> = Pref::new("scrobble.music", || true);
pub static SCROBBLE_RADIO: Pref<bool> = Pref::new("scrobble.radio", || false);
pub static SCROBBLE_PODCASTS: Pref<bool> = Pref::new("scrobble.podcasts", || false);

/// The keychain item holding the session key or token.
const SESSION_KEY: &str = "tunes.scrobble.session";

pub const LASTFM_BASE: &str = "https://ws.audioscrobbler.com/2.0/";
pub const LIBREFM_BASE: &str = "https://libre.fm/2.0/";
pub const LISTENBRAINZ_BASE: &str = "https://api.listenbrainz.org";

// --- Audioscrobbler 2.0 -------------------------------------------------------------------------

/// The Last.fm API 2.0 and its GNU FM relatives.
pub struct Audioscrobbler {
    pub base: String,
    pub api_key: String,
    pub secret: String,
    pub session: String,
}

impl Audioscrobbler {
    /// Sign and send one method call; the answer must be JSON without an `error` member.
    async fn call(
        base: &str,
        api_key: &str,
        secret: &str,
        mut params: Vec<(String, String)>,
    ) -> Result<serde_json::Value, String> {
        params.push(("api_key".into(), api_key.to_string()));
        let sig = api_sig(&params, secret);
        params.push(("api_sig".into(), sig));
        params.push(("format".into(), "json".into()));
        let body = form_encode(&params);
        let req = Request::post(base, body.into_bytes())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", crate::radio::USER_AGENT)
            .timeout(std::time::Duration::from_secs(30));
        let response = day_part_http::fetch_future(req)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let value: serde_json::Value = serde_json::from_str(&response.text()).map_err(|_| {
            format!(
                "HTTP {}: the service answered with something other than JSON",
                response.status
            )
        })?;
        if let Some(code) = value.get("error") {
            let message = value
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("the service refused the request");
            return Err(format!("{message} ({code})"));
        }
        if response.status != 200 {
            return Err(format!("HTTP {}", response.status));
        }
        Ok(value)
    }

    /// A session key from the listener's name and password (`auth.getMobileSession`).
    pub async fn mobile_session(
        base: &str,
        api_key: &str,
        secret: &str,
        username: &str,
        password: &str,
    ) -> Result<String, String> {
        let params = vec![
            ("method".to_string(), "auth.getMobileSession".to_string()),
            ("username".to_string(), username.to_string()),
            ("password".to_string(), password.to_string()),
        ];
        let value = Self::call(base, api_key, secret, params).await?;
        value
            .pointer("/session/key")
            .and_then(|k| k.as_str())
            .map(|k| k.to_string())
            .ok_or_else(|| "the service sent no session key".to_string())
    }
}

impl Scrobbler for Audioscrobbler {
    fn name(&self) -> String {
        host_of(&self.base)
    }

    fn now_playing(&self, listen: &Listen) -> Fut<()> {
        let mut params = vec![
            ("method".to_string(), "track.updateNowPlaying".to_string()),
            ("artist".to_string(), listen.artist.clone()),
            ("track".to_string(), listen.title.clone()),
            ("sk".to_string(), self.session.clone()),
        ];
        if !listen.album.is_empty() {
            params.push(("album".into(), listen.album.clone()));
        }
        if listen.duration > 0.0 {
            params.push(("duration".into(), (listen.duration as i64).to_string()));
        }
        let (base, key, secret) = (self.base.clone(), self.api_key.clone(), self.secret.clone());
        Box::pin(async move { Self::call(&base, &key, &secret, params).await.map(|_| ()) })
    }

    fn scrobble(&self, listens: &[Listen]) -> Fut<()> {
        let mut params = vec![
            ("method".to_string(), "track.scrobble".to_string()),
            ("sk".to_string(), self.session.clone()),
        ];
        for (i, l) in listens.iter().take(50).enumerate() {
            params.push((format!("artist[{i}]"), l.artist.clone()));
            params.push((format!("track[{i}]"), l.title.clone()));
            params.push((format!("timestamp[{i}]"), l.started.to_string()));
            if !l.album.is_empty() {
                params.push((format!("album[{i}]"), l.album.clone()));
            }
            if l.duration > 0.0 {
                params.push((format!("duration[{i}]"), (l.duration as i64).to_string()));
            }
        }
        let (base, key, secret) = (self.base.clone(), self.api_key.clone(), self.secret.clone());
        Box::pin(async move { Self::call(&base, &key, &secret, params).await.map(|_| ()) })
    }
}

/// The API signature: every parameter sorted by name, concatenated as `namevalue`, then the
/// secret, hashed with MD5.
pub fn api_sig(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    md5_hex(s.as_bytes())
}

fn form_encode(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", crate::radio::encode(k), crate::radio::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn host_of(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

// --- ListenBrainz ---------------------------------------------------------------------------------

pub struct ListenBrainz {
    pub base: String,
    pub token: String,
}

impl ListenBrainz {
    fn payload(listen: &Listen, with_time: bool) -> serde_json::Value {
        let mut meta = serde_json::json!({
            "artist_name": listen.artist,
            "track_name": listen.title,
        });
        if !listen.album.is_empty() {
            meta["release_name"] = serde_json::Value::String(listen.album.clone());
        }
        if listen.duration > 0.0 {
            meta["additional_info"] = serde_json::json!({ "duration": listen.duration as i64 });
        }
        let mut item = serde_json::json!({ "track_metadata": meta });
        if with_time {
            item["listened_at"] = serde_json::Value::from(listen.started);
        }
        item
    }

    async fn submit(base: &str, token: &str, body: serde_json::Value) -> Result<(), String> {
        let req = Request::post(
            format!("{base}/1/submit-listens"),
            body.to_string().into_bytes(),
        )
        .header("Content-Type", "application/json")
        .header("Authorization", &format!("Token {token}"))
        .header("User-Agent", crate::radio::USER_AGENT)
        .timeout(std::time::Duration::from_secs(30));
        let response = day_part_http::fetch_future(req)
            .await
            .map_err(|e| format!("{e:?}"))?;
        if response.status != 200 {
            let detail = serde_json::from_str::<serde_json::Value>(&response.text())
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_default();
            return Err(format!("HTTP {} {detail}", response.status)
                .trim()
                .to_string());
        }
        Ok(())
    }

    /// Check a token, answering the user name it belongs to.
    pub async fn validate(base: &str, token: &str) -> Result<String, String> {
        let req = Request::get(format!("{base}/1/validate-token"))
            .header("Authorization", &format!("Token {token}"))
            .header("User-Agent", crate::radio::USER_AGENT)
            .timeout(std::time::Duration::from_secs(30));
        let response = day_part_http::fetch_future(req)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let value: serde_json::Value = serde_json::from_str(&response.text())
            .map_err(|_| format!("HTTP {}", response.status))?;
        if value.get("valid").and_then(|v| v.as_bool()) == Some(true) {
            Ok(value
                .get("user_name")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string())
        } else {
            Err(value
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("the token is not valid")
                .to_string())
        }
    }
}

impl Scrobbler for ListenBrainz {
    fn name(&self) -> String {
        host_of(&self.base)
    }

    fn now_playing(&self, listen: &Listen) -> Fut<()> {
        let body = serde_json::json!({
            "listen_type": "playing_now",
            "payload": [Self::payload(listen, false)],
        });
        let (base, token) = (self.base.clone(), self.token.clone());
        Box::pin(async move { Self::submit(&base, &token, body).await })
    }

    fn scrobble(&self, listens: &[Listen]) -> Fut<()> {
        let items: Vec<serde_json::Value> =
            listens.iter().map(|l| Self::payload(l, true)).collect();
        let body = serde_json::json!({
            "listen_type": if items.len() == 1 { "single" } else { "import" },
            "payload": items,
        });
        let (base, token) = (self.base.clone(), self.token.clone());
        Box::pin(async move { Self::submit(&base, &token, body).await })
    }
}

// --- the app-wide scrobbling ---------------------------------------------------------------------

/// What Settings shows about the account.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Account {
    #[default]
    None,
    SigningIn,
    SignedIn(String),
    Failed(String),
}

pub struct Scrobbling {
    pub account: Signal<Account>,
    /// Whether a flush is in flight.
    pub busy: Signal<bool>,
    /// The last send's outcome, for Settings.
    pub last_result: Signal<Option<Result<usize, String>>>,
    /// The listen on now, as told to the service.
    now: std::cell::RefCell<Option<Listen>>,
}

impl Scrobbling {
    pub fn new() -> Scrobbling {
        let account = if SERVICE.get_untracked() == Service::Off {
            Account::None
        } else {
            match session_secret() {
                Some(_) => Account::SignedIn(USERNAME.get_untracked()),
                None => Account::None,
            }
        };
        Scrobbling {
            account: Signal::new(account),
            busy: Signal::new(false),
            last_result: Signal::new(None),
            now: std::cell::RefCell::new(None),
        }
    }

    /// The service as configured, if signed in.
    fn scrobbler(&self) -> Option<Box<dyn Scrobbler>> {
        let session = session_secret()?;
        Some(match SERVICE.get_untracked() {
            Service::Off => return None,
            Service::LastFm => Box::new(Audioscrobbler {
                base: LASTFM_BASE.into(),
                api_key: API_KEY.get_untracked(),
                secret: API_SECRET.get_untracked(),
                session,
            }),
            Service::LibreFm => Box::new(Audioscrobbler {
                base: LIBREFM_BASE.into(),
                api_key: key_or_default(),
                secret: API_SECRET.get_untracked(),
                session,
            }),
            Service::Custom => Box::new(Audioscrobbler {
                base: custom_base(),
                api_key: key_or_default(),
                secret: API_SECRET.get_untracked(),
                session,
            }),
            Service::ListenBrainz => Box::new(ListenBrainz {
                base: LISTENBRAINZ_BASE.into(),
                token: session,
            }),
        })
    }

    pub fn is_signed_in(&self) -> bool {
        matches!(self.account.get(), Account::SignedIn(_))
    }

    /// Sign in with a name and password (Audioscrobbler) or a token (ListenBrainz): the
    /// session lands in the keychain and the account signal says how it went.
    pub fn sign_in(&'static self, username: String, secret: String) {
        self.account.set(Account::SigningIn);
        let service = SERVICE.get_untracked();
        day::task(async move {
            let result = match service {
                Service::Off => Err("choose a service first".to_string()),
                Service::ListenBrainz => ListenBrainz::validate(LISTENBRAINZ_BASE, secret.trim())
                    .await
                    .map(|user| (secret.trim().to_string(), user)),
                Service::LastFm | Service::LibreFm | Service::Custom => {
                    let base = match service {
                        Service::LastFm => LASTFM_BASE.to_string(),
                        Service::LibreFm => LIBREFM_BASE.to_string(),
                        _ => custom_base(),
                    };
                    let key = if service == Service::LastFm {
                        API_KEY.get_untracked()
                    } else {
                        key_or_default()
                    };
                    if key.trim().is_empty() {
                        Err("an API key is needed".to_string())
                    } else {
                        Audioscrobbler::mobile_session(
                            &base,
                            key.trim(),
                            API_SECRET.get_untracked().trim(),
                            username.trim(),
                            &secret,
                        )
                        .await
                        .map(|sk| (sk, username.trim().to_string()))
                    }
                }
            };
            match result {
                Ok((session, user)) => {
                    store_session(&session);
                    USERNAME.set(user.clone());
                    self.account.set(Account::SignedIn(user));
                    self.flush();
                }
                Err(e) => self.account.set(Account::Failed(e)),
            }
        });
    }

    pub fn sign_out(&self) {
        clear_session();
        self.account.set(Account::None);
        self.last_result.set(None);
    }

    /// Whether listens of this kind go out.
    pub fn wants(&self, kind: i64) -> bool {
        if !self.is_signed_in_untracked() {
            return false;
        }
        match kind {
            crate::library::KIND_TRACK => SCROBBLE_MUSIC.get_untracked(),
            crate::library::KIND_STATION => SCROBBLE_RADIO.get_untracked(),
            crate::library::KIND_EPISODE => SCROBBLE_PODCASTS.get_untracked(),
            _ => false,
        }
    }

    fn is_signed_in_untracked(&self) -> bool {
        matches!(self.account.get_untracked(), Account::SignedIn(_))
    }

    /// Tell the service what is on now (once per listen).
    pub fn now_playing(&'static self, listen: Listen) {
        if listen.title.trim().is_empty() {
            return;
        }
        if self.now.borrow().as_ref() == Some(&listen) {
            return;
        }
        *self.now.borrow_mut() = Some(listen.clone());
        let Some(service) = self.scrobbler() else {
            return;
        };
        let fut = service.now_playing(&listen);
        day::task(async move {
            if let Err(e) = fut.await {
                debug!("now playing not sent: {e}");
            }
        });
    }

    /// Queue a finished listen and try to send the queue.
    pub fn listened(&'static self, lib: &Library, listen: &Listen) {
        if listen.title.trim().is_empty() || listen.artist.trim().is_empty() {
            return;
        }
        lib.queue_scrobble(
            &listen.artist,
            &listen.title,
            &listen.album,
            listen.duration,
            listen.started,
        );
        self.flush();
    }

    /// Send everything queued, oldest first, in batches of fifty.
    pub fn flush(&'static self) {
        if self.busy.get_untracked() {
            return;
        }
        let Some(service) = self.scrobbler() else {
            return;
        };
        let lib = crate::App::app().library;
        let ids: Vec<u64> = Library::ids_of(&lib.pending_scrobbles())
            .into_iter()
            .take(50)
            .collect();
        if ids.is_empty() {
            return;
        }
        let listens: Vec<Listen> = ids
            .iter()
            .filter_map(|id| lib.scrobble_row(*id))
            .map(|s| Listen {
                artist: s.artist,
                title: s.title,
                album: s.album,
                duration: s.duration,
                started: s.started,
            })
            .collect();
        self.busy.set(true);
        let fut = service.scrobble(&listens);
        day::task(async move {
            let result = fut.await;
            let lib = crate::App::app().library;
            match result {
                Ok(()) => {
                    lib.scrobbled(&ids);
                    self.last_result.set(Some(Ok(ids.len())));
                    self.busy.set(false);
                    // More waiting? Keep going.
                    if !lib.pending_scrobbles().ids_untracked().is_empty() {
                        self.flush();
                    }
                }
                Err(e) => {
                    warn!("scrobble failed: {e}");
                    lib.scrobble_failed(&ids);
                    self.last_result.set(Some(Err(e)));
                    self.busy.set(false);
                }
            }
        });
    }
}

impl Default for Scrobbling {
    fn default() -> Self {
        Scrobbling::new()
    }
}

fn key_or_default() -> String {
    let k = API_KEY.get_untracked();
    if k.trim().is_empty() {
        // GNU FM servers accept any key; this one names the app.
        "daytunes".to_string()
    } else {
        k.trim().to_string()
    }
}

fn custom_base() -> String {
    let b = CUSTOM_BASE.get_untracked().trim().to_string();
    if b.is_empty() {
        LIBREFM_BASE.to_string()
    } else if b.ends_with('/') {
        b
    } else {
        format!("{b}/")
    }
}

/// The session key or token, from the keychain where there is one, else the preferences
/// store (the web).
fn session_secret() -> Option<String> {
    match day_part_keychain::get(SESSION_KEY) {
        Ok(s) => s.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty()),
        Err(day_part_keychain::KeychainError::Unsupported) => {
            day::prefs::get(SESSION_KEY).filter(|s| !s.is_empty())
        }
        Err(_) => None,
    }
}

fn store_session(session: &str) {
    let secret = day_part_keychain::Secret::from(session.to_string());
    match day_part_keychain::set(
        SESSION_KEY,
        &secret,
        day_part_keychain::Protection::AfterFirstUnlock,
    ) {
        Ok(()) => {}
        Err(day_part_keychain::KeychainError::Unsupported) => {
            day::prefs::set(SESSION_KEY, session);
        }
        Err(e) => {
            warn!("the session could not be kept: {e}");
            day::prefs::set(SESSION_KEY, session);
        }
    }
}

fn clear_session() {
    let _ = day_part_keychain::remove(SESSION_KEY);
    day::prefs::remove(SESSION_KEY);
}

// --- MD5 ---------------------------------------------------------------------------------------------
// The Audioscrobbler signature. Written out rather than taken from a crate: the algorithm is
// fifty lines and nothing here needs it to be fast.

pub fn md5_hex(input: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let (mut a0, mut b0, mut c0, mut d0) = (
        0x6745_2301u32,
        0xefcd_ab89u32,
        0x98ba_dcfeu32,
        0x1032_5476u32,
    );
    let mut msg = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for (i, word) in m.iter_mut().enumerate() {
            *word = u32::from_le_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    [a0, b0, c0, d0]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_the_reference_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(md5_hex(&[0x61u8; 1000]), "cabe45dcc9ae5b66ba86600cca6b8ba8");
    }

    #[test]
    fn signatures_sort_parameters() {
        let params = vec![
            ("method".to_string(), "auth.getMobileSession".to_string()),
            ("username".to_string(), "ada".to_string()),
            ("password".to_string(), "pw".to_string()),
            ("api_key".to_string(), "k".to_string()),
        ];
        assert_eq!(
            api_sig(&params, "s"),
            md5_hex(b"api_keykmethodauth.getMobileSessionpasswordpwusernameadas")
        );
        assert_eq!(form_encode(&[("a b".into(), "c&d".into())]), "a%20b=c%26d");
    }
}
