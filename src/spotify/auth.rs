//! Spotify ClientToken + Hashcash + Login5 v3/v4 authentication.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::sync::{Mutex, RwLock};

use crate::vault;

use super::proto::{ProtoReader, ProtoWriter, decode_hex, encode, hex_lower, hex_upper};

pub const CLIENT_ID: &str = "9a8d2f0ce77a4e248bb71fefcb557637";
pub const APP_VERSION: &str = "9.2.0.26";
pub const USER_AGENT: &str = "Spotify/9.2.0.26 Android/35 (Pixel 9)";

const CLIENT_TOKEN_URL: &str = "https://clienttoken.spotify.com/v1/clienttoken";
const LOGIN5_V4_URL: &str = "https://login5.spotify.com/v4/login";
const LOGIN5_V3_URL: &str = "https://login5.spotify.com/v3/login";
const VAULT_KEY: &str = "spotify login5";
const MAX_CHALLENGE_ROUNDS: usize = 4;

static COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredSession {
    username: String,
    #[serde(default)]
    display_name: Option<String>,
    device_id: String,
    #[serde(default)]
    stored_cred: Vec<u8>,
    #[serde(default)]
    sp_dc: Option<String>,
}

#[derive(Clone, Debug)]
struct CachedClientToken {
    token: String,
    refresh_at: Instant,
}

#[derive(Clone, Debug)]
struct CachedAccessToken {
    username: String,
    token: String,
    expires_at: Instant,
    client_id: String,
    app_version: String,
    user_agent: String,
    is_web: bool,
}

#[derive(Clone, Debug)]
pub struct SpotifyTokens {
    pub username: String,
    pub access_token: String,
    pub client_token: String,
    pub device_id: String,
    pub client_id: String,
    pub app_version: String,
    pub user_agent: String,
    pub is_web: bool,
}

#[derive(Clone, Debug)]
pub struct EmailCodeChallenge {
    pub email: String,
    pub context: Vec<u8>,
    pub flow_id: String,
    pub method: i32,
    pub code_length: i32,
    pub solutions: Vec<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub enum EmailLoginStep {
    SignedIn(String),
    NeedsCode(EmailCodeChallenge),
}

pub struct SpotifyAuth {
    data_dir: PathBuf,
    device_id: RwLock<String>,
    client_token: RwLock<Option<CachedClientToken>>,
    access_token: RwLock<Option<CachedAccessToken>>,
    stored: RwLock<Option<StoredSession>>,
    saved_user: std::sync::RwLock<Option<String>>,
    saved_display: std::sync::RwLock<Option<String>>,
    refresh_lock: Mutex<()>,
}

impl SpotifyAuth {
    pub fn new(data_dir: PathBuf) -> Self {
        let loaded = load_stored(&data_dir);
        let saved_user = loaded.as_ref().map(|s| s.username.clone());
        let saved_display = loaded
            .as_ref()
            .and_then(|s| s.display_name.clone())
            .filter(|s| !s.is_empty());
        let device_id = loaded
            .as_ref()
            .map(|s| s.device_id.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(random_device_id);
        Self {
            data_dir,
            device_id: RwLock::new(device_id),
            client_token: RwLock::new(None),
            access_token: RwLock::new(None),
            stored: RwLock::new(loaded),
            saved_user: std::sync::RwLock::new(saved_user),
            saved_display: std::sync::RwLock::new(saved_display),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Cached Spotify username loaded from disk/vault on startup (non-blocking).
    pub fn saved_username(&self) -> Option<String> {
        use crate::sync::RwLockExt;
        self.saved_user.read_unpoisoned().clone()
    }

    /// Cached human display name if known, falling back to the raw username.
    pub fn saved_display_name(&self) -> Option<String> {
        use crate::sync::RwLockExt;
        self.saved_display
            .read_unpoisoned()
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.saved_username())
    }

    fn set_saved_user(&self, username: Option<String>, display_name: Option<String>) {
        use crate::sync::RwLockExt;
        *self.saved_user.write_unpoisoned() = username;
        *self.saved_display.write_unpoisoned() = display_name.filter(|s| !s.is_empty());
    }

    pub async fn stored_display_name(&self) -> Option<String> {
        self.stored
            .read()
            .await
            .as_ref()
            .and_then(|s| s.display_name.clone())
            .filter(|s| !s.is_empty())
    }

    pub async fn update_display_name(&self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        {
            use crate::sync::RwLockExt;
            *self.saved_display.write_unpoisoned() = Some(name.to_owned());
        }
        let mut stored = self.stored.write().await;
        if let Some(s) = stored.as_mut() {
            if s.display_name.as_deref() != Some(name) {
                s.display_name = Some(name.to_owned());
                if let Ok(bytes) = serde_json::to_vec(s) {
                    let _ = vault::set(VAULT_KEY, &self.data_dir, &bytes);
                }
            }
        }
    }

    pub async fn signed_in_username(&self) -> Option<String> {
        if let Some(tok) = self.access_token.read().await.as_ref() {
            return Some(tok.username.clone());
        }
        self.stored.read().await.as_ref().map(|s| s.username.clone())
    }

    pub async fn is_signed_in(&self) -> bool {
        let has_access = self.access_token.read().await.is_some();
        has_access || self.stored.read().await.is_some()
    }

    pub async fn device_id(&self) -> String {
        self.device_id.read().await.clone()
    }

    pub async fn is_web(&self) -> bool {
        if let Some(a) = self.access_token.read().await.as_ref() {
            return a.is_web;
        }
        self.stored.read().await.as_ref().is_some_and(|s| s.sp_dc.is_some())
    }

    pub async fn logout(&self) {
        let _guard = self.refresh_lock.lock().await;
        *self.client_token.write().await = None;
        *self.access_token.write().await = None;
        *self.stored.write().await = None;
        self.set_saved_user(None, None);
        let _ = vault::delete(VAULT_KEY, &self.data_dir);
    }

    pub async fn restore_if_needed(&self, http: &Client) -> Result<Option<String>> {
        if let Some(u) = self.signed_in_username().await {
            match self.tokens(http).await {
                Ok(_) => {
                    log::info!("spotify session restored for {u}");
                    return Ok(Some(u));
                }
                Err(e) => {
                    log::warn!("spotify session restore failed for {u}: {e:#}");
                    return Err(e);
                }
            }
        }
        Ok(None)
    }

    pub async fn tokens(&self, http: &Client) -> Result<SpotifyTokens> {
        let now = Instant::now();
        {
            let ct = self.client_token.read().await;
            let at = self.access_token.read().await;
            if let (Some(c), Some(a)) = (ct.as_ref(), at.as_ref()) {
                if now < c.refresh_at && now < a.expires_at {
                    return Ok(SpotifyTokens {
                        username: a.username.clone(),
                        access_token: a.token.clone(),
                        client_token: c.token.clone(),
                        device_id: self.device_id().await,
                        client_id: a.client_id.clone(),
                        app_version: a.app_version.clone(),
                        user_agent: a.user_agent.clone(),
                        is_web: a.is_web,
                    });
                }
            }
        }

        let _guard = self.refresh_lock.lock().await;
        let now = Instant::now();
        let dev = self.device_id().await;

        let saved_opt = self.stored.read().await.clone();
        if let Some(ref saved) = saved_opt
            && let Some(ref sp_dc) = saved.sp_dc
        {
            let ct = self.client_token.read().await.clone();
            let at = self.access_token.read().await.clone();
            if let (Some(c), Some(a)) = (ct, at)
                && a.is_web
                && now < c.refresh_at
                && now < a.expires_at
            {
                return Ok(SpotifyTokens {
                    username: a.username,
                    access_token: a.token,
                    client_token: c.token,
                    device_id: saved.device_id.clone(),
                    client_id: a.client_id,
                    app_version: a.app_version,
                    user_agent: a.user_agent,
                    is_web: true,
                });
            }
            let wt = crate::transfer::spotify::web_tokens_from_cookie(http, sp_dc).await?;
            let ct_val = wt.client_token.clone();
            let dev_val = wt.device_id.clone();
            let access = self
                .save_web_tokens(saved.username.clone(), saved.display_name.clone(), sp_dc, wt)
                .await;
            return Ok(SpotifyTokens {
                username: access.username,
                access_token: access.token,
                client_token: ct_val,
                device_id: dev_val,
                client_id: access.client_id,
                app_version: access.app_version,
                user_agent: access.user_agent,
                is_web: true,
            });
        }

        let cached_ct = self
            .client_token
            .read()
            .await
            .as_ref()
            .filter(|c| now < c.refresh_at)
            .map(|c| c.token.clone());
        let client_token = match cached_ct {
            Some(tok) => tok,
            None => {
                let fresh = fetch_client_token(http, &dev).await?;
                let tok = fresh.token.clone();
                *self.client_token.write().await = Some(fresh);
                tok
            }
        };

        let cached_at = self
            .access_token
            .read()
            .await
            .as_ref()
            .filter(|a| !a.is_web && now < a.expires_at)
            .cloned();
        let access = match cached_at {
            Some(a) => a,
            None => {
                let saved = saved_opt.ok_or_else(|| anyhow!("Not signed in to Spotify"))?;
                let ok = login5_stored(http, &saved.device_id, &client_token, &saved.username, &saved.stored_cred)
                    .await?;
                self.save_login_ok(ok, &saved.device_id).await
            }
        };

        Ok(SpotifyTokens {
            username: access.username,
            access_token: access.token,
            client_token,
            device_id: dev,
            client_id: access.client_id,
            app_version: access.app_version,
            user_agent: access.user_agent,
            is_web: false,
        })
    }

    pub async fn invalidate_access_token(&self) {
        *self.access_token.write().await = None;
    }

    pub async fn invalidate_client_token(&self) {
        *self.client_token.write().await = None;
    }

    pub async fn login_sp_dc(&self, http: &Client, sp_dc: &str) -> Result<String> {
        let _guard = self.refresh_lock.lock().await;
        let wt = crate::transfer::spotify::web_tokens_from_cookie(http, sp_dc).await?;
        let me: serde_json::Value = http
            .get("https://api.spotify.com/v1/me")
            .bearer_auth(&wt.access_token)
            .header("User-Agent", wt.user_agent)
            .timeout(Duration::from_secs(15))
            .send()
            .await?
            .error_for_status()
            .context("Failed to fetch Spotify profile")?
            .json()
            .await?;
        let username = me["id"]
            .as_str()
            .or_else(|| me["display_name"].as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Spotify returned no username for signed-in session"))?
            .to_owned();
        let display_name = me["display_name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let access = self.save_web_tokens(username, display_name, sp_dc, wt).await;
        Ok(access.username)
    }

    pub async fn login_password(
        &self,
        http: &Client,
        username: &str,
        password: &str,
    ) -> Result<String> {
        let _guard = self.refresh_lock.lock().await;
        let dev = self.device_id().await;
        let client_token = self.ensure_client_token_locked(http, &dev).await?;
        let user = username.trim();
        if user.is_empty() || password.is_empty() {
            bail!("Enter both your Spotify username/email and password.");
        }
        let ok = login5_password(http, &dev, &client_token, user, password).await?;
        let access = self.save_login_ok(ok, &dev).await;
        Ok(access.username)
    }

    pub async fn start_email_login(&self, http: &Client, email: &str) -> Result<EmailLoginStep> {
        let _guard = self.refresh_lock.lock().await;
        let dev = self.device_id().await;
        let client_token = self.ensure_client_token_locked(http, &dev).await?;
        let email = email.trim();
        if email.is_empty() || !email.contains('@') {
            bail!("Enter a valid Spotify email address.");
        }
        log::info!("spotify login5 email start");
        let flow_id = random_uuid_v4();
        let outcome = login5_email_round(
            http,
            &dev,
            &client_token,
            email,
            &flow_id,
            None,
            Vec::new(),
        )
        .await?;
        match outcome {
            Login5Round::Ok(ok) => {
                let access = self.save_login_ok(ok, &dev).await;
                Ok(EmailLoginStep::SignedIn(access.username))
            }
            Login5Round::NeedsCode(ch) => Ok(EmailLoginStep::NeedsCode(ch)),
        }
    }

    pub async fn finish_email_login(
        &self,
        http: &Client,
        challenge: &EmailCodeChallenge,
        code: &str,
    ) -> Result<String> {
        let _guard = self.refresh_lock.lock().await;
        let code = code.trim();
        if code.is_empty() {
            bail!("Enter the verification code sent to your email.");
        }
        let dev = self.device_id().await;
        let client_token = self.ensure_client_token_locked(http, &dev).await?;
        let mut solutions = challenge.solutions.clone();
        solutions.push(encode_code_solution(code));
        match login5_email_round(
            http,
            &dev,
            &client_token,
            &challenge.email,
            &challenge.flow_id,
            Some(&challenge.context),
            solutions,
        )
        .await?
        {
            Login5Round::Ok(ok) => {
                let access = self.save_login_ok(ok, &dev).await;
                Ok(access.username)
            }
            Login5Round::NeedsCode(_) => {
                bail!("Verification code was rejected or expired. Request a new code and try again.")
            }
        }
    }

    async fn ensure_client_token_locked(&self, http: &Client, device_id: &str) -> Result<String> {
        let now = Instant::now();
        let cached = self
            .client_token
            .read()
            .await
            .as_ref()
            .filter(|c| now < c.refresh_at)
            .map(|c| c.token.clone());
        if let Some(tok) = cached {
            return Ok(tok);
        }
        let fresh = fetch_client_token(http, device_id).await?;
        let tok = fresh.token.clone();
        *self.client_token.write().await = Some(fresh);
        Ok(tok)
    }

    async fn save_web_tokens(
        &self,
        username: String,
        display_name: Option<String>,
        sp_dc: &str,
        wt: crate::transfer::spotify::WebTokens,
    ) -> CachedAccessToken {
        *self.device_id.write().await = wt.device_id.clone();
        *self.client_token.write().await = Some(CachedClientToken {
            token: wt.client_token,
            refresh_at: Instant::now() + Duration::from_secs(3000),
        });
        let cached = CachedAccessToken {
            username: username.clone(),
            token: wt.access_token,
            expires_at: Instant::now() + Duration::from_secs(3000),
            client_id: wt.client_id,
            app_version: wt.client_version,
            user_agent: wt.user_agent.to_string(),
            is_web: true,
        };
        *self.access_token.write().await = Some(cached.clone());
        let saved = StoredSession {
            username,
            display_name: display_name.clone(),
            device_id: wt.device_id,
            stored_cred: Vec::new(),
            sp_dc: Some(sp_dc.to_owned()),
        };
        if let Ok(bytes) = serde_json::to_vec(&saved) {
            let _ = vault::set(VAULT_KEY, &self.data_dir, &bytes);
        }
        self.set_saved_user(Some(saved.username.clone()), display_name);
        *self.stored.write().await = Some(saved);
        cached
    }

    async fn save_login_ok(&self, ok: LoginOk, device_id: &str) -> CachedAccessToken {
        let ttl = ok.expires_secs.max(120) as u64;
        let refresh_in = ttl.saturating_sub(60).max(60);
        let cached = CachedAccessToken {
            username: ok.username.clone(),
            token: ok.access_token,
            expires_at: Instant::now() + Duration::from_secs(refresh_in),
            client_id: CLIENT_ID.to_string(),
            app_version: APP_VERSION.to_string(),
            user_agent: USER_AGENT.to_string(),
            is_web: false,
        };
        *self.access_token.write().await = Some(cached.clone());
        if !ok.stored_cred.is_empty() {
            let saved = StoredSession {
                username: ok.username,
                display_name: None,
                device_id: device_id.to_string(),
                stored_cred: ok.stored_cred,
                sp_dc: None,
            };
            if let Ok(bytes) = serde_json::to_vec(&saved) {
                let _ = vault::set(VAULT_KEY, &self.data_dir, &bytes);
            }
            self.set_saved_user(Some(saved.username.clone()), None);
            *self.stored.write().await = Some(saved);
        }
        cached
    }
}

fn load_stored(data_dir: &PathBuf) -> Option<StoredSession> {
    let bytes = vault::get(VAULT_KEY, data_dir)?;
    serde_json::from_slice::<StoredSession>(&bytes).ok()
}

pub fn solve_hashcash(context: &[u8], prefix: &[u8], bits: u32) -> (Vec<u8>, Duration) {
    solve_hashcash_bounded(context, prefix, bits, Duration::from_secs(30))
        .unwrap_or_else(|_| (vec![0u8; 16], Duration::from_secs(30)))
}

fn solve_hashcash_bounded(
    context: &[u8],
    prefix: &[u8],
    bits: u32,
    budget: Duration,
) -> Result<(Vec<u8>, Duration)> {
    if bits > 40 {
        bail!("Spotify Hashcash difficulty too high ({bits} bits)");
    }
    let started = Instant::now();
    let ctx_hash = Sha1::digest(context);
    let seed = i64::from_be_bytes(ctx_hash[12..20].try_into().expect("sha1 slice"));
    let workers = std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 4))
        .unwrap_or(2);
    let found = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::with_capacity(workers);

    for worker in 0..workers {
        let prefix = prefix.to_vec();
        let found = found.clone();
        handles.push(std::thread::spawn(move || -> Option<Vec<u8>> {
            let mut counter: i64 = worker as i64;
            let step = workers as i64;
            let mut input = Vec::with_capacity(prefix.len() + 16);
            let mut iters = 0u32;
            loop {
                if iters & 1023 == 0 {
                    if found.load(Ordering::Relaxed) || started.elapsed() >= budget {
                        return None;
                    }
                }
                iters = iters.wrapping_add(1);
                let n1 = seed.wrapping_add(counter).to_be_bytes();
                let n2 = counter.to_be_bytes();
                input.clear();
                input.extend_from_slice(&prefix);
                input.extend_from_slice(&n1);
                input.extend_from_slice(&n2);
                let h = Sha1::digest(&input);
                let tail = i64::from_be_bytes(h[12..20].try_into().expect("sha1 slice"));
                if tail.trailing_zeros() >= bits {
                    found.store(true, Ordering::Relaxed);
                    let mut suffix = Vec::with_capacity(16);
                    suffix.extend_from_slice(&n1);
                    suffix.extend_from_slice(&n2);
                    return Some(suffix);
                }
                counter = counter.wrapping_add(step);
            }
        }));
    }

    let mut answer = None;
    for h in handles {
        if let Ok(Some(s)) = h.join() {
            answer = Some(s);
        }
    }
    match answer {
        Some(suffix) => Ok((suffix, started.elapsed())),
        None => bail!("Solving Spotify Hashcash ({bits} bits) timed out"),
    }
}

async fn solve_hashcash_async(
    context: Vec<u8>,
    prefix: Vec<u8>,
    bits: u32,
) -> Result<(Vec<u8>, Duration)> {
    tokio::task::spawn_blocking(move || {
        solve_hashcash_bounded(&context, &prefix, bits, Duration::from_secs(20))
    })
    .await
    .map_err(|e| anyhow!("Hashcash task failed: {e}"))?
}

async fn fetch_client_token(http: &Client, device_id: &str) -> Result<CachedClientToken> {
    let req_bytes = encode(|w| {
        w.write_enum(1, 1); // CLIENT_DATA
        w.write_msg(2, |cd| {
            cd.write_str(1, APP_VERSION);
            cd.write_str(2, CLIENT_ID);
            cd.write_msg(3, |csd| {
                csd.write_msg(1, |psd| {
                    psd.write_msg(1, |nad| {
                        nad.write_msg(1, |sd| {
                            sd.write_int32(1, 1080);
                            sd.write_int32(2, 2400);
                            sd.write_int32(3, 411);
                            sd.write_int32(4, 420);
                            sd.write_int32(5, 420);
                        });
                        nad.write_str(2, "15");
                        nad.write_uint32(3, 35);
                        nad.write_str(4, "tokay");
                        nad.write_str(5, "Pixel 9");
                        nad.write_str(6, "google");
                        nad.write_str(7, "Google");
                        nad.write_uint32(8, 25);
                        nad.write_str(10, "com.android.vending");
                    });
                });
                csd.write_str(2, device_id);
            });
        });
    });

    let mut body = post_client_token(http, &req_bytes).await?;
    for _ in 0..MAX_CHALLENGE_ROUNDS {
        let resp = ProtoReader::parse(&body)?;
        if let Some(granted) = resp.message(2) {
            let token = granted
                .text(1)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!("Empty Spotify client token"))?;
            let refresh_secs = granted.number(3).or(granted.number(2)).unwrap_or(3600).max(120) as u64;
            log::info!("spotify: client-token acquired (expires/refresh={}s)", refresh_secs);
            return Ok(CachedClientToken {
                token,
                refresh_at: Instant::now() + Duration::from_secs(refresh_secs.saturating_sub(60).max(60)),
            });
        }
        let Some(challenges) = resp.message(3) else {
            bail!("Unexpected Spotify ClientToken response");
        };
        let state = challenges.text(1).unwrap_or_default();
        let challenge_list = challenges.messages(2);
        let mut answers: Vec<Vec<u8>> = Vec::new();
        for ch in challenge_list {
            let ctype = ch.number(1).unwrap_or(0);
            if ctype == 3 {
                if let Some(hc) = ch.message(4) {
                    let bits = hc.number(1).unwrap_or(10).max(0) as u32;
                    let prefix_hex = hc.text(2).unwrap_or_default();
                    let prefix = decode_hex(&prefix_hex)
                        .ok_or_else(|| anyhow!("Invalid Hashcash prefix in ClientToken"))?;
                    log::info!("spotify clienttoken hashcash bits={bits}");
                    let (suffix, _) = solve_hashcash_async(Vec::new(), prefix, bits).await?;
                    answers.push(encode(|aw| {
                        aw.write_enum(1, 3);
                        aw.write_msg(4, |ha| {
                            ha.write_str(1, &hex_upper(&suffix));
                        });
                    }));
                }
            }
        }
        if answers.is_empty() {
            bail!("Unsupported Spotify ClientToken challenge");
        }
        let answer_req = encode(|w| {
            w.write_enum(1, 2); // CHALLENGE_ANSWERS
            w.write_msg(3, |ca| {
                ca.write_str(1, &state);
                for ans in &answers {
                    ca.write_bytes(2, ans);
                }
            });
        });
        body = post_client_token(http, &answer_req).await?;
    }
    bail!("Spotify ClientToken challenge did not complete")
}

async fn post_client_token(http: &Client, payload: &[u8]) -> Result<Vec<u8>> {
    let resp = http
        .post(CLIENT_TOKEN_URL)
        .timeout(Duration::from_secs(15))
        .header("Accept", "application/x-protobuf")
        .header("Content-Type", "application/x-protobuf")
        .header("User-Agent", USER_AGENT)
        .body(payload.to_vec())
        .send()
        .await
        .context("Spotify ClientToken request failed")?;
    let status = resp.status();
    let bytes = resp.bytes().await?.to_vec();
    if !status.is_success() {
        bail!("Spotify ClientToken HTTP {status}");
    }
    Ok(bytes)
}

#[derive(Clone, Debug)]
struct LoginOk {
    username: String,
    access_token: String,
    stored_cred: Vec<u8>,
    expires_secs: i32,
}

enum Login5Round {
    Ok(LoginOk),
    NeedsCode(EmailCodeChallenge),
}

async fn login5_stored(
    http: &Client,
    device_id: &str,
    client_token: &str,
    username: &str,
    stored_cred: &[u8],
) -> Result<LoginOk> {
    login5_auto_hashcash(http, LOGIN5_V3_URL, device_id, client_token, None, |w, ctx, sols, _flow| {
        encode_login5_base(w, device_id, ctx, sols, None);
        w.write_msg(100, |sc| {
            sc.write_str(1, username);
            sc.write_bytes(2, stored_cred);
        });
    })
    .await
    .map_err(|(_, e)| e)
}

async fn login5_password(
    http: &Client,
    device_id: &str,
    client_token: &str,
    username: &str,
    password: &str,
) -> Result<LoginOk> {
    let res = login5_auto_hashcash(http, LOGIN5_V4_URL, device_id, client_token, None, |w, ctx, sols, flow| {
        encode_login5_base(w, device_id, ctx, sols, Some(flow));
        w.write_msg(101, |pw| {
            pw.write_str(1, username);
            pw.write_str(2, password);
        });
    })
    .await;
    match res {
        Ok(ok) => Ok(ok),
        Err((_protocol_or_cred, v4_err)) => {
            let flow = random_uuid_v4();
            login5_auto_hashcash(
                http,
                LOGIN5_V3_URL,
                device_id,
                client_token,
                Some(flow),
                |w, ctx, sols, _| {
                    encode_login5_base(w, device_id, ctx, sols, None);
                    w.write_msg(101, |pw| {
                        pw.write_str(1, username);
                        pw.write_str(2, password);
                    });
                },
            )
            .await
            .map_err(|(_, _)| v4_err)
        }
    }
}

async fn login5_email_round(
    http: &Client,
    device_id: &str,
    client_token: &str,
    email: &str,
    flow_id: &str,
    initial_context: Option<&[u8]>,
    initial_solutions: Vec<Vec<u8>>,
) -> Result<Login5Round> {
    let mut ctx_opt = initial_context.map(|b| b.to_vec());
    let mut solutions = initial_solutions;

    for round in 0..MAX_CHALLENGE_ROUNDS {
        let payload = encode(|w| {
            encode_login5_base(w, device_id, ctx_opt.as_deref(), &solutions, Some(flow_id));
            w.write_msg(111, |em| {
                em.write_str(1, email);
            });
        });
        log::info!("spotify login5 email request round={round}");
        let (status, raw) = post_login5(http, LOGIN5_V4_URL, client_token, &payload).await?;
        log::info!("spotify login5 email response round={round} status={status} bytes={}", raw.len());
        if !status.is_success() {
            bail!("Spotify Login5 HTTP {status}");
        }
        let resp = ProtoReader::parse(&raw)?;
        if let Some(ok) = parse_login5_ok(&resp)? {
            log::info!("spotify login5 email signed in user={}", ok.username);
            return Ok(Login5Round::Ok(ok));
        }
        if let Some(err_code) = resp.number(2) {
            log::warn!("spotify login5 email error={err_code} round={round}");
            bail!("{}", describe_login5_error(err_code));
        }
        let next_ctx = resp.bytes(5).unwrap_or(&[]).to_vec();
        let Some(challenges_msg) = resp.message(3) else {
            bail!("Unexpected Spotify Login5 response");
        };
        let mut round_hc_solutions = Vec::new();
        let mut code_challenge: Option<(i32, i32)> = None;
        for ch in challenges_msg.messages(1) {
            if let Some(hc) = ch.message(1) {
                let prefix = hc.bytes(1).unwrap_or(&[]).to_vec();
                let bits = hc.number(2).unwrap_or(10).max(0) as u32;
                log::info!("spotify login5 email solving hashcash bits={bits} round={round}");
                let (suffix, dur) = solve_hashcash_async(next_ctx.clone(), prefix, bits).await?;
                log::info!("spotify login5 email solved hashcash in {:?}", dur);
                round_hc_solutions.push(encode_hashcash_solution(&suffix, dur));
            }
            if let Some(code) = ch.message(2) {
                let method = code.number(1).unwrap_or(1);
                let len = code.number(2).unwrap_or(6);
                log::info!("spotify login5 email code challenge method={method} len={len} round={round}");
                code_challenge = Some((method, len));
            }
        }
        ctx_opt = Some(next_ctx.clone());
        solutions = round_hc_solutions.clone();
        if let Some((method, code_length)) = code_challenge {
            return Ok(Login5Round::NeedsCode(EmailCodeChallenge {
                email: email.to_string(),
                context: next_ctx,
                flow_id: flow_id.to_string(),
                method,
                code_length,
                solutions: round_hc_solutions,
            }));
        }
        if solutions.is_empty() {
            log::warn!("spotify login5 email unsupported challenge at round={round}");
            bail!("Spotify requested an interactive challenge DawnFM cannot complete");
        }
    }
    bail!("Spotify Login5 exceeded challenge rounds")
}

async fn login5_auto_hashcash(
    http: &Client,
    url: &str,
    device_id: &str,
    client_token: &str,
    flow_override: Option<String>,
    mut fill_creds: impl FnMut(&mut ProtoWriter, Option<&[u8]>, &[Vec<u8>], &str),
) -> std::result::Result<LoginOk, (bool, anyhow::Error)> {
    let flow = flow_override.unwrap_or_else(random_uuid_v4);
    let mut ctx: Option<Vec<u8>> = None;
    let mut sols: Vec<Vec<u8>> = Vec::new();

    for round in 0..MAX_CHALLENGE_ROUNDS {
        let payload = encode(|w| fill_creds(w, ctx.as_deref(), &sols, &flow));
        log::info!("spotify login5 request round={round} url={url}");
        let (status, raw) = post_login5(http, url, client_token, &payload)
            .await
            .map_err(|e| (false, e))?;
        log::info!("spotify login5 response round={round} status={status} bytes={}", raw.len());
        if status.as_u16() == 400 {
            return Err((true, anyhow!("Spotify Login5 HTTP {status}")));
        }
        if !status.is_success() {
            return Err((false, anyhow!("Spotify Login5 HTTP {status}")));
        }
        let resp = ProtoReader::parse(&raw).map_err(|e| (false, e))?;
        if let Some(ok) = parse_login5_ok(&resp).map_err(|e| (false, e))? {
            log::info!("spotify login5 signed in user={}", ok.username);
            return Ok(ok);
        }
        if let Some(err_code) = resp.number(2) {
            log::warn!("spotify login5 password error={err_code} round={round} url={url}");
            return Err((err_code == 3, anyhow!("{}", describe_login5_error(err_code))));
        }
        let next_ctx = resp.bytes(5).unwrap_or(&[]).to_vec();
        let Some(challenges_msg) = resp.message(3) else {
            let _ = device_id;
            return Err((false, anyhow!("Unexpected Spotify Login5 response")));
        };
        let mut next_sols = Vec::new();
        for ch in challenges_msg.messages(1) {
            if let Some(hc) = ch.message(1) {
                let prefix = hc.bytes(1).unwrap_or(&[]).to_vec();
                let bits = hc.number(2).unwrap_or(10).max(0) as u32;
                log::info!("spotify login5 hashcash bits={bits} round={round}");
                let (suffix, dur) = solve_hashcash_async(next_ctx.clone(), prefix, bits)
                    .await
                    .map_err(|e| (false, e))?;
                log::info!("spotify login5 solved hashcash in {:?}", dur);
                next_sols.push(encode_hashcash_solution(&suffix, dur));
            }
        }
        if next_sols.is_empty() {
            log::warn!("spotify login5 unsupported challenge at round={round} url={url}");
            return Err((false, anyhow!("Spotify requested an interactive or email check; try Email code sign-in instead.")));
        }
        ctx = Some(next_ctx);
        sols = next_sols;
    }
    Err((false, anyhow!("Spotify Login5 exceeded challenge rounds")))
}

fn encode_login5_base(
    w: &mut ProtoWriter,
    device_id: &str,
    context: Option<&[u8]>,
    solutions: &[Vec<u8>],
    flow_id: Option<&str>,
) {
    w.write_msg(1, |ci| {
        ci.write_str(1, CLIENT_ID);
        ci.write_str(2, device_id);
    });
    if let Some(ctx) = context.filter(|c| !c.is_empty()) {
        w.write_bytes(2, ctx);
    }
    if !solutions.is_empty() {
        w.write_msg(3, |cs| {
            for sol in solutions {
                cs.write_bytes(1, sol);
            }
        });
    }
    if let Some(flow) = flow_id.filter(|f| !f.is_empty()) {
        w.write_str(6, flow);
    }
}

fn encode_hashcash_solution(suffix: &[u8], dur: Duration) -> Vec<u8> {
    encode(|w| {
        w.write_msg(1, |hc| {
            hc.write_bytes(1, suffix);
            hc.write_msg(2, |d| {
                d.write_int64(1, dur.as_secs() as i64);
                d.write_int32(2, dur.subsec_nanos() as i32);
            });
        });
    })
}

fn encode_code_solution(code: &str) -> Vec<u8> {
    encode(|w| {
        w.write_msg(2, |cs| {
            cs.write_str(1, code);
        });
    })
}

fn parse_login5_ok(resp: &ProtoReader) -> Result<Option<LoginOk>> {
    let Some(ok) = resp.message(1) else {
        return Ok(None);
    };
    let username = ok.text(1).unwrap_or_default();
    let access_token = ok
        .text(2)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("Spotify Login5 returned an empty access token"))?;
    let stored_cred = ok.bytes(3).unwrap_or(&[]).to_vec();
    let expires_secs = ok.number(4).unwrap_or(3600);
    Ok(Some(LoginOk {
        username,
        access_token,
        stored_cred,
        expires_secs,
    }))
}

async fn post_login5(
    http: &Client,
    url: &str,
    client_token: &str,
    payload: &[u8],
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    let resp = http
        .post(url)
        .timeout(Duration::from_secs(15))
        .header("Accept", "application/x-protobuf")
        .header("Content-Type", "application/x-protobuf")
        .header("client-token", client_token)
        .header("User-Agent", USER_AGENT)
        .body(payload.to_vec())
        .send()
        .await
        .context("Spotify Login5 request failed")?;
    let status = resp.status();
    let bytes = resp.bytes().await?.to_vec();
    Ok((status, bytes))
}

fn describe_login5_error(code: i32) -> &'static str {
    match code {
        1 => "Invalid Spotify credentials (or try Email code if this account uses passwordless/Google login)",
        2 => "Spotify could not complete Login5 authentication (unsupported method)",
        3 => "Spotify rejected the login protocol version",
        4 => "Spotify account requires verification or is unavailable",
        5 => "Spotify does not recognize that email or username",
        6 | 8 => "Too many Spotify login attempts — wait a minute and try again",
        _ => "Spotify Login5 rejected the sign-in request",
    }
}

pub fn random_device_id() -> String {
    let entropy = random_16_bytes();
    hex_lower(&entropy)
}

pub fn random_uuid_v4() -> String {
    let mut b = random_16_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    )
}

fn random_16_bytes() -> [u8; 16] {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha1::new();
    hasher.update(nanos.to_le_bytes());
    hasher.update(pid.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashcash_meets_bit_target() {
        let ctx = b"dawnfm-hashcash-ctx";
        let prefix = b"prefix-bytes";
        let bits = 10;
        let (suffix, _dur) = solve_hashcash(ctx, prefix, bits);
        assert_eq!(suffix.len(), 16);
        let mut combined = Vec::with_capacity(prefix.len() + 16);
        combined.extend_from_slice(prefix);
        combined.extend_from_slice(&suffix);
        let digest = Sha1::digest(&combined);
        let tail = i64::from_be_bytes(digest[12..20].try_into().unwrap());
        assert!(tail.trailing_zeros() >= bits);
    }
}
