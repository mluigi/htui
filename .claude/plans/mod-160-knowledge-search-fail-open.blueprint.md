# Blueprint: MOD-160 — Knowledge search fails open cheaply and says so (T1–T6)

> Produced by `code-architect` 2026-11-05 from the confirmed plan
> `.claude/plans/mod-160-knowledge-search-fail-open.plan.md` (D1–D10, Q1–Q4 answered as
> recommended). Tree read at `f8826cb9` (branch `hr/MOD-160`). The plan stays the spec. §0 lists where
> the tree disagrees with a plan claim or a plan file set. Every row is **adopted**, and none reopens
> a decision. Line numbers are `HEAD` at `f8826cb9`.

## 0. Plan corrections

| # | Finding | Evidence | Resolution |
|---|---|---|---|
| P-1 | **Hidden coupling between T2 and T5 in `concepts.rs`.** `the_index_job_and_the_search_share_one_model_load` (`concepts.rs:1529-1569`) builds a `QdrantIndex` over `http://127.0.0.1:1` and asserts that `index.search(..)` is **still waiting** after 500 ms on the job's pending model load. Under D2 that search asks Qdrant first and fails at once (connection refused), so the test breaks under T2. The test lives in T5's file. | `concepts.rs:1546-1566`; D2 | **adopted**: **T1** rewrites the case to await `index.embedder()` (`concepts_worker.rs:508`, `pub(crate)`) in place of `index.search(..)`. It asserts the same H-26 property (one load slot shared by the job and every search) and passes both before and after T2. T2 and T5 then share no file. |
| P-2 | **The in-process URL-write refresh (D5) cannot live in `qdrant_settings_info.rs`.** `serve`/`serve_with` is the harness's and `tests/qdrant_settings_pg.rs`'s entry (`store_worker.rs:2111-2121`, `testkit.rs:413`). Reaching the process-wide index from there would touch `shared_qdrant()` in tests (MOD-158 blueprint P-13). | `store_worker.rs:2377-2386, 3292-3315`; `concepts_worker.rs:599-620` | **adopted**: the store loop tells its `ConceptsRuntime` after any URL write (§T2.4): `concepts.url_changed()`, from the loop's `other =>` arm. T2's set **drops** `qdrant_settings_info.rs` and **adds** `store_worker.rs`. T4 edits `store_worker.rs` too, and runs after T2 anyway. |
| P-3 | `docs/htui-mcp.md` describes the old wording and the old offer rule (`:124-126` "search unavailable: <cause>", `:143`, `:160-163` "offered by the TUI always … worker only when the keyring held a URL at start", `:637`). The plan does not list it. | doc | **adopted**: in T2's set. |
| P-4 | **T5 needs no `state_cmd.rs` change.** The CLI's sentences are built in `concepts.rs`: `unresolved_failure` (`:582-595`), `search_index` (`:696-727`) and `index_refusal` (`:733-749`). `state_cmd` only carries them through (`Answer::exit` `:479-498`, `search_doc` `:909-952`). Its tests and snapshots build their answers from literals (`search_documents_by_index_state` `:1470-1495`, `htui__state_cmd__tests__search_index_*.snap`). `tests/state_cli_pg.rs:713` (`message.starts_with("HTUI_QDRANT_URL ")`) does break under D8. | `state_cmd.rs:350-498, 909-952` | **adopted**: T5's set is `concepts.rs`, `tests/state_cli_pg.rs` and `docs/htui-cli.md`. |
| P-5 | **T3 touches more key files than the plan lists.** (a) `keys/hint.rs:783-795` pins the whole help box of `CONCEPTS_QUERY`. (b) `keys/catalogue.rs:2942` pins `CATALOGUE.len() == 216`. (c) `:3392-3428` pins the set of `in_capture` acts. (d) `keys/views.rs:1432-1446` pins what `CONCEPTS_QUERY` resolves. (e) `docs/htui-keys.md:160-165` is the `[concepts]` table, pinned byte for byte by `tests/keys_file.rs:723-745`. That doc is the plan's "docs/ key reference". | grep `CONCEPTS_QUERY`, `ConceptsReindex` | **adopted**: all five are in T3's set (§T3.1). No other doc lists the Concepts keys (`git grep "decisions only"` finds only `docs/htui-keys.md`). |
| P-6 | **`ctrl-e` is free in the Concepts stack.** `CONCEPTS_QUERY` (`views.rs:509-524`) stacks `[concepts]`, `FIELD`, `OVERLAY`, `QUIT` and `HELP`. The only catalogue default `ctrl-e` is `Act::FormExternalEditor` in `Context::Form` (`catalogue.rs:1167-1173`), which is not in that stack. `FieldEnd` is `end` only (`:2162`), and no global or view default uses `ctrl-e`. | `git grep '"ctrl-e"'` | **adopted**: the default chord is `ctrl-e`. A `version = 2` keys file that already binds a global act to `ctrl-e` now collides in strict mode, in the same way `validate.rs:690-720` shows for `ctrl-d`/`ctrl-p`; version 1 still loads. Listed as a risk (§9 R-5). |
| P-7 | **`ProbeQdrant` has a third route.** The test harness routes the concepts requests itself (`testkit.rs:401-412`), apart from the loop (`store_worker.rs:3273`) and `try_serve` (`:2403-2411`). The new request must also **stay out of** `qdrant_settings_info::REQUEST_NAMES`: `is_qdrant` (`store_worker.rs:1234`) sends those to `serve_with` on the loop. | `testkit.rs:401` | **adopted**: T4's set adds `testkit.rs`. `ProbeQdrant` joins `concepts_worker::REQUEST_NAMES` (now 3 entries) and its `request_names_match_the_name_arms` test (`concepts_worker.rs:1237`). |
| P-8 | **A deterministic off-loop test for the probe.** The plan's "probe to a blackhole address" depends on the network: in the hr sandbox a non-routable address may answer "network unreachable" at once. The repo's own precedent is a `MemIndex` that waits on a paused clock (`store_worker.rs:3723-3768`). | `store_worker.rs:3729` | **adopted**: the loop test uses `MemIndex::new().delayed(1 s).with_check(..)` under `start_paused` (§T4.6). The real client's bound is proven in `vector.rs` by a probe of `http://127.0.0.1:1` (§T1.4). |
| P-9 | **The probe's timeout.** qdrant-client 1.19.0 defaults both `timeout` and `connect_timeout` to 5 s (`qdrant_client/config.rs:258-259`). A failed connect is not retried (`channel_pool.rs:88-100`: `get_channel` returns `?` before the retry branch). `vector::client` (`vector.rs:993-1002`) sets neither. | registry source | **adopted**: `client` splits into `client_config(settings) -> QdrantConfig` and `.build()`. `probe` sets `.timeout(PROBE_TIMEOUT).connect_timeout(PROBE_TIMEOUT)` with `PROBE_TIMEOUT = 3 s`, the gate step's bound. Searches keep the 5 s defaults. |
| P-10 | **Two test seams the plan's tests need.** (a) `probe(settings, collection)` checks against `model::identity()`, while the live suite's collections record `HashEmbedder`'s identity (`qdrant_live.rs:55-62`). A "ready" live case therefore needs the identity as a parameter. (b) `QdrantIndex` searches the fixed `COLLECTION`. A live "creates no collection" case on a shared Qdrant needs a throwaway name. `with_loader` is `pub(crate)` (`concepts_worker.rs:480`), so that case cannot sit in `tests/qdrant_worker.rs`. | `qdrant_live.rs:44-62`; `concepts_worker.rs:480-499` | **adopted**: `vector::probe_with(settings, collection, mine, dim)`, with `probe` a thin wrapper. `QdrantIndex::with_collection(&str)` is `pub(crate)`. T2's live cases sit in `concepts_worker.rs`'s tests, gated on `HTUI_TEST_QDRANT_URL` with the skip line. `tests/qdrant_worker.rs` is **not** in T2's set. |
| P-11 | **The Settings probe shows up in existing request assertions.** The section sends `ProbeQdrant` after a snapshot reply (§T4.3). Tests that read the requests after a reply without draining therefore see it, for example `a_qdrant_reload_just_before_a_write_is_not_taken_as_its_answer` (`tests/settings.rs:5925-5939`). | `tests/settings.rs:5745-5756, 5769, 5788, 5886, 5935, 6109`; `tests/qdrant_settings.rs:774` | **adopted**: in T4, the `requests_of` helpers in `tests/settings.rs` and `tests/qdrant_settings.rs` drop `StoreRequest::ProbeQdrant`, and dedicated cases assert the probe. |
| P-12 | **What "after the store is first online" means at launch.** `App::start` (`app/state.rs:865-875`) runs while the backend is still `connecting` (`Backend::label`, `htui-store/src/backend.rs:93-105`). A probe sent then would resolve `Offline` and say nothing. | `app/update.rs:497-533` | **adopted**: the launch probe is sent from `observe_reply` on the first `StoreReply::StoreState { label == "online" }`. It is gated on `qdrant_offer_check` (set by `register_all`, `app/mod.rs:156`; MOD-158 P-6), so a harness that never calls `register_all` sends none. |
| P-13 | Minor line drift: the worker's gate is `worker_cmd.rs:116-123` and `worker_search` is `:141-155` (the plan says `:117-133`, `:152-160`). `ExistingCollection::find_in` is `vector.rs:1028-1041`. | source | noted |
| P-14 | `docs/htui-worker.md:245-248` is the `search_concepts` paragraph that T2 rewrites. The worker's start warning goes in its "Logs" / troubleshooting text. The README's Settings table (`README.md:447`) is **not** edited: README belongs to T6, and T4 runs at the same time as T6. | doc | **adopted** |

---

## 1. Shared decisions

- **The sentence** is `htui_core::qdrant::KNOWLEDGE_SEARCH_UNAVAILABLE` = `"knowledge search unavailable"`, and `unavailable(cause)` = `"{SENTENCE}: {cause}"`. A blank cause gives the sentence alone.
- **What counts as unavailable (D1).** An `Err` from a search is "unavailable". An empty collection never is: it answers `Ok(vec![])` (TUI, MCP) or `IndexAnswer::Empty` (CLI).
- **One probe** is `htui_store::vector::probe` (T1). It never loads the model, never creates anything, and is bounded by `PROBE_TIMEOUT = 3 s`.
- **One classification** is `concepts_worker::QdrantCheck` (T2). The worker (T2), Settings and the launch notice (T4) all read it, so the three surfaces say the same states with the same words.
- **The flag (D5)** lives on `QdrantSource` and is read without awaiting. It is refreshed only by spawned tasks or by reads already in flight. `R-NF-3` holds: nothing on the store loop awaits it.
- **P-13 (MOD-158) still holds.** No test builds `QdrantSource::from_process()` or calls `shared_qdrant()` to search, sync, probe or read the flag. A test that needs Qdrant reads `HTUI_TEST_QDRANT_URL`, never `HTUI_QDRANT_*`.
- **No migration, no `.sqlx` entry, no requirement edit.**

---

## T1 — Store and core primitives (D1, D3, D9 probe) · first, alone

### T1.1 `crates/htui-core/src/qdrant.rs` (UPDATE, after `stored_url_unusable`, `:240-247`)

```rust
/// MOD-160 D1: what every knowledge-search surface says when the search cannot answer.
pub const KNOWLEDGE_SEARCH_UNAVAILABLE: &str = "knowledge search unavailable";

/// MOD-160 D1: the sentence, then its cause after a colon (CLI, MCP, logs). A blank cause is the
/// sentence alone.
#[must_use]
pub fn unavailable(cause: &str) -> String;
```

### T1.2 `crates/htui-store/src/vector.rs` (UPDATE)

```rust
/// MOD-160 D9: how long a probe waits for Qdrant to connect and to answer each call.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// `client`'s builder before `build()` (P-9); `client` = `client_config(s).build()`.
fn client_config(settings: &QdrantSettings) -> qdrant_client::config::QdrantConfig;

/// `QdrantStore::existing`'s body, over any client (D3): read the collection's info, then
/// `check_existing`. `QdrantStore::existing` (`:~905`) becomes a one-line call of it.
async fn read_existing(client: &Qdrant, collection: &str, mine: &EmbedderIdentity, dim: usize)
    -> Result<Existing, StoreError>;

impl ExistingCollection {
    /// MOD-160 D3: `open`'s width and embedder check, before any embedder exists. Reads only:
    /// `Matches`, `Stamp` and `Rebuild` all pass, as in `open`.
    ///
    /// # Errors
    /// `embedder mismatch` ([`is_embedder_mismatch`]), [`QDRANT_REFUSED`], or a read failure.
    pub async fn check(&self, mine: &EmbedderIdentity, dim: usize) -> Result<(), StoreError>;
}

/// MOD-160 D9: what a model-free look at Qdrant found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// The collection exists and passes the check.
    Ready,
    /// Qdrant answered and holds no collection: an empty index, never a warning.
    NoCollection,
    /// No answer within `PROBE_TIMEOUT`, or a transport or server error: its sentence.
    Unreachable(String),
    /// Qdrant refused the API key ([`is_qdrant_refused`]): its sentence.
    Refused(String),
    /// Another width, another embedder or a newer payload stamp ([`is_embedder_mismatch`]).
    Mismatch(String),
}
impl Probe {
    /// The probe's verdict on a failed call: `Refused`, `Mismatch`, else `Unreachable`.
    #[must_use] pub fn from_error(err: &StoreError) -> Self;
}

/// MOD-160 D9: `collection_exists`, then `ExistingCollection::check` against the pinned model's
/// identity (`model::identity()`, `embed::DENSE_DIM`). The client is built with `PROBE_TIMEOUT`.
/// Loads no model and creates nothing.
pub async fn probe(settings: &QdrantSettings, collection: &str) -> Probe;

/// [`probe`] against `mine`/`dim` (the live tests' `HashEmbedder`, P-10).
pub async fn probe_with(settings: &QdrantSettings, collection: &str, mine: &EmbedderIdentity, dim: usize) -> Probe;
```

`open` (`:1056-1066`) keeps its own check (D3, belt and braces).

### T1.3 `crates/htui/src/concepts.rs` (UPDATE, test only, P-1)

`the_index_job_and_the_search_share_one_model_load` (`:1529-1569`): replace
`index.search(query("anything", ..))` with `index.embedder()`. Keep the 500 ms "still waiting"
assertion and `loads == 1`. Fix the comment: the index job and every search take the model from one
load slot (H-26).

### T1.4 Tests first

- `htui-core` `qdrant.rs`:
  - `unavailable_puts_the_cause_after_the_sentence`
  - `a_blank_cause_is_the_sentence_alone`
- `vector.rs` unit (no Qdrant):
  - `probe_from_error_tells_refused_mismatch_and_unreachable`, over `qdrant_with_code(.., Some(7|16), ..)`, a `check_existing` refusal and `backend(..)`.
  - `a_probe_of_a_closed_port_is_unreachable_within_the_bound`: `http://127.0.0.1:1`, wrapped in `tokio::time::timeout(PROBE_TIMEOUT + 2 s)` in real time, and asserts `Probe::Unreachable(_)`.
- `tests/qdrant_live.rs`. Each case uses a throwaway collection (`fresh_name()`) and drops it.
  - `check_refuses_another_width_before_any_embedder`: a collection of `HashEmbedder::new(8)`, checked against `DENSE_DIM`.
  - `check_refuses_another_embedder`
  - `check_passes_an_unstamped_collection_and_writes_nothing`: the metadata is unchanged afterwards (the `stamping_keeps_the_collections_other_metadata` shape).
  - `check_passes_an_older_payload_stamp_and_drops_nothing`: the points are still counted afterwards.
  - `probe_of_a_missing_collection_is_no_collection_and_creates_none`: `collection_exists` is still `false`.
  - `probe_of_a_matching_collection_is_ready`, via `probe_with` and `HashEmbedder::new(DENSE_DIM).identity()`.
  - `probe_of_another_embedders_collection_is_mismatch`, via `probe` (the pinned identity) over a `HashEmbedder` collection.
- The `concepts.rs` rewrite (T1.3) passes as it is.

### T1.5 Commit groups

1. `test(MOD-160): the shared-model test waits on the embedder slot` (`concepts.rs`).
2. `feat(MOD-160): the knowledge-search sentence` (`htui-core/src/qdrant.rs`).
3. `feat(MOD-160): a read-only embedder check and a model-free probe` (`vector.rs`, `tests/qdrant_live.rs`).

**T1 files:** `crates/htui-core/src/qdrant.rs`, `crates/htui-store/src/vector.rs`, `crates/htui-store/tests/qdrant_live.rs`, `crates/htui/src/concepts.rs` (test only).

---

## T2 — Search path, session offer, worker (D2, D4, D5, D6, D9 worker) · after T1, parallel with T3, T5 and T6

### T2.1 D4: tagged connection slots (`concepts_worker.rs:388-453`)

```rust
/// How a cached store was made (MOD-160 D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Made {
    /// `QdrantStore::connect`: the collection ensured (payload indexes, stamp, MOD-144 rebuild).
    Ensured,
    /// `ExistingCollection::open` (D2): read-only; never serves a sync.
    ReadOnly,
}

struct Connections<T> { slot: Mutex<Option<Cached<T>>> }
struct Cached<T> { settings: QdrantSettings, made: Made, connection: Arc<T> }

impl<T> Connections<T> {
    /// The cached connection for `settings`' URL and key that `need` accepts: `ReadOnly` takes
    /// either kind, `Ensured` only an ensured one.
    fn get(&self, settings: &QdrantSettings, need: Made) -> Option<Arc<T>>;
    /// Caches `connection`. A `ReadOnly` put never replaces an `Ensured` connection for the same
    /// settings (a search racing an index run keeps the better one).
    fn put(&self, settings: QdrantSettings, made: Made, connection: Arc<T>);
    // `clear`, `resolved`, `forget`: unchanged (MOD-158 R1 B-L1).
}
```

### T2.2 D2: the search path (`QdrantIndex`, `:377-597`)

- Add the field `collection: String` (default `COLLECTION`) and
  `pub(crate) fn with_collection(self, name: &str) -> Self` (P-10). `connect` uses
  `QdrantStore::connect_to(&settings, embedder, &self.collection)`.
- `connect` (`:535-551`): `get(&settings, Made::Ensured)`, then `put(.., Made::Ensured, ..)`. The
  worker's index job (`concepts.rs:455-468`, `models.connect`) and `sync` therefore never run on a
  read-only store. If the cache holds one, they connect and replace it.
- New `async fn open_read_only(&self, settings: QdrantSettings) -> Result<Option<Arc<QdrantStore<RtenEmbedder>>>, String>`:
  1. `ExistingCollection::find_in(&settings, &self.collection)`. `None` returns `Ok(None)`: nothing
     is cached, created or loaded.
  2. `found.check(&model::identity(), DENSE_DIM)`. A mismatch is refused with no model.
  3. `self.embedder().await?` (the shared load).
  4. `found.open(embedder)`, then `put(settings, Made::ReadOnly, store)`.

  Every `StoreError` is mapped as `connect` maps it: `format!("{:#}", concepts::qdrant_refusal(e))`.
- `search` (`:573-582`) runs in this order:
  1. `self.settings()`, as today.
  2. `get(&settings, Made::ReadOnly)`.
  3. Otherwise `open_read_only`. `None` answers `Ok(Vec::new())`.
  4. `store.search(&query)`. A failure `forget`s the store, as today.
- `sync` (`:584-593`): unchanged apart from `connect`'s tag.

### T2.3 D5: the cached flag (`QdrantSource`, `:243-352`)

```rust
/// D5: how old a known answer may be before `offered()` asks again.
pub(crate) const OFFER_STALE_AFTER: Duration = Duration::from_secs(30);

/// D5: whether a Qdrant URL resolves, as last read. Shared with the spawned reads (`Arc`), so a
/// read needs no `&QdrantSource` and the tests' owned sources keep `publish(&self)`.
#[derive(Default)]
struct Configured {
    /// `None` until first known; `tokio::time::Instant`, so a paused clock ages it (no real child).
    known: Mutex<Option<(bool, tokio::time::Instant)>>,
    /// One staleness re-read at a time (`offered()`'s single flight). A drop guard in the task
    /// clears it, so an aborted task cannot leave it set.
    reading: AtomicBool,
    /// Bumped by `publish` and `reread`. A read records only if no bump happened since it began,
    /// so a read of the old store never overwrites the new store's answer.
    generation: AtomicU64,
}

pub struct QdrantSource {
    // env, store, key, keyring: unchanged
    configured: Arc<Configured>,
}

impl QdrantSource {
    // `new`: with `env.url` set, `known = Some((QdrantUrl::parse(..).is_ok(), now))` at once
    // (the override wins over every row, so it is known without a read).

    /// D5: the store, then the flag. `None` records at once with no task: the override's verdict,
    /// else `false` (offline, `--demo`). `Some` bumps the generation and spawns one forced read.
    pub fn publish(&self, store: Option<Arc<dyn UrlRow>>);

    /// D5: an in-process URL write or clear: bump the generation, spawn one forced read.
    pub fn reread(&self);

    /// D5: the cached answer, never awaited. Unknown answers `false` and starts a read. Older
    /// than `OFFER_STALE_AFTER`, it answers the old value and starts at most one read.
    #[must_use] pub fn offered(&self) -> bool;

    /// The read behind `publish`/`reread`/`offered`: `tokio::runtime::Handle::try_current()`,
    /// else nothing (no runtime, no read: the flag stays as it was); `resolve_url` over a
    /// URL-only `QdrantEnv` clone (no key copied) and the current store; records under its
    /// starting generation; logs nothing above `debug`.
    fn spawn_read(&self, forced: bool);

    // `url()` and `settings()`: record `resolve_url`'s verdict as soon as it is known (before the
    // key: a URL that resolves is configured even when the key cannot be read), under the
    // generation read at their start.
}
```

The worker's start already awaits `qdrant.source().url()` (`worker_cmd.rs:118-123`), and that read
now records the flag, so its first session knows. In the TUI, the loop's first `concepts.publish`
(`store_worker.rs:2999`) starts the read before any chat can open.

### T2.4 D5: the seams and the loop

- `htui-mcp/src/search.rs` `ConceptSearch`: `fn offered(&self) -> bool { true }`. Doc: "whether
  the tool is offered to a session opened now; read once at `open`, never awaited". The `search`
  doc names `unavailable(cause)`.
- `htui-mcp/src/host.rs` `open` (`:400-458`):
  `let search = config.search.clone().filter(|search| search.offered());`, then
  `Session { search: search.clone(), caps: HostCaps { search: search.is_some() }, .. }`.
  `offered()` takes only its own lock and awaits nothing (`R-NF-3`).
- `htui-mcp/src/tools/search.rs` (`:96-110`): both errors become
  `htui_core::qdrant::unavailable(..)`, including `"no concept index is attached"` (D6).
- `concepts_worker.rs` `ConceptIndex` (`:57-65`):
  - `fn offered(&self) -> bool { true }`
  - `fn url_changed(&self) {}`

  `QdrantIndex` forwards them to `source.offered()` and `source.reread()`.
  `ConceptsRuntime::url_changed(&self)` forwards to `self.index`.
- `mcp_search.rs`: `McpSearch::offered` returns `self.0.offered()`. Update `production`'s doc
  (`:87-90`): "offered per session while a URL resolves (MOD-160 D5)".
- `store_worker.rs`:
  - `StoreRequest::changes_qdrant_url(&self) -> bool` matches `SetQdrantUrl { .. } |
    ClearQdrantUrl { .. } | AdoptOldQdrantUrl`.
  - In the loop's `other =>` arm (`:3292-3315`), after `try_serve`:
    `if other.changes_qdrant_url() { concepts.url_changed(); }`. It fires whatever the reply was:
    a stale or refused write says the row may have moved, and the re-read is one cheap read.

### T2.5 D9 worker half: `QdrantCheck` (`concepts_worker.rs`, new; T4 reuses it)

```rust
/// MOD-160 D9: what one model-free look at this process's Qdrant says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QdrantCheck {
    /// No URL, or an unusable one: `Unresolved`'s sentence.
    NotConfigured(String),
    /// Unreachable, an embedder mismatch, the row or the key unreadable: the sentence.
    Unavailable(String),
    /// Refused while this box has a key: the sentence.
    KeyRefused(String),
    /// Refused and no key from any source.
    NoKey(String),
    /// Answered without any key (Q4): info, not a warning.
    Keyless,
    /// Answered with a key, or nothing to say.
    Ready,
}
impl QdrantCheck {
    /// Pure (unit-tested): `Err(NoUrl | Unusable)` → NotConfigured; `Err(Offline)` → `None`
    /// (nothing probed); `Err(Read | Key)` → Unavailable; `Ok(s)` with `probe`:
    /// `Ready | NoCollection` → Keyless when `s.api_key` is `None`, else Ready;
    /// `Unreachable | Mismatch` → Unavailable; `Refused` → KeyRefused with a key, NoKey without.
    pub(crate) fn of(resolved: Result<&QdrantSettings, &Unresolved>, probe: Option<&Probe>) -> Option<Self>;
    /// The state in words: "no Qdrant is configured", "Qdrant is unreachable or refuses the
    /// collection", "the Qdrant refused this box's API key", "this box lacks the Qdrant API key".
    #[must_use] pub fn state(&self) -> Option<&'static str>;
    /// `"{state}: {cause}"` for the four warnings; `None` for Keyless and Ready.
    #[must_use] pub fn warning(&self) -> Option<String>;
    /// "this Qdrant answers without an API key" for Keyless only.
    #[must_use] pub const fn info(&self) -> Option<&'static str>;
}

/// D9: `source.settings()`, then `vector::probe` when they resolve. No model, nothing created.
pub(crate) async fn check_qdrant(source: &QdrantSource, collection: &str) -> Option<QdrantCheck>;
```

`worker_cmd.rs` (`:116-160`):

- Keep the awaited `qdrant.source().url()` select. It now only primes the flag; bind it to `_`.
- Delete `worker_search`. Attach unconditionally:
  `match crate::mcp_search::production() { Some(s) => tools.with_search(s), None => tools }`.
- After `spawn_index_job`, run `tokio::spawn(start_check(Arc::clone(qdrant.source())))`. It is not
  awaited, so a 3 s unreachable probe never delays "htui worker ready". It logs `start_line`'s line
  once.
- `fn start_line(check: Option<&QdrantCheck>) -> Option<(tracing::Level, String)>`:
  - a warning gives `WARN` with `unavailable(&warning)`;
  - `Keyless` gives `INFO` with the info line;
  - otherwise `None`.

  It is never `ERROR`: an `error` becomes a GlitchTip report (`docs/htui-worker.md`).

### T2.6 Tests first

- `concepts_worker.rs` (unit, no Qdrant, `QdrantEnv` literals, `with_keyring(no_key)`):
  - `a_read_only_connection_never_serves_a_sync` (`Connections<u32>`):
    - a `ReadOnly` put is seen by `get(.., ReadOnly)` and not by `get(.., Ensured)`;
    - an `Ensured` put serves both;
    - a `ReadOnly` put over an `Ensured` one for the same settings keeps the `Ensured` one.
  - Rewrite `a_connection_is_reused_for_its_own_settings_until_it_is_forgotten` (`:1673`) and
    `unresolved_settings_drop_the_cached_connection` (`:1759`) for the new `get`/`put`.
  - `a_search_against_an_unreachable_qdrant_loads_no_model`: a counting loader, env URL
    `http://127.0.0.1:1`. The search answers `Err` naming "cannot reach Qdrant", the load count is 0
    and nothing is cached.
  - `qdrant_index_without_a_stored_url_names_where_to_set_it` (`:1593`): kept as it is.
  - Flag cases, `#[tokio::test(start_paused = true)]`, over `MemStore` rows and a counting
    `UrlRow` (the `NoRow` shape, `concepts.rs:1491`), no child process:
    - `an_unknown_offer_is_false_and_starts_one_read`
    - `a_url_row_is_offered_after_the_read`
    - `no_url_and_offline_are_not_offered` (a `None` publish records at once: no read counted)
    - `an_override_is_known_without_a_read`
    - `a_stale_answer_is_used_and_read_again_once`: advance 31 s, call `offered()` five times
      while the counting row sleeps 1 s, then assert exactly one read and the new value after it.
    - `a_read_of_the_old_store_never_overwrites_the_new_one`: publish A (slow), publish B (fast),
      then assert B's verdict.
    - `a_reread_after_a_write_picks_up_a_cleared_row`
    - `a_search_records_the_offer`
  - `qdrant_check_names_each_state` (pure `QdrantCheck::of`, the full table, with Offline → `None`).
  - `a_check_never_says_the_key`: env key `KEY-SENTINEL`, URL `http://127.0.0.1:1`. No string the
    check produces contains it.
  - Live, gated on `HTUI_TEST_QDRANT_URL` (skip line), each over `with_collection(fresh)` with a
    `pending()` loader:
    - `a_tui_search_against_an_empty_qdrant_creates_no_collection_and_loads_nothing`: `Ok([])`,
      `find_in(fresh)` is `None`, and the load count is 0.
    - `a_tui_search_of_another_embedders_collection_is_refused_before_the_model`: create with
      `QdrantStore::connect_to(.., HashEmbedder::new(DENSE_DIM), fresh)`, search, expect
      `EMBEDDER_MISMATCH` and a load count of 0, then drop the collection.
- `htui-mcp/tests/tools_search.rs`. `Recording` gains `offered: AtomicBool` (default `true`).
  - `a_search_not_offered_at_open_is_not_advertised_and_a_call_is_unknown`: absent from
    `tools/list`, and a call answers `-32602`.
  - `the_offer_is_read_per_session`: flip between two opens.
  - Update `an_unavailable_index_is_is_error_with_its_cause` (`:269`) to
    `knowledge search unavailable: {cause}`.
  - The snapshot `tools_search__a_fresh_chat_scope_sees_box_profile_and_search_concepts` is
    unchanged (the default is offered).
- `mcp_search.rs`: `the_adapter_forwards_the_offer`. `production_offers_the_shared_index`
  (`:317`) is kept: it asserts `is_some()` only.
- `store_worker.rs`: `a_qdrant_url_write_tells_the_index_to_reread` over
  `spawn_with_concepts(demo, .., ConceptsRuntime::new(counting index))`. Send `SetQdrantUrl`
  (answered `DEMO_SESSION`): one `url_changed`. Then send `QdrantInfo`: still one.
- `worker_cmd.rs`: delete `the_worker_offers_search_only_where_a_url_resolves_at_start` (`:357`)
  and add `the_start_line_warns_or_informs_and_never_errors`.

### T2.7 Commit groups

1. `feat(MOD-160): searches ask Qdrant first, never create, and tag their connection` (`concepts_worker.rs` D2/D4 and tests).
2. `feat(MOD-160): search_concepts offered per session from a cached flag` (`concepts_worker.rs` D5, `mcp_search.rs`, `htui-mcp/src/{search,host}.rs`, `htui-mcp/src/tools/search.rs`, `htui-mcp/tests/tools_search.rs`, `store_worker.rs`, `docs/htui-mcp.md`).
3. `feat(MOD-160): the worker offers per session and warns at start` (`concepts_worker.rs` `QdrantCheck`, `worker_cmd.rs`, `docs/htui-worker.md`).

**T2 files:** `crates/htui/src/concepts_worker.rs`, `crates/htui/src/mcp_search.rs`, `crates/htui/src/store_worker.rs`, `crates/htui/src/worker_cmd.rs`, `crates/htui-mcp/src/search.rs`, `crates/htui-mcp/src/host.rs`, `crates/htui-mcp/src/tools/search.rs`, `crates/htui-mcp/tests/tools_search.rs`, `docs/htui-worker.md`, `docs/htui-mcp.md`.

---

## T3 — The overlay (D7) · after T1, parallel with T2, T5 and T6

### T3.1 Keys (follow `Act::ConceptsReindex` end to end)

| Place | Change |
|---|---|
| `keys/catalogue.rs` `Act` (`:563-573`) | `/// \`concepts.why\`: show why knowledge search is unavailable.` `ConceptsWhy,` after `ConceptsDown` |
| `CATALOGUE` `[concepts]` rows (`:1438-1466`) | `capture_row(Act::ConceptsWhy, Concepts, "why", &["ctrl-e"], "why search is unavailable"),` after `down`, with a `// concepts_search.rs` site comment. `since` 1 (not M2's) |
| `ALL` (`:2686-2687`) | append `Act::ConceptsWhy` **last**, after `LogsBottom` (MOD-158 precedent: no position moves) |
| `position` (`:2908`) | `Act::ConceptsWhy => 216,` |
| `every_act_has_exactly_one_row` (`:2942`) | `216` → `217` |
| `in_capture_acts_…` (`:3392-3428`) | add `Act::ConceptsWhy` |
| `keys/views.rs` `CONCEPTS_QUERY` (`:509-524`) | add `Act::ConceptsWhy` to the `[concepts]` view layer, last |
| `views.rs` `the_view_stacks_resolve_as_designed` (`:1432-1446`) | `assert_eq!(resolve(CONCEPTS_QUERY, "ctrl-e"), [Act::ConceptsWhy]);` |
| `keys/hint.rs` (`:783-795`) | the "Search concepts:" line gains ` · Ctrl+e why search is unavailable` |
| `docs/htui-keys.md` (`:160-165`) | regenerate the toml block with `cargo run -q -p htui -- --default-keys --print-keys` (a `why = ["ctrl-e"]` row) |

`every_m2_row_is_since_2` and `the_v1_defaults_…` are unaffected: the act is appended after M2's block and is `since` 1.

### T3.2 `ui/overlay/concepts_search.rs`

- Add the field `why: bool`, the row being open. It is cleared by `enter` when it sends and by a
  successful reply.
- Add a sibling of `HINT` (`:123-130`): `HINT_WHY` = `HINT` plus `Hint::One(Act::ConceptsWhy, "why")`
  before `OverlayClose`. `render` uses it only while `error.is_some()`.
- Add
  `pub fn unavailable_status(why: Option<&str>) -> String`. With a key bound it is
  `"{KNOWLEDGE_SEARCH_UNAVAILABLE} \u{2014} {why} why"`; with none it is the sentence alone.
  `why` is `keys.label(views::CONCEPTS_QUERY, Act::ConceptsWhy)`.
- `status` (`:280-308`): the error branch answers `(unavailable_status(..), theme.error)`. It no
  longer answers `with_place(error)`.
- `on_key` (`:343-372`): `Some(Act::ConceptsWhy)` toggles `why` when `error.is_some()`, and is
  consumed with no change otherwise.
- `render` (`:421-512`): while `why && error.is_some()` and the hits are empty (a failed search
  clears them), the empty hit rows show `wrap(&with_place(error), cells)` in `theme.error`, first
  in the rows. The index line's own long-error wrap stays and comes after.
  - The status row is now short, so the "status error too wide" branch no longer fires for
    searches.
  - `OTHER_ROWS` and the modal size are unchanged, so no other `concepts_search__*` snapshot moves.
- The Ctrl+R index line keeps `with_place` (D7). `place`/`with_place` are unchanged.

### T3.3 Tests first

- `concepts_search.rs` unit (Bench):
  - `a_failed_search_says_unavailable_and_names_the_why_key`: the status row reads
    `knowledge search unavailable — Ctrl+e why`.
  - `the_why_key_toggles_the_cause_with_its_place`: `qdrant: …` shows
    `… — Settings › Qdrant`, and a second `ctrl-e` hides it.
  - `the_why_key_without_a_cause_changes_nothing`
  - `a_new_search_closes_the_why_row`
  - `the_border_names_why_only_while_there_is_a_cause`
  - `an_unbound_why_key_leaves_the_sentence_alone`
  - `an_empty_index_still_says_no_matches`
  - Rewrite these to go through the why row, keeping what each pins (the wrap, the place by
    origin): `a_long_error_wraps_across_the_empty_hit_rows` (`:1177`),
    `a_short_error_stays_on_its_row` (`:1225`), `an_error_names_the_place_of_its_origin`
    (`:1381`), `an_error_names_where_to_fix_it` (`:1426`) and `a_failed_search_lets_enter_retry`
    (`:1054`).
- `tests/concepts_search.rs` (harness, `MemIndex::failing`):
  - `a_failing_search_says_unavailable_and_ctrl_e_shows_why`, with the `error` snapshot regenerated
    and a new `error_why` snapshot.
  - `a_rebound_why_key_moves_the_hint_and_acts`, in the shape of
    `a_rebound_reindex_acts_and_the_old_chord_is_inert` (`:492`).
  - Update `without_a_concepts_runtime_the_search_says_not_available` (`:396`): the sentence on the
    status row, `NOT_AVAILABLE` behind `ctrl-e`.
- Keys: the five pins of §T3.1, plus `tests/keys_file.rs` `reference_page` against the regenerated
  doc.
- Snapshots: `concepts_search__error.snap` moves and `concepts_search__error_why.snap` is new.
  `decisions_project`, `empty`, `hits`, `hits_80` and `searching` must not move: confirm with a full
  `cargo insta test -p htui --all-features`.

### T3.4 Commit groups

1. `feat(MOD-160): concepts.why, a rebindable key for the search's cause` (`keys/{catalogue,views,hint}.rs`, `docs/htui-keys.md`).
2. `feat(MOD-160): the search overlay says knowledge search unavailable, its cause one key away` (`concepts_search.rs`, `tests/concepts_search.rs`, snapshots).

**T3 files:** `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs`, `crates/htui/src/keys/hint.rs`, `crates/htui/src/ui/overlay/concepts_search.rs`, `crates/htui/tests/concepts_search.rs`, `crates/htui/tests/snapshots/concepts_search__error.snap`, `crates/htui/tests/snapshots/concepts_search__error_why.snap` (new), `docs/htui-keys.md`.

---

## T4 — Settings and launch warnings (D9) · after T2

### T4.1 Request, reply and routing (`store_worker.rs`)

- `StoreRequest::ProbeQdrant`. Doc: "MOD-160 D9: a model-free look at this process's Qdrant,
  served by `ConceptsRuntime` on a task of its own (`R-NF-3`), never on the loop". `name()` is
  `"probe_qdrant"`. It is **not** `is_qdrant`.
- `StoreReply::QdrantProbe(Option<QdrantCheck>)`, next to `QdrantOldUrl` (`:1628-1630`). `None`
  means nothing was probed: offline, no runtime, or `--demo`.
- Routing:
  - The loop's concepts arm (`:3273`) gains `| StoreRequest::ProbeQdrant`.
  - `try_serve` (`:2403-2411`) answers `ProbeQdrant => StoreReply::QdrantProbe(None)` for the
    harness and `--demo`.
  - `testkit.rs:401` gains it in the concepts arm (P-7).

### T4.2 The runtime (`concepts_worker.rs`)

- `REQUEST_NAMES: [&str; 3] = ["search_concepts", "index_concepts", "probe_qdrant"]`.
- `ConceptIndex::check(&self) -> BoxFuture<'_, Option<QdrantCheck>> { Box::pin(async { None }) }`.
  `QdrantIndex` answers `check_qdrant(&self.source, &self.collection)`. `MemIndex` gains
  `with_check(QdrantCheck)` and answers it after `pause()`.
- `ConceptsRuntime::serve` (`:815-875`) gains an arm `StoreRequest::ProbeQdrant`. It spawns
  `index.check()` and sends `QdrantProbe(check)`, keyed by `(origin, kind)` like the others (a newer
  probe from the same view aborts the older one).

### T4.3 The Settings section (`ui/tabs/settings/qdrant.rs`)

- New fields:
  - `probe: Option<QdrantCheck>`
  - `probed_for: Option<(UrlRowState, Option<Result<QdrantUrl, QdrantUrlError>>, QdrantState)>`,
    the snapshot fields the last probe was sent for.
- `on_reply` (`:1113-1120`):
  - On `StoreReply::Qdrant(s)`, run `on_snapshot(s)`. Then, unless `s.url` is `Offline` or
    `NotApplicable`, and if `probed_for` differs, `ctx.request(StoreRequest::ProbeQdrant)` and
    record it.
  - On `StoreReply::QdrantProbe(c)`, set `self.probe = c.clone()`.
- `Act::Reload` (`:743-748`) also clears `probed_for`, so the reload's snapshot probes again (D9,
  "on its reload key").
- `render` (`:1132-1176`): between the form and the notice, `probe_lines(width, theme)`. A warning
  is drawn in `theme.warning` (`theme.rs:88`), the Keyless info line in `theme.dim`. Nothing is
  drawn while `unavailable` is set or the URL row is `Offline`/`NotApplicable`. No kit change (the
  kit's `Tone` is Info/Error only, `kit/text.rs:42-47`). Nothing that renders the key: the check
  carries none.

### T4.4 The launch notice (`app/state.rs`, `app/update.rs`)

- `state.rs` fields (beside `:501-506`, initialised at `:612-614`): `qdrant_probe_sent: bool` and
  `qdrant_warning_pending: Option<String>`.
- `update.rs` `observe_reply`, `StoreState` arm (`:518-533`): when `label == "online" &&
  self.qdrant_offer_check && !self.qdrant_probe_sent`, set the flag, then
  `self.dispatch(Origin::App, StoreRequest::ProbeQdrant)` (P-12).
- `on_app_reply` (`:601-656`): on `StoreReply::QdrantProbe(Some(check))`, take `check.state()` and
  set `qdrant_warning_pending = Some(format!("{KNOWLEDGE_SEARCH_UNAVAILABLE}: {state}
  \u{2014} Settings \u{203a} Qdrant"))`, then `say_qdrant_warning()`.
- `say_qdrant_warning` mirrors `say_qdrant_offer` (`:568-575`): it says the notice only when
  `self.status.is_none()`, otherwise it waits. Call it from `on_tick` beside `say_qdrant_offer`
  (`:299`). Ready, Keyless and `None` say nothing.

### T4.5 Docs

None beyond the code's docs: Settings' rows explain themselves, and README belongs to T6 (P-14).

### T4.6 Tests first

- `store_worker.rs`: `the_loop_answers_while_a_qdrant_probe_runs` (`start_paused`):
  `ConceptsRuntime::new(MemIndex::new().delayed(1 s).with_check(QdrantCheck::Unavailable(..)))`.
  Send `ProbeQdrant` (seq 1) and then `BoxInfo` (seq 2). The first reply is seq 2. The second is
  `QdrantProbe(Some(Unavailable))` (P-8).
- `store_worker.rs`: `without_a_runtime_a_probe_answers_nothing_probed` (`serve` → `QdrantProbe(None)`).
- `concepts_worker.rs`:
  - `request_names_match_the_name_arms` for three entries;
  - `a_probe_is_a_concepts_reply_task_and_never_failed`;
  - `a_qdrant_index_probe_of_a_closed_port_is_unavailable` (`127.0.0.1:1`, real time, bounded).
- `tests/qdrant_settings.rs` (literal replies, no runtime):
  - `a_snapshot_asks_for_one_probe` and `the_same_snapshot_does_not_ask_again`
  - `r_probes_again`
  - `offline_and_demo_send_no_probe`
  - one case per state, each with a snapshot: `probe_no_url`, `probe_unreachable`,
    `probe_mismatch`, `probe_key_refused`, `probe_no_key`, `probe_keyless_info`
  - `ready_draws_no_row`
  - `no_probe_line_renders_a_key`: key `KEY-SENTINEL` in the fixture.
  - `requests_of` drops `ProbeQdrant` (P-11).
- `tests/settings.rs`: `requests_of` (`:5745-5756`) drops `ProbeQdrant`. The existing Qdrant cases
  keep their assertions.
- `app/update.rs` unit:
  - `the_qdrant_probe_is_sent_once_when_the_store_is_first_online`
  - `no_probe_without_register_all`
  - `a_warning_is_said_once_and_never_over_an_earlier_notice`
  - `ready_keyless_and_nothing_probed_say_nothing`
- Snapshots: only the new `qdrant_settings__probe_*.snap` are created. All 13 existing
  `qdrant_settings__*.snap` and every shell frame must not move (no probe reply in them; a harness
  over Postgres answers `QdrantProbe(None)`): confirm with a full insta run.

### T4.7 Commit groups

1. `feat(MOD-160): ProbeQdrant, served off the loop by the concepts runtime` (`store_worker.rs`, `concepts_worker.rs`, `testkit.rs`).
2. `feat(MOD-160): Settings › Qdrant warns from the probe` (`qdrant.rs`, `tests/qdrant_settings.rs`, `tests/settings.rs`, snapshots).
3. `feat(MOD-160): one launch notice when knowledge search is unavailable` (`app/state.rs`, `app/update.rs`).

**T4 files:** `crates/htui/src/store_worker.rs`, `crates/htui/src/concepts_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/src/ui/tabs/settings/qdrant.rs`, `crates/htui/src/app/state.rs`, `crates/htui/src/app/update.rs`, `crates/htui/tests/qdrant_settings.rs`, `crates/htui/tests/settings.rs`, `crates/htui/tests/snapshots/qdrant_settings__probe_*.snap` (new). `tests/qdrant_settings_pg.rs` is **not** touched: the probe is not `qdrant_settings_info`'s.

---

## T5 — CLI wording (D8) · after T1, parallel with T2, T3 and T6

`crates/htui/src/concepts.rs`:

- `fn unavailable_failure(mut failure: Failure) -> Failure` sets
  `failure.error.message = htui_core::qdrant::unavailable(&failure.error.message)`.
  `unresolved_failure` (`:582-595`) passes every arm through it. `store`, `code` and the exit code
  are unchanged.
- `search_index` (`:696-727`): the model's failure becomes
  `IndexAnswer::Failed(unavailable(&format!("{err:#}")))`.
- `index_refusal` (`:733-749`): every sentence goes through `unavailable(..)`.
- `IndexAnswer::Empty` and the `Hits` notes are unchanged. So are `--index-items` (`open`,
  `:108-132`) and `qdrant_refusal`, which the TUI's connect reads too.
- The human line is `StateExit`'s message, printed by `main`, so it changes with the JSON
  `error.message`.

Tests first:

- `concepts.rs` unit (no database):
  - `index_refusal_says_knowledge_search_unavailable_first` (all four kinds, kinds unchanged)
  - update `index_refusals_by_cause` (`:1012`)
  - `unresolved_failures_keep_their_code_and_say_unavailable` (NoUrl, Unusable, Read
    Unreachable/other, Key, Offline)
  - `search_index_against_a_closed_port_is_offline_unavailable_and_loads_nothing` (`127.0.0.1:1`,
    a panicking `load`)
- `tests/state_cli_pg.rs`:
  - `:587-591`, `:638`, `:695-698` and `:738` additionally assert
    `message.starts_with("knowledge search unavailable: ")`;
  - `:713` becomes `starts_with("knowledge search unavailable: HTUI_QDRANT_URL ")`;
  - exit codes and `error.code`s unchanged;
  - `a_search_without_a_collection_is_empty_and_loads_no_model` (`:792`) still answers `empty`
    with no message.
- `docs/htui-cli.md` (`:356-425` and the codes table): the message of every unavailable search
  starts `knowledge search unavailable: `; codes and exits unchanged; `empty` unchanged.

Commit: `feat(MOD-160): --search-items says knowledge search unavailable` (all three files).

**T5 files:** `crates/htui/src/concepts.rs`, `crates/htui/tests/state_cli_pg.rs`, `docs/htui-cli.md`.

---

## T6 — Gate step `qdrant` (D10) · parallel with T2, T3 and T5 (it needs nothing from T1)

- **`scripts/gate.sh`**:
  - `STEPS=(fmt clippy-all clippy-bare postgres qdrant test …)` (`:44`), and the usage comment
    (`:11-12`).
  - `step_qdrant` after `step_postgres` (`:86-105`):
    - unset prints "qdrant: HTUI_TEST_QDRANT_URL unset - the Qdrant suites return early and prove
      nothing" and returns 3;
    - otherwise match `^https?://([^@/]*@)?(\[[^]/]+\]|[^:/?#]+)(:([0-9]+))?`, with no match
      printing "qdrant: URL set, host not parsed (not probed)" and returning 0;
    - host is group 2 with the brackets stripped; port is group 4, else 80 for `http`, 443 for
      `https` (what the client would dial);
    - `timeout 3 bash -c ">/dev/tcp/$host/$port"`, failing with "qdrant: $host:$port unreachable -
      the Qdrant suites would fail" (return 1), else "qdrant: $host:$port reachable; the Qdrant
      suites run live";
    - nothing beyond host:port is ever printed.
  - Self-test (`:260-267` shape):
    - unset → 3;
    - `http://127.0.0.1:1` → 1;
    - `not-a-url` → 0;
    - `http://u:s3cret@127.0.0.1:1` → output without `s3cret`.
- **`scripts/gate.ps1`**:
  - `ValidateSet` (`:18`), `$Steps` (`:35`), the usage comment (`:7-8`);
  - `Step-Qdrant` mirroring `Step-Postgres` (`:72-99`, `TcpClient.ConnectAsync(..).Wait(3000)`);
  - the same four self-test cases inside a saved and restored `$env:HTUI_TEST_QDRANT_URL`
    (`:247-257` shape).
- **`.claude/skills/htui-dev/hooks/commands.ts:8`**: `GATE_STEPS` gains `'qdrant'` after
  `'postgres'`. **`tests/commands.test.ts:57, 128`**: both lists.
- **`docs/htui-dev.md:339`**: the `/gate <step>` steps list.
- **`README.md:699-700`**: one sentence. "The gate's `qdrant` step reports whether they ran:
  skipped while `HTUI_TEST_QDRANT_URL` is unset, failed when its host and port do not answer within
  3 seconds."

Commit: `feat(MOD-160): gate step qdrant` (all six files together, because `gate.sh --self-test`
compares `GATE_STEPS` with `STEPS`, `:284-289`).

**T6 files:** `scripts/gate.sh`, `scripts/gate.ps1`, `.claude/skills/htui-dev/hooks/commands.ts`, `.claude/skills/htui-dev/tests/commands.test.ts`, `docs/htui-dev.md`, `README.md`.

---

## 7. File sets and coupling

| Task | Files |
|---|---|
| T1 | `htui-core/src/qdrant.rs`, `htui-store/src/vector.rs`, `htui-store/tests/qdrant_live.rs`, `htui/src/concepts.rs` (one test) |
| T2 | `htui/src/{concepts_worker,mcp_search,store_worker,worker_cmd}.rs`, `htui-mcp/src/{search,host}.rs`, `htui-mcp/src/tools/search.rs`, `htui-mcp/tests/tools_search.rs`, `docs/htui-{worker,mcp}.md` |
| T3 | `htui/src/keys/{catalogue,views,hint}.rs`, `htui/src/ui/overlay/concepts_search.rs`, `htui/tests/concepts_search.rs`, `concepts_search__error{,_why}.snap`, `docs/htui-keys.md` |
| T5 | `htui/src/concepts.rs`, `htui/tests/state_cli_pg.rs`, `docs/htui-cli.md` |
| T6 | `scripts/gate.{sh,ps1}`, `htui-dev/hooks/commands.ts`, `htui-dev/tests/commands.test.ts`, `docs/htui-dev.md`, `README.md` |
| T4 | `htui/src/{store_worker,concepts_worker,testkit}.rs`, `htui/src/ui/tabs/settings/qdrant.rs`, `htui/src/app/{state,update}.rs`, `htui/tests/{qdrant_settings,settings}.rs`, `qdrant_settings__probe_*.snap` |

- **T2, T3, T5 and T6 are pairwise disjoint** once P-1 moves the shared-model test into T1 and P-2
  moves the URL-write hook from `qdrant_settings_info.rs` into `store_worker.rs`. The parallel
  marking holds.
- **T4 after T2.** T4 shares `concepts_worker.rs` and `store_worker.rs` with T2, and reads its
  `QdrantCheck`. T4 and T3/T5/T6 are disjoint, so T4 may start as soon as T2 lands, even while
  T3/T5/T6 still run.

Coupling checked and found clean:

- **Snapshots.** T3 owns `concepts_search__*` and T4 owns `qdrant_settings__*`. T2 renders no
  frame. The `tools_search` snapshot is unchanged because a fake is offered by default.
- **Keys.** The catalogue, print-keys doc and catalogue pins are all T3's.
- **No `.sqlx` or migration** in any task.
- **`REQUEST_NAMES` and request names.** `concepts_worker::REQUEST_NAMES` and its test are T4's.
  `qdrant_settings_info::REQUEST_NAMES` is untouched.
- **store_worker routing tests.** They are T2's (`a_qdrant_url_write_tells_the_index_to_reread`)
  and T4's (`the_loop_answers_while_a_qdrant_probe_runs`), which run serially.
- **Trait impls.** `ConceptIndex` impls outside the module (`Publishes`, `store_worker.rs:3787`)
  take the new default methods, so neither task edits them.

---

## 8. Verification per task (each implementer runs these on the real tree)

| Task | Commands |
|---|---|
| T1 | `cargo test -p htui-core`; `cargo test -p htui-store --all-features --lib vector`; `cargo test -p htui-store --all-features --test qdrant_live -- --test-threads=1` (live, owed on the host if the sandbox Qdrant times out); `cargo test -p htui --all-features --lib concepts::` |
| T2 | `cargo test -p htui-mcp --all-features`; `cargo test -p htui --all-features --lib concepts_worker mcp_search worker_cmd store_worker -- --test-threads=1`; `cargo clippy --workspace -- -D warnings` |
| T3 | `cargo test -p htui --all-features --lib keys ui::overlay::concepts_search`; `cargo test -p htui --all-features --test concepts_search --test keys --test keys_file`; `cargo insta test -p htui --all-features` and read each changed frame |
| T4 | `cargo test -p htui --all-features --lib store_worker concepts_worker app:: -- --test-threads=1`; `cargo test -p htui --all-features --test qdrant_settings --test settings`; full insta run |
| T5 | `cargo test -p htui --all-features --lib concepts:: state_cmd`; `cargo test -p htui --all-features --test state_cli_pg -- --test-threads=1` (Postgres) |
| T6 | `bash scripts/gate.sh --self-test`; `pwsh -NoProfile -File scripts/gate.ps1 -SelfTest` (pwsh from /tmp, `DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1`); `bun test` in `.claude/skills/htui-dev`; `bash scripts/gate.sh --only qdrant` with `HTUI_TEST_QDRANT_URL` set (passed) and unset (skipped) |

Then the plan's Validation block on the merged tree: `bash scripts/gate.sh`, featureless clippy,
`cargo test -p htui --all-features -- --test-threads=1` and the workflow validator.

## 9. Risks and gotchas

- **R-1: background flag reads through the shared index in store-loop tests.** Loop tests built
  with `ConceptsRuntime::production()` (`store_worker.rs:2916`) publish `Some(pg)` on a Postgres
  backend, which spawns one `qdrant_url()` read through the test's pool. It is harmless: one read,
  logged at `debug`, and no test asserts the shared flag. `None` publishes (memory, offline) spawn
  nothing (§T2.3).
- **R-2: no runtime, no read.** `offered()` from code without a runtime returns the cached value
  and starts nothing (`Handle::try_current`). Unit tests of `McpHost` with a fake are unaffected
  (default `true`).
- **R-3: the border hint at 100 columns.** MOD-115 L-E Q5 keeps the concepts border row on 100
  columns. `HINT_WHY` adds `Ctrl+e why` only while a cause is shown. T3 checks the 100-column frame
  (the `INNER_CELLS` cases, `tests/concepts_search.rs:41-45`); the status row names the key either
  way.
- **R-4: the launch probe in Postgres harness tests.** A harness that calls `register_all` and
  goes `online` sends one `ProbeQdrant`, which `serve` answers `QdrantProbe(None)`. A test pinning
  the exact request list of such a harness would see it. None was found (`qdrant_info_seq`,
  `app/update.rs:1204`, filters by variant); the T4 full run confirms.
- **R-5: a strict keys file that binds `ctrl-e` globally** now collides in the concepts search
  (P-6). The catalogue tests accept the default.
- **R-6: live Qdrant suites in the sandbox** (3.5–19 s creates): T1's and T2's live cases may time
  out. They are owed on the host, and the done-report says which.
- **R-7: the flag lags a URL set on another box by up to 30 s** (Q1, accepted). The first session
  after a TUI start may open before the first read lands; unknown is "not offered" (`R-NF-2`).
