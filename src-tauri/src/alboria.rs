//! The player's Alboria account, from the launcher. Alboria is our online ecosystem; its protocol is
//! in `alboria-notes:azura/alboria-relay.md` (sections 3 and 4.6).
//!
//! An account is the player's email, with no password: a 6-digit code sent to it proves it is
//! theirs. This installation is a device of the account: the launcher links it the first time, from
//! the welcome screen, and the game then signs in with the same key by itself.
//!
//! - The key is the Ed25519 seed in `identity.toml`, in the saves folder, which the game also reads
//!   (the SDK's `player_identity.h`). The launcher makes the file, in the game's format, when the
//!   game has not run yet. The seed never leaves this computer and is never logged: only the public
//!   key and signatures are sent.
//! - The computer's own ID (macOS `IOPlatformUUID`, Linux `/etc/machine-id`, Windows `MachineGuid`)
//!   is sent only as a hash (`machine_hash`), so that a banned player cannot come back with a new
//!   account from the same computer (section 3.2).
//!
//! Requests go to `https://api.alboria.org` (or `ALBORIA_API_URL`, for a development server) over
//! HTTPS with rustls and ring, the same stack the updater uses.

use std::fs;
use std::io;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The service's address.
pub const DEFAULT_API_URL: &str = "https://api.alboria.org";
/// Gears of War 3's title ID: the game a device of this launcher plays.
pub const TITLE_ID: &str = "4D5308AB";
/// The identity's file name, in the saves folder (the SDK's `kPlayerIdentityFileName`).
const IDENTITY_FILE: &str = "identity.toml";
/// What a signature covers before the challenge (section 4.1).
const AUTH_CONTEXT: &[u8] = b"alboria-auth-v1";
/// What the computer's ID is hashed with (section 3.2).
const MACHINE_CONTEXT: &[u8] = b"alboria-machine-v1";
/// How long a request may take before the service counts as unreachable.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// How long the events channel may stay silent: the service sends a ping every 25 seconds, so a
/// longer silence means the connection is gone.
const EVENTS_SILENCE: Duration = Duration::from_secs(60);

// --- identity.toml ---------------------------------------------------------------------------

/// The installation's identity, as the game keeps it in `identity.toml`. No `Debug`: the seed must
/// never reach a log.
pub struct Identity {
    xuid: u64,
    machine_id: [u8; 6],
    key_seed: [u8; 32],
}

/// The offline XUID prefix and the bits of player 1's XUID that are random (`player_identity.cpp`).
const OFFLINE_XUID_PREFIX: u64 = 0xE000_0000_0000_0000;
const OFFLINE_XUID_PREFIX_MASK: u64 = 0xFFFF_0000_0000_0000;
const XUID_RANDOM_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
/// Players 2 to 4 are player 1's XUID plus 1 to 3: the random part leaves room for them.
const LOCAL_PLAYERS_AFTER_FIRST: u64 = 3;

fn valid_xuid(xuid: u64) -> bool {
    xuid & OFFLINE_XUID_PREFIX_MASK == OFFLINE_XUID_PREFIX
        && (xuid & XUID_RANDOM_MASK) + LOCAL_PLAYERS_AFTER_FIRST <= XUID_RANDOM_MASK
}

/// A locally administered unicast MAC address, as the game's machine IDs are.
fn valid_machine_id(machine_id: &[u8; 6]) -> bool {
    machine_id[0] & 0x03 == 0x02
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; N];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

#[derive(Deserialize)]
struct IdentityFile {
    xuid: String,
    machine_id: String,
    key_seed: String,
}

/// The identity in `identity.toml`'s `text`, or `None` when it is not a valid one (the SDK's
/// `ParsePlayerIdentity`).
fn parse_identity(text: &str) -> Option<Identity> {
    let file: IdentityFile = toml::from_str(text).ok()?;
    let xuid = u64::from_str_radix(&file.xuid, 16).ok().filter(|_| file.xuid.len() == 16)?;
    let machine_id = from_hex::<6>(&file.machine_id)?;
    let key_seed = from_hex::<32>(&file.key_seed)?;
    (valid_xuid(xuid) && valid_machine_id(&machine_id)).then_some(Identity { xuid, machine_id, key_seed })
}

/// `identity.toml`'s text for `identity`, as the game writes it (`SerializePlayerIdentity`).
fn serialize_identity(identity: &Identity) -> String {
    format!(
        "# This installation's identity for playing with other people. Keep it with the saves;\n\
         # deleting it gives the next start a new one.\n\
         xuid = \"{:016X}\"\n\
         machine_id = \"{}\"\n\
         key_seed = \"{}\"\n",
        identity.xuid,
        to_hex(&identity.machine_id),
        to_hex(&identity.key_seed)
    )
}

/// A new identity, as the game makes one (`GeneratePlayerIdentity`).
fn generate_identity() -> Result<Identity, String> {
    let random = SystemRandom::new();
    let fill = |bytes: &mut [u8]| random.fill(bytes).map_err(|_| "No random numbers from the system".to_string());
    let mut xuid;
    loop {
        let mut bits = [0u8; 8];
        fill(&mut bits)?;
        xuid = OFFLINE_XUID_PREFIX | (u64::from_be_bytes(bits) & XUID_RANDOM_MASK);
        if valid_xuid(xuid) {
            break;
        }
    }
    let mut machine_id = [0u8; 6];
    fill(&mut machine_id)?;
    machine_id[0] = (machine_id[0] & 0xFC) | 0x02;
    let mut key_seed = [0u8; 32];
    fill(&mut key_seed)?;
    Ok(Identity { xuid, machine_id, key_seed })
}

/// Writes `identity` to `path` through a temporary file, readable by the player only.
fn write_identity(path: &Path, identity: &Identity) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, serialize_identity(identity))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)
}

/// The identity kept in `saves_folder`, made and saved there when the game has not made one yet.
/// A file that is not a valid identity is kept aside as `identity.toml.bad` and replaced, as the
/// game does (`LoadOrCreatePlayerIdentity`).
pub fn load_or_create_identity(saves_folder: &Path) -> Result<Identity, String> {
    let path = saves_folder.join(IDENTITY_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => {
            if let Some(identity) = parse_identity(&text) {
                return Ok(identity);
            }
            let _ = fs::rename(&path, saves_folder.join(format!("{IDENTITY_FILE}.bad")));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Cannot read {}: {error}", path.display())),
    }
    let identity = generate_identity()?;
    write_identity(&path, &identity).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(identity)
}

impl Identity {
    fn key_pair(&self) -> Result<Ed25519KeyPair, String> {
        Ed25519KeyPair::from_seed_unchecked(&self.key_seed).map_err(|_| "The identity's key is not valid".to_string())
    }

    /// The public key, in base64, as the service knows the device.
    pub fn public_key(&self) -> Result<String, String> {
        Ok(BASE64.encode(self.key_pair()?.public_key().as_ref()))
    }

    /// The signature of `challenge` that proves this installation holds its key (section 4.1).
    fn sign_challenge(&self, challenge: &[u8]) -> Result<String, String> {
        let message = [AUTH_CONTEXT, challenge].concat();
        Ok(BASE64.encode(self.key_pair()?.sign(&message).as_ref()))
    }
}

// --- The computer's ID -----------------------------------------------------------------------

/// The ID the operating system keeps for this computer, or `None` when it cannot be read.
#[cfg(target_os = "macos")]
fn raw_machine_id() -> Option<String> {
    // The IOPlatformExpertDevice's IOPlatformUUID, as `ioreg` prints it: `"IOPlatformUUID" = "…"`.
    let output = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().find(|line| line.contains("\"IOPlatformUUID\""))?;
    let value = line.rsplit('=').next()?.trim().trim_matches('"');
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(target_os = "linux")]
fn raw_machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[cfg(target_os = "windows")]
fn raw_machine_id() -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
    };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let key = wide("SOFTWARE\\Microsoft\\Cryptography");
    let name = wide("MachineGuid");
    let mut buffer = [0u16; 128];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // The 64-bit view of the registry, where Windows keeps it, whatever this process is.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let length = (size as usize / 2).saturating_sub(1);
    let value = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
    (!value.trim().is_empty()).then(|| value.trim().to_string())
}

/// The hash of `raw`, a computer's ID, as the service receives it (section 3.2).
fn hash_machine_id(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MACHINE_CONTEXT);
    hasher.update([0u8]);
    hasher.update(raw.trim().to_lowercase().as_bytes());
    BASE64.encode(hasher.finalize())
}

/// The hash of this computer's ID. When the system does not give one (a Linux without
/// `machine-id`), the identity's own machine ID stands in: the service still gets a value, which
/// only survives as long as `identity.toml` does.
fn machine_hash(identity: &Identity) -> String {
    let raw = raw_machine_id().unwrap_or_else(|| format!("identity:{}", to_hex(&identity.machine_id)));
    hash_machine_id(&raw)
}

/// What the service calls this system.
fn platform_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "Mac"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    }
}

// --- Requests --------------------------------------------------------------------------------

/// A request the service refused, or one that did not reach it (`status` 0, `error` "network").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlboriaError {
    pub status: u16,
    pub error: String,
    pub message: String,
    /// Seconds to wait before asking for another code (`429 too_many_requests`).
    pub retry_in: Option<u64>,
    /// Guesses left for a code (`401 bad_code`).
    pub attempts_left: Option<u64>,
}

impl AlboriaError {
    fn local(error: &str, message: impl Into<String>) -> Self {
        Self { status: 0, error: error.into(), message: message.into(), retry_in: None, attempts_left: None }
    }

    /// A failure on this computer, before any request: `error` says where ("identity", "settings").
    pub fn local_error(error: &str, message: impl Into<String>) -> Self {
        Self::local(error, message)
    }
}

/// Who the player is once signed in (`gamertag`, `tag`), and whether the account's email is
/// verified: an account made while the service sent no codes is not, and must be by `verify_by`
/// (an ISO time) once codes are back, or it is deleted.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub gamertag: String,
    pub tag: u64,
    pub email_verified: bool,
    pub verify_by: Option<String>,
}

/// A code on its way (`/v1/account/code`); or, while the service sends no codes, that none is
/// needed (`code_needed` false): the request goes on without one.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSent {
    pub expires_in: u64,
    pub resend_in: u64,
    pub code_needed: bool,
    /// The code itself, which only a development server sends (its ALBORIA_DEV_CODES), for the
    /// launcher to fill in; never from the published service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dev_code: Option<String>,
}

/// What the service says about itself (`/v1/status`, alboria-notes:azura/alboria-relay.md section
/// 4.1): whether it sends codes now, its maintenance and notice for everyone, the asking game's own
/// state, and the scheduled actions to warn about. The interface words and shows it
/// (alboriaService.js).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub email_codes: bool,
    pub maintenance: bool,
    /// When the service's maintenance ends by itself, ISO 8601; None while it lasts until turned off.
    pub maintenance_until: Option<String>,
    /// The notice for every player, as HTML the interface rebuilds from an allowlist; None for none.
    pub motd: Option<String>,
    /// The asking game's state, when the request named a game the service knows.
    pub game: Option<GameStatus>,
    pub warnings: Vec<Warning>,
}

/// A game's own state on the service.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameStatus {
    /// Whether its online play is on; off, its rooms are closed.
    pub online: bool,
    pub maintenance: bool,
    pub maintenance_until: Option<String>,
    pub motd: Option<String>,
    /// The oldest launcher that may play it online, such as "0.6.0".
    pub min_launcher: Option<String>,
}

/// A scheduled action about to happen: `maint_on`, `restart`, `game_off`, `close_rooms` and the
/// like, for the whole service (`all`) or the asking game (`game`), at `at` (ISO 8601).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub action: String,
    pub scope: String,
    pub at: String,
}

/// The status from `/v1/status`'s answer. A server from before codes could be off always sends
/// them, and one from before the console has no maintenance, notices or warnings.
fn status_from(answer: &Value) -> Status {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    let flag = |value: &Value, key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    Status {
        email_codes: answer.get("email_codes").and_then(Value::as_bool).unwrap_or(true),
        maintenance: flag(answer, "maintenance"),
        maintenance_until: text(answer, "maintenance_until"),
        motd: text(answer, "motd"),
        game: answer.get("game").filter(|game| game.is_object()).map(|game| GameStatus {
            online: game.get("online").and_then(Value::as_bool).unwrap_or(true),
            maintenance: flag(game, "maintenance"),
            maintenance_until: text(game, "maintenance_until"),
            motd: text(game, "motd"),
            min_launcher: text(game, "min_launcher"),
        }),
        warnings: answer
            .get("warnings")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|warning| {
                        Some(Warning { action: text(warning, "action")?, scope: text(warning, "scope")?, at: text(warning, "at")? })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The connection to the service, and the token of this device while it is signed in.
pub struct Alboria {
    http: Option<reqwest::Client>,
    /// For the events channel, which stays open: no overall time limit, only for silence.
    events_http: Option<reqwest::Client>,
    /// The service's address: production, or for the development launcher the server it chose.
    base: Mutex<String>,
    client: String,
    token: Mutex<Option<String>>,
}

impl Default for Alboria {
    fn default() -> Self {
        // The updater installs the same provider; whichever comes first wins, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build().ok();
        let events_http =
            reqwest::Client::builder().connect_timeout(REQUEST_TIMEOUT).read_timeout(EVENTS_SILENCE).build().ok();
        Self {
            http,
            events_http,
            base: Mutex::new(std::env::var("ALBORIA_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.into())),
            client: format!("azura-launcher/{}", env!("CARGO_PKG_VERSION")),
            token: Mutex::new(None),
        }
    }
}

impl Alboria {
    /// POSTs `body` to `path`, with the token when `signed_in`; the answer's JSON, or why it failed.
    async fn post(&self, path: &str, body: Value, token: Option<String>) -> Result<Value, AlboriaError> {
        let http = self.http.as_ref().ok_or_else(|| AlboriaError::local("network", "No HTTPS client"))?;
        let mut request = http.post(format!("{}{path}", self.base())).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| AlboriaError::local("network", e.to_string()))?;
        let status = response.status().as_u16();
        let json: Value = response.json().await.unwrap_or(Value::Null);
        if status == 200 {
            return Ok(json);
        }
        let text = |key: &str| json.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        Err(AlboriaError {
            status,
            error: if json.get("error").is_some() { text("error") } else { "server".into() },
            message: text("message"),
            retry_in: json.get("retry_in").and_then(Value::as_u64),
            attempts_left: json.get("attempts_left").and_then(Value::as_u64),
        })
    }

    /// The fields that prove this installation holds its key: a fresh challenge, signed.
    async fn proof(&self, identity: &Identity) -> Result<serde_json::Map<String, Value>, AlboriaError> {
        let public_key = identity.public_key().map_err(|e| AlboriaError::local("identity", e))?;
        let answer = self.post("/v1/auth/challenge", json!({ "public_key": public_key }), None).await?;
        let challenge_text = answer.get("challenge").and_then(Value::as_str).unwrap_or_default();
        let challenge = BASE64
            .decode(challenge_text)
            .ok()
            .filter(|bytes| bytes.len() == 32)
            .ok_or_else(|| AlboriaError::local("server", "The challenge is not 32 bytes"))?;
        let signature = identity.sign_challenge(&challenge).map_err(|e| AlboriaError::local("identity", e))?;
        let mut fields = serde_json::Map::new();
        fields.insert("public_key".into(), json!(public_key));
        fields.insert("challenge".into(), json!(challenge_text));
        fields.insert("signature".into(), json!(signature));
        fields.insert("client".into(), json!(self.client));
        Ok(fields)
    }

    /// Keeps the token of an answer that signed in, and gives who the player is.
    fn keep_session(&self, answer: &Value) -> Session {
        if let Some(token) = answer.get("token").and_then(Value::as_str) {
            *self.token.lock().unwrap() = Some(token.to_string());
        }
        session_from(answer)
    }

    /// Whether the service is up and how it is, for the game `title_id` when given (its own
    /// maintenance, notice and minimum launcher come only then).
    pub async fn status(&self, title_id: Option<&str>) -> Result<Status, AlboriaError> {
        let body = title_id.map_or_else(|| json!({}), |title_id| json!({ "title_id": title_id }));
        Ok(status_from(&self.post("/v1/status", body, None).await?))
    }

    /// Signs this device in with `gamertag`, which the account takes on (keeping its tag when it
    /// can). `403 no_account` when no account has linked it yet.
    pub async fn sign_in(&self, identity: &Identity, gamertag: &str) -> Result<Session, AlboriaError> {
        let mut body = self.proof(identity).await?;
        body.insert("gamertag".into(), json!(gamertag));
        let answer = self.post("/v1/auth/session", Value::Object(body), None).await?;
        Ok(self.keep_session(&answer))
    }

    /// Asks for a code for `purpose` ("create", with the gamertag, or "signin") sent to `email`.
    pub async fn request_code(
        &self,
        identity: &Identity,
        purpose: &str,
        email: &str,
        gamertag: Option<&str>,
        language: &str,
    ) -> Result<CodeSent, AlboriaError> {
        let mut body = json!({
            "purpose": purpose,
            "email": email,
            "language": language,
            "machine_id": machine_hash(identity),
        });
        if let Some(gamertag) = gamertag {
            body["gamertag"] = json!(gamertag);
        }
        let answer = self.post("/v1/account/code", body, None).await?;
        Ok(code_sent(&answer))
    }

    /// Uses `code` to create the account (with `gamertag`) or link this device to it, and signs in.
    /// While the service sends no codes, an account is created with no code (`None`), unverified.
    pub async fn verify(
        &self,
        identity: &Identity,
        purpose: &str,
        email: &str,
        code: Option<&str>,
        gamertag: Option<&str>,
        language: &str,
    ) -> Result<Session, AlboriaError> {
        let mut body = self.proof(identity).await?;
        body.insert("purpose".into(), json!(purpose));
        body.insert("email".into(), json!(email));
        if let Some(code) = code {
            body.insert("code".into(), json!(code));
        }
        if let Some(gamertag) = gamertag {
            body.insert("gamertag".into(), json!(gamertag));
        }
        body.insert("language".into(), json!(language));
        body.insert("title_id".into(), json!(TITLE_ID));
        body.insert("platform".into(), json!(platform_name()));
        body.insert("machine_id".into(), json!(machine_hash(identity)));
        let answer = self.post("/v1/account/verify", Value::Object(body), None).await?;
        Ok(self.keep_session(&answer))
    }

    /// A request of the signed-in account. Without a token, or with an expired one, the device
    /// signs in first with `gamertag`, and the request is sent once more.
    pub async fn account_request(
        &self,
        identity: &Identity,
        gamertag: &str,
        path: &str,
        body: Value,
    ) -> Result<Value, AlboriaError> {
        let token = self.token.lock().unwrap().clone();
        if let Some(token) = token {
            match self.post(path, body.clone(), Some(token)).await {
                Err(error) if error.status == 401 => {}
                result => return result,
            }
        }
        self.sign_in(identity, gamertag).await?;
        let token = self.token.lock().unwrap().clone();
        self.post(path, body, token).await
    }

    /// Opens the events channel (`GET /v1/events`, section 4.7), signing in first when there is no
    /// token or it ran out. The answer's body is the stream of events (read it with `EventStream`).
    pub async fn open_events(&self, identity: &Identity, gamertag: &str) -> Result<reqwest::Response, AlboriaError> {
        let http = self.events_http.as_ref().ok_or_else(|| AlboriaError::local("network", "No HTTPS client"))?;
        for attempt in 0..2 {
            if attempt == 1 || self.token.lock().unwrap().is_none() {
                self.sign_in(identity, gamertag).await?;
            }
            let token = self.token.lock().unwrap().clone().unwrap_or_default();
            let response = http
                .get(format!("{}/v1/events", self.base()))
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| AlboriaError::local("network", e.to_string()))?;
            match response.status().as_u16() {
                200 => return Ok(response),
                401 if attempt == 0 => continue,
                status => {
                    return Err(AlboriaError {
                        status,
                        error: "events".into(),
                        message: response.text().await.unwrap_or_default(),
                        retry_in: None,
                        attempts_left: None,
                    })
                }
            }
        }
        Err(AlboriaError::local("events", "The events channel did not open"))
    }

    /// The service's address, such as `https://api.alboria.org`.
    pub fn base(&self) -> String {
        self.base.lock().unwrap().clone()
    }

    /// Talks to the service at `base` from now on, signed out: each server has its own accounts.
    pub fn set_base(&self, base: String) {
        *self.base.lock().unwrap() = base;
        self.forget_token();
    }

    /// Forgets the token: the device signed out, or was removed from the account.
    pub fn forget_token(&self) {
        *self.token.lock().unwrap() = None;
    }
}

/// An event from the events channel: its name and its data, parsed as JSON (Null when it is not).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServiceEvent {
    pub event: String,
    pub data: Value,
}

/// Reads Server-Sent Events from chunks of the channel as they arrive: an event is its `event:`
/// and `data:` lines, ended by a blank line. Comment lines (`: ping`) only keep the channel alive.
#[derive(Default)]
pub struct EventStream {
    buffer: String,
}

impl EventStream {
    /// Adds a chunk and returns the events it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<ServiceEvent> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        // Lines may end in \r\n or \n; one form is enough to find the blank lines between events.
        if self.buffer.contains('\r') {
            self.buffer = self.buffer.replace("\r\n", "\n");
        }
        let mut events = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let block: String = self.buffer.drain(..end + 2).collect();
            let mut name = String::from("message");
            let mut data = Vec::new();
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    name = value.trim().to_string();
                } else if let Some(value) = line.strip_prefix("data:") {
                    data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
                }
            }
            if !data.is_empty() {
                let text = data.join("\n");
                events.push(ServiceEvent { event: name, data: serde_json::from_str(&text).unwrap_or(Value::Null) });
            }
        }
        events
    }
}

fn code_sent(answer: &Value) -> CodeSent {
    CodeSent {
        expires_in: answer.get("expires_in").and_then(Value::as_u64).unwrap_or(600),
        resend_in: answer.get("resend_in").and_then(Value::as_u64).unwrap_or(60),
        code_needed: answer.get("code_needed").and_then(Value::as_bool).unwrap_or(true),
        dev_code: answer
            .get("dev_code")
            .and_then(Value::as_str)
            .filter(|code| code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()))
            .map(str::to_owned),
    }
}

/// Who the player is, from an answer that signed in. A server from before unverified accounts says
/// nothing about the email: it is verified.
fn session_from(answer: &Value) -> Session {
    Session {
        gamertag: answer.get("gamertag").and_then(Value::as_str).unwrap_or_default().to_string(),
        tag: answer.get("tag").and_then(Value::as_u64).unwrap_or_default(),
        email_verified: answer.get("email_verified").and_then(Value::as_bool).unwrap_or(true),
        verify_by: answer.get("verify_by").and_then(Value::as_str).map(str::to_string),
    }
}

/// `code_sent` for the account's own code requests (email and deletion).
pub fn code_sent_from(answer: &Value) -> CodeSent {
    code_sent(answer)
}

/// Where the identity is kept in `saves_folder`.
#[cfg(test)]
fn identity_path(saves_folder: &Path) -> PathBuf {
    saves_folder.join(IDENTITY_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempFolder;

    const SAMPLE: &str = "# This installation's identity for playing with other people. Keep it with the saves;\n\
        # deleting it gives the next start a new one.\n\
        xuid = \"E0007A75AC144039\"\n\
        machine_id = \"e22c2af621f6\"\n\
        key_seed = \"000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\"\n";

    #[test]
    fn reads_events_split_anyhow_across_chunks() {
        let mut stream = EventStream::default();
        assert!(stream.push(b": open\n\n").is_empty());
        assert!(stream.push(b"event: presence\ndata: {\"id\":7,").is_empty());
        let events = stream.push(b"\"presence\":{\"status\":\"online\"}}\n\n: ping\n\nevent: friends\r\ndata: {}\r\n\r\n");
        assert_eq!(
            events,
            vec![
                ServiceEvent { event: "presence".into(), data: json!({ "id": 7, "presence": { "status": "online" } }) },
                ServiceEvent { event: "friends".into(), data: json!({}) },
            ]
        );
        // Data that is not JSON still arrives, as Null.
        assert_eq!(stream.push(b"event: odd\ndata: not json\n\n")[0].data, Value::Null);
    }

    #[test]
    fn reads_the_service_state_notices_and_warnings() {
        let status = status_from(&json!({
            "status": "ok", "email_codes": false, "maintenance": true, "maintenance_until": "2026-11-01T18:00:00.000Z",
            "motd": "<b>Hoy</b> mantenimiento",
            "game": { "online": false, "maintenance": false, "maintenance_until": null, "motd": null, "min_launcher": "0.6.0" },
            "warnings": [{ "action": "restart", "scope": "all", "at": "2026-11-01T17:30:00.000Z" }, { "scope": "game" }]
        }));
        assert!(!status.email_codes && status.maintenance);
        assert_eq!(status.maintenance_until.as_deref(), Some("2026-11-01T18:00:00.000Z"));
        assert_eq!(status.motd.as_deref(), Some("<b>Hoy</b> mantenimiento"));
        let game = status.game.unwrap();
        assert!(!game.online && game.motd.is_none());
        assert_eq!(game.min_launcher.as_deref(), Some("0.6.0"));
        // A warning missing a field is left out.
        assert_eq!(status.warnings.len(), 1);
        assert_eq!((status.warnings[0].action.as_str(), status.warnings[0].scope.as_str()), ("restart", "all"));
    }

    #[test]
    fn a_server_from_before_the_console_has_no_notices() {
        let status = status_from(&json!({ "status": "ok" }));
        assert!(status.email_codes && !status.maintenance && status.motd.is_none() && status.game.is_none());
        assert!(status.warnings.is_empty());
        // An empty notice is no notice.
        assert!(status_from(&json!({ "motd": "" })).motd.is_none());
    }

    #[test]
    fn reads_whether_a_code_is_needed_and_the_email_verified() {
        // While the service sends no codes, it says so, and the account it makes is unverified.
        assert!(!code_sent(&json!({ "code_needed": false })).code_needed);
        assert!(code_sent(&json!({ "expires_in": 600, "resend_in": 60 })).code_needed);
        assert_eq!(code_sent(&json!({ "expires_in": 600 })).dev_code, None);
        assert_eq!(code_sent(&json!({ "dev_code": "012345" })).dev_code.as_deref(), Some("012345"));
        assert_eq!(code_sent(&json!({ "dev_code": "12ab56" })).dev_code, None);
        let unverified = session_from(&json!({
            "token": "t", "gamertag": "Anya", "tag": 1500, "email_verified": false, "verify_by": "2026-11-03T12:00:00.000Z"
        }));
        assert!(!unverified.email_verified);
        assert_eq!(unverified.verify_by.as_deref(), Some("2026-11-03T12:00:00.000Z"));
        // A server from before unverified accounts says nothing: the email is verified.
        let older = session_from(&json!({ "token": "t", "gamertag": "Anya", "tag": 1500 }));
        assert!(older.email_verified);
        assert_eq!(older.verify_by, None);
    }

    #[test]
    fn reads_and_writes_the_games_identity_format() {
        let identity = parse_identity(SAMPLE).expect("a valid identity");
        assert_eq!(identity.xuid, 0xE000_7A75_AC14_4039);
        assert_eq!(serialize_identity(&identity), SAMPLE);
        // An XUID that is not offline, a multicast machine ID or a short seed is not an identity.
        assert!(parse_identity(&SAMPLE.replace("E0007A75AC144039", "0009000000000001")).is_none());
        assert!(parse_identity(&SAMPLE.replace("e22c2af621f6", "e32c2af621f6")).is_none());
        assert!(parse_identity(&SAMPLE.replace("1e1f\"", "\"")).is_none());
    }

    #[test]
    fn makes_an_identity_once_and_keeps_a_broken_one_aside() {
        let folder = TempFolder::new("alboria-identity");
        let first = load_or_create_identity(&folder).unwrap();
        let again = load_or_create_identity(&folder).unwrap();
        assert_eq!(first.public_key().unwrap(), again.public_key().unwrap());
        assert!(valid_xuid(first.xuid) && valid_machine_id(&first.machine_id));

        fs::write(identity_path(&folder), "not an identity").unwrap();
        let replaced = load_or_create_identity(&folder).unwrap();
        assert_ne!(replaced.public_key().unwrap(), first.public_key().unwrap());
        assert_eq!(fs::read_to_string(folder.join("identity.toml.bad")).unwrap(), "not an identity");
    }

    #[test]
    fn signs_as_the_game_does() {
        // The seed 0..31 gives this public key with standard Ed25519 (RFC 8032), as the game's
        // monocypher crypto_ed25519 does.
        let identity = parse_identity(SAMPLE).unwrap();
        assert_eq!(identity.public_key().unwrap(), "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=");
        let signature = BASE64.decode(identity.sign_challenge(&[7u8; 32]).unwrap()).unwrap();
        let public_key = BASE64.decode(identity.public_key().unwrap()).unwrap();
        let message = [AUTH_CONTEXT, &[7u8; 32]].concat();
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
            .verify(&message, &signature)
            .expect("the signature verifies");
    }

    #[test]
    fn hashes_the_computers_id_with_the_alboria_prefix() {
        // SHA-256("alboria-machine-v1" 0x00 "abc-123"), whatever the case or spaces around it.
        assert_eq!(hash_machine_id(" ABC-123\n"), hash_machine_id("abc-123"));
        let mut hasher = Sha256::new();
        hasher.update(b"alboria-machine-v1\0abc-123");
        assert_eq!(hash_machine_id("abc-123"), BASE64.encode(hasher.finalize()));
        // This computer gives one.
        assert!(raw_machine_id().is_some());
    }
}
