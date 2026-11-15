//! MOD-127 M2 T3 (D2-D5): the `mcp_server` registry and its `toolbox_binding` rows, on a real
//! server.
//!
//! `0042_mcp_server.sql` adds the table, widens `toolbox_binding.kind` and adds the attachment
//! shape check. What `MemStore` cannot show is that Postgres agrees with it: the same servers
//! attached at the same levels resolve to the same step list through `model::resolve_mcp`, the
//! constraints refuse the rows the model's rules rule out under their own names, and a server's
//! JSON columns round-trip in order. The conformance cases (`pg_conformance`) cover the writers.
//!
//! Raw inserts are `sqlx::query`, unchecked, so the file adds nothing to `.sqlx`. With
//! `HTUI_TEST_DATABASE_URL` unset each case prints `common::SKIP` and passes.
#![cfg(feature = "demo")]

use htui_store::testkit as common;

use htui_core::fixtures::ids;
use htui_core::model::{
    Activation, Attachment, BindingChange, BindingKey, BindingLevel, BindingRef, BoundMcp,
    EnvValue, ItemKindId, McpServer, McpServerId, McpTransport, NewMcpServer, PhaseId, ProjectId,
    ResolveScope, WorkspaceId,
};
use htui_core::store::{MemStore, WriteStore};
use sqlx::postgres::{PgPool, PgQueryResult};
use uuid::Uuid;

/// A stdio server request with a fresh id and one env entry of each reference kind.
fn stdio(name: &str) -> NewMcpServer {
    NewMcpServer {
        id: McpServerId::new(),
        name: name.to_owned(),
        description: format!("{name} server"),
        transport: McpTransport::Stdio,
        command: format!("mcp-{name}"),
        args: vec!["--stdio".to_owned(), "--quiet".to_owned()],
        url: None,
        headers: Vec::new(),
        env: vec![
            ("Z_MODE".to_owned(), EnvValue::Literal("fast".to_owned())),
            ("A_TOKEN".to_owned(), EnvValue::Secret("a-token".to_owned())),
        ],
        allow: vec!["search".to_owned()],
        deny: vec!["delete".to_owned()],
        created_by: ids::USER,
    }
}

/// One attachment to plant: the server's name, its key's four scope parts and its activation.
type Plant = (
    &'static str,
    Option<WorkspaceId>,
    Option<ProjectId>,
    Option<ItemKindId>,
    Option<PhaseId>,
    Activation,
);

/// The attachments of the table: each server at the levels the cases below resolve.
fn plants() -> Vec<Plant> {
    let project = Some(ids::PROJECT_HTUI);
    vec![
        ("alpha", None, None, None, None, Activation::Always),
        (
            "alpha",
            Some(ids::WORKSPACE_PLATFORM),
            None,
            None,
            None,
            Activation::Off,
        ),
        ("beta", None, project, None, None, Activation::Always),
        (
            "beta",
            None,
            project,
            None,
            Some(ids::PHASE_HTUI_IMPLEMENT),
            Activation::Off,
        ),
        ("gamma", None, None, None, None, Activation::Off),
        (
            "gamma",
            None,
            project,
            Some(ids::KIND_HTUI_FEAT),
            None,
            Activation::Always,
        ),
        (
            "delta",
            None,
            project,
            None,
            Some(ids::PHASE_HTUI_IMPLEMENT),
            Activation::Always,
        ),
    ]
}

/// Creates `requests` in `store` and plants [`plants`]; `epsilon` stays unattached.
async fn fill<S: WriteStore>(store: &S, requests: &[NewMcpServer]) -> Vec<McpServer> {
    let mut made = Vec::new();
    for request in requests {
        made.push(
            store
                .create_mcp_server(request.clone())
                .await
                .expect("create a server"),
        );
    }
    for (name, workspace, project, item_kind, phase, activation) in plants() {
        let server = made
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("`{name}` is one of the five"));
        let key = BindingKey {
            target: BindingRef::McpServer(server.id),
            workspace,
            project,
            item_kind,
            phase,
        };
        store
            .set_skill_binding(
                key,
                None,
                BindingChange::Attach(Attachment {
                    pinned_version: None,
                    position: 0,
                    activation,
                    globs: Vec::new(),
                    languages: Vec::new(),
                }),
            )
            .await
            .unwrap_or_else(|err| panic!("attach {name}: {err}"));
    }
    made
}

/// A resolved step list as `(name, level, activation, active)`.
fn names(bound: &[BoundMcp]) -> Vec<(&str, BindingLevel, Activation, bool)> {
    bound
        .iter()
        .map(|row| {
            (
                row.server.name.as_str(),
                row.level,
                row.activation,
                row.active,
            )
        })
        .collect()
}

/// One row of the resolution table: what is reached, and the step list it resolves to.
type Expected = Vec<(&'static str, BindingLevel, Activation, bool)>;

/// D5 on both stores: per server the most specific attachment wins whole, an `off` winner is
/// returned inactive, an unattached server and a level the scope does not reach never appear, and
/// the list is ordered by name bytes. The fixed table spans all five levels.
#[tokio::test(flavor = "multi_thread")]
async fn bound_mcp_servers_resolves_all_five_levels() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let requests: Vec<NewMcpServer> = ["alpha", "beta", "gamma", "delta", "epsilon"]
        .into_iter()
        .map(stdio)
        .collect();
    let mem = MemStore::demo();
    let pg_made = fill(&db.store, &requests).await;
    let mem_made = fill(&mem, &requests).await;

    use Activation::{Always, Off};
    use BindingLevel::{Global, ItemKind, Phase, Project, Workspace};
    let full = ResolveScope {
        workspace: Some(ids::WORKSPACE_PLATFORM),
        project: ids::PROJECT_HTUI,
        item_kind: Some(ids::KIND_HTUI_FEAT),
        phase: Some(ids::PHASE_HTUI_IMPLEMENT),
    };
    let table: [(&str, ResolveScope, Expected); 5] = [
        (
            "every level reached",
            full,
            vec![
                ("alpha", Workspace, Off, false),
                ("beta", Phase, Off, false),
                ("delta", Phase, Always, true),
                ("gamma", ItemKind, Always, true),
            ],
        ),
        (
            "no workspace, item kind or phase",
            ResolveScope::project(ids::PROJECT_HTUI, None),
            vec![
                ("alpha", Global, Always, true),
                ("beta", Project, Always, true),
                ("gamma", Global, Off, false),
            ],
        ),
        (
            "the phase but no item kind",
            ResolveScope::project(ids::PROJECT_HTUI, Some(ids::PHASE_HTUI_IMPLEMENT)),
            vec![
                ("alpha", Global, Always, true),
                ("beta", Phase, Off, false),
                ("delta", Phase, Always, true),
                ("gamma", Global, Off, false),
            ],
        ),
        (
            "another project: only the global rows",
            ResolveScope::project(ids::PROJECT_AGY, None),
            vec![
                ("alpha", Global, Always, true),
                ("gamma", Global, Off, false),
            ],
        ),
        (
            "a workspace over another project",
            ResolveScope {
                workspace: Some(ids::WORKSPACE_PLATFORM),
                project: ids::PROJECT_AGY,
                item_kind: None,
                phase: None,
            },
            vec![
                ("alpha", Workspace, Off, false),
                ("gamma", Global, Off, false),
            ],
        ),
    ];
    for (what, scope, expected) in table {
        let pg = db.store.bound_mcp_servers(scope).await.expect("PgStore");
        let mem_rows = mem.bound_mcp_servers(scope).await.expect("MemStore");
        assert_eq!(names(&pg), expected, "PgStore resolves {what}");
        assert_eq!(names(&mem_rows), expected, "MemStore resolves {what}");
        // The records come back whole and equal to what each store was given.
        for (rows, made) in [(&pg, &pg_made), (&mem_rows, &mem_made)] {
            for row in rows {
                let stored = made
                    .iter()
                    .find(|server| server.id == row.server.id)
                    .expect("a resolved server was created");
                assert_eq!(&row.server, stored, "{what}: the record as stored");
            }
        }
    }
}

/// A server's two JSON columns come back in the order they went in, secrets as references, and
/// the url and the two text arrays survive; Postgres and Memory read the same row.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_round_trips_in_order_on_both_stores() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let mem = MemStore::demo();
    let mut remote = stdio("remote");
    remote.transport = McpTransport::Http;
    remote.command = String::new();
    remote.args = Vec::new();
    remote.url = Some("https://mcp.example.test/rpc".to_owned());
    remote.env = Vec::new();
    remote.headers = vec![
        ("X-Zeta-Mode".to_owned(), EnvValue::Literal("on".to_owned())),
        (
            "Authorization".to_owned(),
            EnvValue::Secret("remote-auth".to_owned()),
        ),
    ];
    for request in [stdio("local"), remote] {
        let pg = db
            .store
            .create_mcp_server(request.clone())
            .await
            .expect("Pg create");
        let mem_row = mem
            .create_mcp_server(request.clone())
            .await
            .expect("Mem create");
        assert_eq!(
            (&pg.env, &pg.headers, &pg.args, &pg.url, &pg.allow, &pg.deny),
            (
                &request.env,
                &request.headers,
                &request.args,
                &request.url,
                &request.allow,
                &request.deny,
            ),
            "Postgres keeps the order and the references"
        );
        assert_eq!(
            (&pg.env, &pg.headers, &pg.url),
            (&mem_row.env, &mem_row.headers, &mem_row.url),
            "both stores read one row"
        );
    }
    let listed = db.store.mcp_servers().await.expect("Pg list");
    assert_eq!(
        listed
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["local", "remote"],
        "name byte order"
    );
}

/// The constraint a refused insert names.
fn constraint(result: Result<PgQueryResult, sqlx::Error>) -> String {
    let err = result.expect_err("the insert must be refused");
    err.as_database_error()
        .and_then(|db| db.constraint())
        .unwrap_or_else(|| panic!("a constraint violation, got {err}"))
        .to_owned()
}

/// Plants an `mcp_server` row, raw, so the constraint answers instead of the writer's rule.
async fn plant_server(
    pool: &PgPool,
    transport: &str,
    command: &str,
    url: Option<&str>,
    args: &[&str],
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO mcp_server (id, name, transport, command, url, args, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(McpServerId::new().as_uuid())
    .bind(format!("raw-{}", Uuid::now_v7().simple()))
    .bind(transport)
    .bind(command)
    .bind(url)
    .bind(
        args.iter()
            .map(|arg| (*arg).to_owned())
            .collect::<Vec<String>>(),
    )
    .bind(ids::USER.as_uuid())
    .execute(pool)
    .await
}

/// Inserts a raw global `toolbox_binding` row of `kind`.
async fn plant_binding(
    pool: &PgPool,
    kind: &str,
    ref_id: Uuid,
    pinned: Option<i32>,
    activation: &str,
    globs: &[&str],
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO toolbox_binding (kind, ref_id, pinned_version, activation, globs) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(kind)
    .bind(ref_id)
    .bind(pinned)
    .bind(activation)
    .bind(
        globs
            .iter()
            .map(|glob| (*glob).to_owned())
            .collect::<Vec<String>>(),
    )
    .execute(pool)
    .await
}

/// The CHECKs refuse what the model's rules rule out, each under its own name (`0042`): a
/// transport outside the two, a shape that mixes the transports, a third `kind`, and an
/// `mcp_server` attachment that pins, globs or has `glob` activation; the shapes that are
/// allowed insert.
#[tokio::test(flavor = "multi_thread")]
async fn the_checks_refuse_what_the_rules_refuse() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let pool = &db.pool;
    let none: &[&str] = &[];
    for (what, transport, command, url, args, name) in [
        // A third transport breaks `mcp_server_transport_check` and `mcp_server_shape` alike;
        // Postgres names the shape check first, so that is the one pinned here.
        (
            "a third transport",
            "ws",
            "x",
            None,
            none,
            "mcp_server_shape",
        ),
        (
            "a stdio server with no command",
            "stdio",
            "",
            None,
            none,
            "mcp_server_shape",
        ),
        (
            "a stdio server with a url",
            "stdio",
            "x",
            Some("https://x.test"),
            none,
            "mcp_server_shape",
        ),
        (
            "an http server with no url",
            "http",
            "",
            None,
            none,
            "mcp_server_shape",
        ),
        (
            "an http server with a command",
            "http",
            "x",
            Some("https://x.test"),
            none,
            "mcp_server_shape",
        ),
        (
            "an http server with args",
            "http",
            "",
            Some("https://x.test"),
            &["--a"][..],
            "mcp_server_shape",
        ),
    ] {
        assert_eq!(
            constraint(plant_server(pool, transport, command, url, args).await),
            name,
            "{what}"
        );
    }
    plant_server(pool, "stdio", "mcp-ok", None, &["--stdio"])
        .await
        .expect("a stdio shape inserts");
    plant_server(pool, "http", "", Some("https://x.test"), &[])
        .await
        .expect("an http shape inserts");

    let server = McpServerId::new().as_uuid();
    let glob: &[&str] = &["**/*.rs"];
    for (what, kind, pinned, activation, globs, name) in [
        (
            "a third kind",
            "persona",
            None,
            "always",
            none,
            "toolbox_binding_kind_check",
        ),
        (
            "a pinned server",
            "mcp_server",
            Some(1),
            "always",
            none,
            "toolbox_binding_mcp_shape",
        ),
        (
            "a glob server",
            "mcp_server",
            None,
            "glob",
            glob,
            "toolbox_binding_mcp_shape",
        ),
        (
            "globs under always",
            "mcp_server",
            None,
            "always",
            glob,
            "toolbox_binding_mcp_shape",
        ),
    ] {
        assert_eq!(
            constraint(plant_binding(pool, kind, server, pinned, activation, globs).await),
            name,
            "{what}"
        );
    }
    plant_binding(pool, "mcp_server", server, None, "off", none)
        .await
        .expect("an off server inserts");
    // A skill row keeps its pins and globs: the shape check is for the new kind only.
    plant_binding(
        pool,
        "skill",
        McpServerId::new().as_uuid(),
        Some(2),
        "glob",
        glob,
    )
    .await
    .expect("a pinned glob skill row inserts");
}
