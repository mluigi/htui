//! The pinned BGE-small model files (MOD-68, `docs/ANA-23.md` §7.1).
//!
//! [`crate::embed::RtenEmbedder`] loads exactly two files, `model.onnx` and `tokenizer.json`, from
//! `Xenova/bge-small-en-v1.5` at one commit. This module owns that pin and the tokenizer settings
//! that fastembed used to read from the repository's `config.json`, `tokenizer_config.json` and
//! `special_tokens_map.json`, so neither of those files is needed.
//!
//! [`ensure_model`] finds those two files on first use (plan D6): a verified copy in
//! `<cache>/htui/model/`, else a verified copy adopted from fastembed's old cache, else a download
//! from Hugging Face at the pinned commit, hashed as it arrives and renamed into place only on a
//! match. Nothing is fetched at build time.
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use htui_core::store::StoreError;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

/// The Hugging Face repository the model files come from.
pub const REPO: &str = "Xenova/bge-small-en-v1.5";

/// The commit of [`REPO`] every file is fetched at (the one fastembed 3.14.1 resolved).
pub const REVISION: &str = "ea104dacec62c0de699686887e3f920caeb4f3e3";

/// sha256 of `onnx/model.onnx` at [`REVISION`].
pub const ONNX_SHA256: &str = "828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35";

/// Size in bytes of `onnx/model.onnx` at [`REVISION`].
pub const ONNX_BYTES: u64 = 133_093_490;

/// sha256 of `tokenizer.json` at [`REVISION`].
pub const TOKENIZER_SHA256: &str =
    "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";

/// Size in bytes of `tokenizer.json` at [`REVISION`].
pub const TOKENIZER_BYTES: u64 = 711_396;

/// Longest input in tokens, special tokens included; longer texts are truncated. fastembed took
/// `min(512, model_max_length)` from `tokenizer_config.json`, which is 512.
pub const MAX_TOKENS: usize = 512;

/// The `[PAD]` token's id (`config.json` `pad_token_id`): padding positions hold it, with an
/// attention mask of 0.
pub const PAD_ID: u32 = 0;

/// The special tokens of `special_tokens_map.json`, which fastembed re-added to the tokenizer.
pub const SPECIAL_TOKENS: [&str; 5] = ["[CLS]", "[MASK]", "[PAD]", "[SEP]", "[UNK]"];

/// How a text's vector is taken from the model's output: the `[CLS]` token's hidden state.
pub const POOLING: &str = "cls";

/// How that vector is normalised: to unit L2 length.
pub const NORMALISATION: &str = "l2";

/// The two files the embedder loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelFiles {
    /// The ONNX graph and weights (`onnx/model.onnx` in [`REPO`]).
    pub onnx: PathBuf,
    /// The tokenizer (`tokenizer.json` in [`REPO`]).
    pub tokenizer: PathBuf,
}

impl ModelFiles {
    /// `dir/model.onnx` and `dir/tokenizer.json`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            onnx: dir.join("model.onnx"),
            tokenizer: dir.join("tokenizer.json"),
        }
    }
}

/// Where the files are fetched from in production.
const BASE_URL: &str = "https://huggingface.co";

/// The model directory's name under `<cache_root>/model`: repo name plus the commit's first eight.
const MODEL_DIR: &str = "bge-small-en-v1.5-ea104dac";

/// fastembed's snapshots of [`REPO`], under the cache root; each may hold the pinned files.
const FASTEMBED_SNAPSHOTS: &str = "fastembed/models--Xenova--bge-small-en-v1.5/snapshots";

/// A `.part` older than this belongs to a killed run and is removed (A-3). A live download writes
/// at least once per [`READ_TIMEOUT`], so a younger one may be another process's and is left.
const STALE_PART: Duration = Duration::from_secs(10 * 60);

/// Bound on opening the connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Bound on the gap between two body chunks; there is no total timeout, so 133 MB on a slow
/// line is not cut off, while a peer that goes silent fails in a minute.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// `htui/<version>`, so a CDN log names the client.
const USER_AGENT: &str = concat!("htui/", env!("CARGO_PKG_VERSION"));

/// Read and copy buffer size for hashing.
const CHUNK: usize = 64 * 1024;

/// Guards [`install_crypto_provider`].
static PROVIDER: Once = Once::new();

/// Makes every `.part` name unique within the process; the pid makes it unique across processes.
static PART_SEQ: AtomicU64 = AtomicU64::new(0);

/// Lowercase hex, the spelling of the pins (a copy of `htui-agent`'s `install/fetch.rs`).
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The model files, verified: cached, adopted from fastembed's cache, or downloaded (plan D6).
///
/// For each pinned file in `<cache>/htui/model/bge-small-en-v1.5-ea104dac/`: keep it if its sha256
/// is the pin; else copy it from a fastembed snapshot whose copy hashes to the pin; else stream it
/// from `https://huggingface.co/<REPO>/resolve/<REVISION>/<file>`. Hashing and copying run on the
/// blocking pool; only the HTTP body is async. The future is `Send`.
///
/// # Errors
///
/// [`StoreError::Backend`] naming the file and the URL (and both hashes on a mismatch); never
/// panics.
pub async fn ensure_model() -> Result<ModelFiles, StoreError> {
    ModelSource::production()?.ensure().await
}

/// One pinned file: where it lives in [`REPO`], its name in the model directory, and its pin.
#[derive(Debug, Clone)]
pub(crate) struct Pin {
    /// Path inside [`REPO`] at [`REVISION`] (and inside a fastembed snapshot).
    pub(crate) remote: &'static str,
    /// File name in the model directory.
    pub(crate) local: &'static str,
    /// Lowercase hex sha256.
    pub(crate) sha256: String,
    /// Exact size in bytes; a body larger than this is refused before it fills the disk (A-9).
    pub(crate) bytes: u64,
}

/// The production pins: `model.onnx` first, then `tokenizer.json`.
pub(crate) fn pins() -> Vec<Pin> {
    vec![
        Pin {
            remote: "onnx/model.onnx",
            local: "model.onnx",
            sha256: ONNX_SHA256.to_owned(),
            bytes: ONNX_BYTES,
        },
        Pin {
            remote: "tokenizer.json",
            local: "tokenizer.json",
            sha256: TOKENIZER_SHA256.to_owned(),
            bytes: TOKENIZER_BYTES,
        },
    ]
}

/// Where [`ensure_model`] looks and fetches: the seam the tests point at a loopback stub and a
/// temporary cache root.
#[derive(Debug, Clone)]
pub(crate) struct ModelSource {
    base_url: String,
    cache_root: PathBuf,
    pins: Vec<Pin>,
}

impl ModelSource {
    /// Hugging Face, `<user cache>/htui`, [`pins`].
    pub(crate) fn production() -> Result<Self, StoreError> {
        let cache = dirs::cache_dir().ok_or_else(|| {
            StoreError::Backend("embedding model: no user cache directory on this system".into())
        })?;
        Ok(Self::new(BASE_URL, cache.join("htui"), pins()))
    }

    pub(crate) fn new(
        base_url: impl Into<String>,
        cache_root: impl Into<PathBuf>,
        pins: Vec<Pin>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            cache_root: cache_root.into(),
            pins,
        }
    }

    /// `<cache_root>/model/bge-small-en-v1.5-ea104dac`.
    pub(crate) fn model_dir(&self) -> PathBuf {
        self.cache_root.join("model").join(MODEL_DIR)
    }

    /// `<base>/<REPO>/resolve/<REVISION>/<remote>`.
    pub(crate) fn url(&self, pin: &Pin) -> String {
        format!(
            "{}/{REPO}/resolve/{REVISION}/{}",
            self.base_url.trim_end_matches('/'),
            pin.remote
        )
    }

    /// See [`ensure_model`].
    pub(crate) async fn ensure(&self) -> Result<ModelFiles, StoreError> {
        let dir = self.model_dir();
        let missing = {
            let dir = dir.clone();
            let snapshots = self.cache_root.join(FASTEMBED_SNAPSHOTS);
            let pins = self.pins.clone();
            blocking(move || find_local(&dir, &snapshots, pins)).await??
        };
        if !missing.is_empty() {
            // Built only when there is something to download (B10).
            let client = http_client()?;
            for pin in &missing {
                self.download(&client, pin, &dir.join(pin.local)).await?;
            }
        }
        Ok(ModelFiles::in_dir(&dir))
    }

    /// Streams one pinned file into a unique `.part`, hashing as it arrives, and renames it onto
    /// `dest` only when the hash is the pin. Every failure removes the `.part` first.
    async fn download(
        &self,
        client: &reqwest::Client,
        pin: &Pin,
        dest: &Path,
    ) -> Result<(), StoreError> {
        let url = self.url(pin);
        let local = pin.local;
        let transport = |e: &dyn std::fmt::Display| {
            StoreError::Backend(format!(
                "embedding model: cannot download {local} from {url}: {e}"
            ))
        };
        let mut response = client.get(&url).send().await.map_err(|e| transport(&e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(StoreError::Backend(format!(
                "embedding model: cannot download {local}: {url} answered HTTP {}",
                status.as_u16()
            )));
        }
        let part = part_path(dest, pin);
        let mut file = tokio::fs::File::create(&part)
            .await
            .map_err(|e| cannot_write(&part, &e))?;
        let mut hasher = Sha256::new();
        let mut done = 0_u64;
        // Every way out is a value, so the handle is dropped before the `.part` is removed or
        // renamed: Windows refuses both on an open file (H-9).
        let failure = loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    done += chunk.len() as u64;
                    if done > pin.bytes {
                        break Some(StoreError::Backend(format!(
                            "embedding model: {local} from {url} is larger than the expected {} \
                             bytes",
                            pin.bytes
                        )));
                    }
                    hasher.update(&chunk);
                    if let Err(e) = file.write_all(&chunk).await {
                        break Some(cannot_write(&part, &e));
                    }
                }
                Ok(None) => break None,
                Err(e) => break Some(transport(&e)),
            }
        };
        let failure = match failure {
            Some(err) => Some(err),
            None => file.sync_all().await.err().map(|e| cannot_write(&part, &e)),
        };
        drop(file);
        if let Some(err) = failure {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(err);
        }
        let computed = hex(&hasher.finalize());
        if computed != pin.sha256 {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(StoreError::Backend(format!(
                "embedding model: {local} from {url} has sha256 {computed}, expected {}",
                pin.sha256
            )));
        }
        match tokio::fs::rename(&part, dest).await {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = tokio::fs::remove_file(&part).await;
                // Another process may have won the race and holds `dest` open (Windows).
                let (dest_owned, pin_owned) = (dest.to_path_buf(), pin.clone());
                if blocking(move || verify(&dest_owned, &pin_owned)).await? {
                    Ok(())
                } else {
                    Err(cannot_write(dest, &e))
                }
            }
        }
    }
}

/// Runs a file step on the blocking pool.
async fn blocking<T: Send + 'static>(
    step: impl FnOnce() -> T + Send + 'static,
) -> Result<T, StoreError> {
    tokio::task::spawn_blocking(step)
        .await
        .map_err(|e| StoreError::Backend(format!("embedding model: the file check stopped: {e}")))
}

/// Creates the model directory, sweeps stale `.part`s, then keeps or adopts each pin; answers
/// the pins that still need a download, in order.
fn find_local(dir: &Path, snapshots: &Path, pins: Vec<Pin>) -> Result<Vec<Pin>, StoreError> {
    std::fs::create_dir_all(dir).map_err(|e| {
        StoreError::Backend(format!(
            "embedding model: cannot create {}: {e}",
            dir.display()
        ))
    })?;
    sweep_stale_parts(dir, SystemTime::now());
    let mut missing = Vec::new();
    for pin in pins {
        let dest = dir.join(pin.local);
        if verify(&dest, &pin) || adopt(snapshots, &dest, &pin) {
            continue;
        }
        missing.push(pin);
    }
    Ok(missing)
}

/// Whether `path` holds exactly the pinned bytes. A missing file is `false` quietly; a present
/// one that does not match is `false` with a warning.
fn verify(path: &Path, pin: &Pin) -> bool {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return false,
    };
    if size != pin.bytes {
        tracing::warn!(path = %path.display(), size, expected = pin.bytes, "embedding model file has the wrong size; fetching it again");
        return false;
    }
    match hash_file(path) {
        Ok(computed) if computed == pin.sha256 => true,
        Ok(computed) => {
            tracing::warn!(path = %path.display(), %computed, expected = %pin.sha256, "embedding model file has the wrong sha256; fetching it again");
            false
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "cannot read the embedding model file; fetching it again");
            false
        }
    }
}

/// sha256 of a file, read in [`CHUNK`]s.
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Copies `pin` from the first fastembed snapshot whose copy hashes to the pin, through a fresh
/// `.part`, then renames it onto `dest`. The snapshot is only read: its files may be symlinks
/// into fastembed's `blobs/`, and `File::open` follows them.
fn adopt(snapshots: &Path, dest: &Path, pin: &Pin) -> bool {
    for snap in snapshot_dirs(snapshots) {
        let src = snap.join(pin.remote);
        match std::fs::metadata(&src) {
            Ok(meta) if meta.is_file() && meta.len() == pin.bytes => {}
            _ => continue,
        }
        let part = part_path(dest, pin);
        match copy_hashed(&src, &part) {
            Ok(computed) if computed == pin.sha256 => match std::fs::rename(&part, dest) {
                Ok(()) => {
                    tracing::info!(from = %src.display(), to = %dest.display(), "adopted the embedding model file from fastembed's cache");
                    return true;
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    if verify(dest, pin) {
                        return true;
                    }
                    tracing::warn!(path = %dest.display(), error = %e, "cannot place the adopted embedding model file");
                }
            },
            Ok(computed) => {
                let _ = std::fs::remove_file(&part);
                tracing::warn!(path = %src.display(), %computed, expected = %pin.sha256, "fastembed's copy has the wrong sha256; not adopting it");
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                tracing::warn!(path = %src.display(), error = %e, "cannot copy fastembed's model file");
            }
        }
    }
    false
}

/// The snapshot directories: the one named [`REVISION`] first, then the rest sorted.
fn snapshot_dirs(snapshots: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(snapshots) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort_by_key(|path| (path.file_name() != Some(REVISION.as_ref()), path.clone()));
    dirs
}

/// Copies `src` into a new `part`, hashing the bytes as they pass; the handle is synced and
/// dropped before this returns.
fn copy_hashed(src: &Path, part: &Path) -> std::io::Result<String> {
    let mut from = std::fs::File::open(src)?;
    let mut to = std::fs::File::create(part)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; CHUNK];
    loop {
        let n = from.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        to.write_all(&buf[..n])?;
    }
    to.sync_all()?;
    Ok(hex(&hasher.finalize()))
}

/// Removes every `*.part` in `dir` last modified more than [`STALE_PART`] before `now`. Errors are
/// ignored: a sweep that cannot run leaves debris, not a failure.
fn sweep_stale_parts(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "part") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > STALE_PART);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// `<dest dir>/<local>.<pid>.<seq>.part`: unique, so two processes never write one file (A-3).
fn part_path(dest: &Path, pin: &Pin) -> PathBuf {
    dest.with_file_name(format!(
        "{}.{}.{}.part",
        pin.local,
        std::process::id(),
        PART_SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// The one HTTP client of an [`ensure`](ModelSource::ensure) that downloads: two timeouts, no
/// total, after the provider install.
fn http_client() -> Result<reqwest::Client, StoreError> {
    install_crypto_provider();
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|e| {
            StoreError::Backend(format!(
                "embedding model: cannot build the HTTP client: {e}"
            ))
        })
}

/// Installs `ring` as the process's rustls provider once, ignoring "already installed" (mirrors
/// `htui-agent`'s `install/http.rs`). Not strictly needed in every build (H-6: reqwest falls back
/// to aws-lc-rs here, and the `htui` binary's default backend is native-tls), but it keeps one
/// provider process-wide.
fn install_crypto_provider() {
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// E5: a file in the model directory that cannot be written.
fn cannot_write(path: &Path, e: &dyn std::fmt::Display) -> StoreError {
    StoreError::Backend(format!(
        "embedding model: cannot write {}: {e}",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_files_in_a_dir_end_in_onnx_and_json() {
        let files = ModelFiles::in_dir(Path::new("/cache/model"));
        assert_eq!(files.onnx, Path::new("/cache/model/model.onnx"));
        assert_eq!(files.tokenizer, Path::new("/cache/model/tokenizer.json"));
    }

    use std::collections::HashMap;
    use std::io::{BufRead as _, BufReader};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime};

    /// What the stub answers on one path.
    #[derive(Debug, Clone)]
    enum Answer {
        Body(u16, Vec<u8>),
        Redirect(u16, &'static str),
    }

    /// A loopback HTTP/1.1 responder on a std thread: scripted routes, every request path
    /// recorded. No new dev-dependency; the test runtime never drives it.
    #[derive(Debug)]
    struct Stub {
        addr: SocketAddr,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl Stub {
        fn start(routes: Vec<(String, Answer)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
            let addr = listener.local_addr().expect("the stub's address");
            let seen = Arc::new(Mutex::new(Vec::new()));
            let routes: Arc<HashMap<String, Answer>> = Arc::new(routes.into_iter().collect());
            let record = Arc::clone(&seen);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    serve(&stream, &routes, &record);
                }
            });
            Self { addr, seen }
        }

        fn base(&self) -> String {
            format!("http://{}", self.addr)
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().expect("the stub's log").clone()
        }
    }

    /// One request on one connection, answered with `Connection: close`.
    fn serve(stream: &TcpStream, routes: &HashMap<String, Answer>, seen: &Mutex<Vec<String>>) {
        let mut reader = BufReader::new(stream);
        let mut path = None;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if path.is_none() {
                path = line
                    .strip_prefix("GET ")
                    .and_then(|rest| rest.split(' ').next())
                    .map(str::to_owned);
            }
        }
        let Some(path) = path else { return };
        seen.lock().expect("the stub's log").push(path.clone());
        let (code, location, body) = match routes.get(&path) {
            Some(Answer::Body(code, body)) => (*code, None, body.clone()),
            Some(Answer::Redirect(code, to)) => (*code, Some(*to), Vec::new()),
            None => (404, None, Vec::new()),
        };
        let mut head = format!(
            "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        if let Some(to) = location {
            head.push_str(&format!("Location: {to}\r\n"));
        }
        head.push_str("\r\n");
        let mut out = stream;
        let _ = out.write_all(head.as_bytes());
        let _ = out.write_all(&body);
        let _ = out.flush();
    }

    fn sha(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    /// The production names, with the hashes and sizes of small fake bodies.
    fn fake_pins(onnx: &[u8], tok: &[u8]) -> Vec<Pin> {
        let mut pins = pins();
        pins[0].sha256 = sha(onnx);
        pins[0].bytes = onnx.len() as u64;
        pins[1].sha256 = sha(tok);
        pins[1].bytes = tok.len() as u64;
        pins
    }

    fn source(stub: &Stub, root: &Path, pins: Vec<Pin>) -> ModelSource {
        ModelSource::new(stub.base(), root, pins)
    }

    /// The request path of a production file.
    fn path_of(remote: &str) -> String {
        format!("/Xenova/bge-small-en-v1.5/resolve/{REVISION}/{remote}")
    }

    const ONNX: &[u8] = b"fake onnx bytes";
    const TOK: &[u8] = b"{\"fake\": \"tokenizer\"}";

    fn both_routes() -> Vec<(String, Answer)> {
        vec![
            (path_of("onnx/model.onnx"), Answer::Body(200, ONNX.to_vec())),
            (path_of("tokenizer.json"), Answer::Body(200, TOK.to_vec())),
        ]
    }

    fn model_dir(root: &Path) -> PathBuf {
        root.join("model").join("bge-small-en-v1.5-ea104dac")
    }

    fn parts_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.ends_with(".part"))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn err_text(result: Result<ModelFiles, StoreError>) -> String {
        match result {
            Ok(files) => panic!("expected an error, got {files:?}"),
            Err(err) => err.to_string(),
        }
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, bytes).expect("write");
    }

    #[tokio::test]
    async fn downloads_both_files_into_the_model_dir() {
        let root = tempfile::tempdir().expect("tempdir");
        let stub = Stub::start(both_routes());
        let files = source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("downloads");
        assert_eq!(files, ModelFiles::in_dir(&model_dir(root.path())));
        assert_eq!(std::fs::read(&files.onnx).expect("onnx"), ONNX);
        assert_eq!(std::fs::read(&files.tokenizer).expect("tokenizer"), TOK);
        assert_eq!(
            stub.seen(),
            vec![path_of("onnx/model.onnx"), path_of("tokenizer.json")]
        );
        assert!(parts_in(&model_dir(root.path())).is_empty());
    }

    #[tokio::test]
    async fn follows_a_relative_redirect() {
        let root = tempfile::tempdir().expect("tempdir");
        let stub = Stub::start(vec![
            (path_of("onnx/model.onnx"), Answer::Body(200, ONNX.to_vec())),
            (
                path_of("tokenizer.json"),
                Answer::Redirect(307, "/api/resolve-cache/tok"),
            ),
            (
                "/api/resolve-cache/tok".to_owned(),
                Answer::Body(200, TOK.to_vec()),
            ),
        ]);
        let files = source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("follows the redirect");
        assert_eq!(std::fs::read(&files.tokenizer).expect("tokenizer"), TOK);
        assert!(stub.seen().contains(&"/api/resolve-cache/tok".to_owned()));
    }

    #[tokio::test]
    async fn a_hash_mismatch_names_both_hashes_and_leaves_no_file() {
        let root = tempfile::tempdir().expect("tempdir");
        let other = b"other onnx bytes"[..ONNX.len()].to_vec();
        assert_eq!(other.len(), ONNX.len());
        assert_ne!(other, ONNX);
        let stub = Stub::start(vec![(
            path_of("onnx/model.onnx"),
            Answer::Body(200, other.clone()),
        )]);
        let src = source(&stub, root.path(), fake_pins(ONNX, TOK));
        let url = src.url(&src.pins[0]);
        let text = err_text(src.ensure().await);
        assert!(text.contains(&url), "{text}");
        assert!(text.contains(&sha(&other)), "{text}");
        assert!(text.contains(&sha(ONNX)), "{text}");
        assert!(!model_dir(root.path()).join("model.onnx").exists());
        assert!(parts_in(&model_dir(root.path())).is_empty());
    }

    #[tokio::test]
    async fn a_body_longer_than_the_pin_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut longer = ONNX.to_vec();
        longer.push(b'!');
        let stub = Stub::start(vec![(
            path_of("onnx/model.onnx"),
            Answer::Body(200, longer),
        )]);
        let text = err_text(
            source(&stub, root.path(), fake_pins(ONNX, TOK))
                .ensure()
                .await,
        );
        assert!(text.contains("larger than the expected"), "{text}");
        assert!(!model_dir(root.path()).join("model.onnx").exists());
        assert!(parts_in(&model_dir(root.path())).is_empty());
    }

    #[tokio::test]
    async fn http_404_names_the_url() {
        let root = tempfile::tempdir().expect("tempdir");
        let stub = Stub::start(Vec::new());
        let src = source(&stub, root.path(), fake_pins(ONNX, TOK));
        let url = src.url(&src.pins[0]);
        let text = err_text(src.ensure().await);
        assert!(text.contains(&url), "{text}");
        assert!(text.contains("HTTP 404"), "{text}");
    }

    #[tokio::test]
    async fn an_unreachable_host_is_an_error_not_a_panic() {
        let root = tempfile::tempdir().expect("tempdir");
        let addr = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr")
        };
        let src = ModelSource::new(format!("http://{addr}"), root.path(), fake_pins(ONNX, TOK));
        let url = src.url(&src.pins[0]);
        let text = err_text(src.ensure().await);
        assert!(text.contains(&url), "{text}");
    }

    #[tokio::test]
    async fn present_files_make_no_request() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = model_dir(root.path());
        write(&dir.join("model.onnx"), ONNX);
        write(&dir.join("tokenizer.json"), TOK);
        let stub = Stub::start(both_routes());
        let files = source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("present");
        assert_eq!(files, ModelFiles::in_dir(&dir));
        assert!(stub.seen().is_empty(), "{:?}", stub.seen());
    }

    #[tokio::test]
    async fn a_corrupt_present_file_is_replaced() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = model_dir(root.path());
        write(&dir.join("model.onnx"), b"corrupt");
        write(&dir.join("tokenizer.json"), TOK);
        let stub = Stub::start(both_routes());
        source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("replaced");
        assert_eq!(std::fs::read(dir.join("model.onnx")).expect("onnx"), ONNX);
        assert_eq!(stub.seen(), vec![path_of("onnx/model.onnx")]);
    }

    fn snapshot(root: &Path) -> PathBuf {
        root.join("fastembed/models--Xenova--bge-small-en-v1.5/snapshots")
            .join(REVISION)
    }

    #[tokio::test]
    async fn a_fastembed_snapshot_is_adopted_without_a_request() {
        let root = tempfile::tempdir().expect("tempdir");
        let snap = snapshot(root.path());
        write(&snap.join("tokenizer.json"), TOK);
        let onnx_link = snap.join("onnx/model.onnx");
        #[cfg(unix)]
        {
            let blob = root
                .path()
                .join("fastembed/models--Xenova--bge-small-en-v1.5/blobs/x");
            write(&blob, ONNX);
            std::fs::create_dir_all(onnx_link.parent().expect("parent")).expect("mkdir");
            std::os::unix::fs::symlink("../../../blobs/x", &onnx_link).expect("symlink");
            assert!(
                std::fs::symlink_metadata(&onnx_link)
                    .expect("link")
                    .file_type()
                    .is_symlink()
            );
        }
        #[cfg(not(unix))]
        write(&onnx_link, ONNX);
        let stub = Stub::start(both_routes());
        let files = source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("adopted");
        assert!(stub.seen().is_empty(), "{:?}", stub.seen());
        for (path, bytes) in [(&files.onnx, ONNX), (&files.tokenizer, TOK)] {
            assert!(
                std::fs::symlink_metadata(path)
                    .expect("dest")
                    .file_type()
                    .is_file()
            );
            assert_eq!(std::fs::read(path).expect("dest"), bytes);
        }
        assert_eq!(std::fs::read(&onnx_link).expect("snapshot onnx"), ONNX);
        assert_eq!(
            std::fs::read(snap.join("tokenizer.json")).expect("snapshot tok"),
            TOK
        );
        assert!(parts_in(&model_dir(root.path())).is_empty());
    }

    #[tokio::test]
    async fn a_snapshot_with_wrong_hashes_is_ignored() {
        let root = tempfile::tempdir().expect("tempdir");
        let snap = snapshot(root.path());
        let wrong_onnx = vec![b'x'; ONNX.len()];
        let wrong_tok = vec![b'y'; TOK.len()];
        write(&snap.join("onnx/model.onnx"), &wrong_onnx);
        write(&snap.join("tokenizer.json"), &wrong_tok);
        let stub = Stub::start(both_routes());
        let files = source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("downloaded");
        assert_eq!(std::fs::read(&files.onnx).expect("onnx"), ONNX);
        assert_eq!(std::fs::read(&files.tokenizer).expect("tok"), TOK);
        assert_eq!(stub.seen().len(), 2);
        assert_eq!(
            std::fs::read(snap.join("onnx/model.onnx")).expect("snap"),
            wrong_onnx
        );
        assert_eq!(
            std::fs::read(snap.join("tokenizer.json")).expect("snap"),
            wrong_tok
        );
        assert!(parts_in(&model_dir(root.path())).is_empty());
    }

    #[tokio::test]
    async fn a_stale_part_is_swept_and_a_fresh_one_left() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = model_dir(root.path());
        let stale = dir.join("model.onnx.1.0.part");
        let fresh = dir.join("tokenizer.json.2.0.part");
        write(&stale, b"half");
        write(&fresh, b"half");
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .expect("open stale")
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .expect("age the stale part");
        let stub = Stub::start(both_routes());
        source(&stub, root.path(), fake_pins(ONNX, TOK))
            .ensure()
            .await
            .expect("ensured");
        assert!(!stale.exists());
        assert!(fresh.exists());
    }

    #[tokio::test]
    async fn an_unwritable_cache_root_is_an_error() {
        let root = tempfile::tempdir().expect("tempdir");
        let file = root.path().join("not-a-dir");
        std::fs::write(&file, b"").expect("write");
        let stub = Stub::start(both_routes());
        let text = err_text(source(&stub, &file, fake_pins(ONNX, TOK)).ensure().await);
        assert!(text.contains("cannot create"), "{text}");
    }

    /// `concepts_worker` boxes the load into a `BoxFuture` and `spawn_index_job` `tokio::spawn`s
    /// it: both need `Send` (H-4).
    #[test]
    fn ensure_model_futures_are_send() {
        fn send<T: Send>(_: T) {}
        send(ensure_model());
        send(ModelSource::new("http://127.0.0.1:1", "/nonexistent", pins()).ensure());
    }

    #[test]
    fn production_source_is_the_pinned_commit() {
        let pins = pins();
        assert_eq!(pins[0].sha256, ONNX_SHA256);
        assert_eq!(pins[0].bytes, ONNX_BYTES);
        assert_eq!(pins[1].sha256, TOKENIZER_SHA256);
        assert_eq!(pins[1].bytes, TOKENIZER_BYTES);
        assert!(REVISION.starts_with(MODEL_DIR.rsplit('-').next().expect("a suffix")));
        let src = ModelSource::new("https://huggingface.co", "/cache/htui", pins.clone());
        assert_eq!(
            src.url(&pins[1]),
            "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/\
             ea104dacec62c0de699686887e3f920caeb4f3e3/tokenizer.json"
        );
        assert_eq!(
            src.model_dir(),
            Path::new("/cache/htui/model/bge-small-en-v1.5-ea104dac")
        );
    }

    /// The only test of TLS and of Hugging Face's real redirects (A-8). 133 MB into a tempdir.
    #[tokio::test]
    #[ignore = "downloads 133 MB from huggingface.co"]
    async fn real_download_from_huggingface() {
        let root = tempfile::tempdir().expect("tempdir");
        let src = ModelSource::new("https://huggingface.co", root.path(), pins());
        let started = std::time::Instant::now();
        let files = src.ensure().await.expect("downloads from huggingface.co");
        let elapsed = started.elapsed();
        for (path, pin) in [(&files.onnx, &pins()[0]), (&files.tokenizer, &pins()[1])] {
            let bytes = std::fs::read(path).expect("downloaded file");
            assert_eq!(bytes.len() as u64, pin.bytes, "{}", path.display());
            assert_eq!(sha(&bytes), pin.sha256, "{}", path.display());
        }
        eprintln!(
            "real_download_from_huggingface: {} + {} bytes in {elapsed:.1?}, both sha256 verified",
            ONNX_BYTES, TOKENIZER_BYTES
        );
    }
}
