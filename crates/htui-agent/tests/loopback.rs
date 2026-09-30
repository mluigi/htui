//! MOD-22 T1: the loopback paste-back (D264–D268, D273, D276, D285).
//!
//! Reading the advertised redirect, validating a paste against it, and delivering it to a
//! hand-rolled listener on `127.0.0.1:0`. Not `cfg(unix)`: there is no process here, only sockets.
//!
//! No assertion here prints a real code. The pasted `code` and `state` are the sentinels
//! [`CODE`] and [`STATE`], and every redaction case asserts their absence.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use htui_agent::auth::loopback::{
    Advertised, DeliverError, DeliverLimits, EXCERPT_WIDTH, ListenerReply, PASTE_MAX, PasteError,
    RESPONSE_CAP, RedirectUrl, deliver, precheck, validate,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The pasted authorization code every case uses.
const CODE: &str = "CODE-SENTINEL-4f1c";
/// The link's and the paste's `state`.
const STATE: &str = "STATE-SENTINEL-9a2e";

/// An authorisation link whose `redirect_uri` is `http://{host_port}/`, percent-encoded.
fn link(host_port: &str, state: &str) -> String {
    format!(
        "https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F{host_port}%2F\
         &state={state}"
    )
}

/// What a link to `127.0.0.1:{port}` with `state=STATE` advertises.
fn advertised(port: u16) -> Advertised {
    Advertised::from_auth_url(&link(&format!("127.0.0.1:{port}"), STATE))
        .expect("a loopback link advertises its redirect")
}

fn paste(text: &str) -> RedirectUrl {
    RedirectUrl::new(text.to_owned())
}

/// The address a browser on another machine could not open.
fn good(port: u16) -> String {
    format!("http://127.0.0.1:{port}/?code={CODE}&state={STATE}")
}

/// Reads one request head, up to and including the blank line.
async fn read_head(stream: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await.expect("the request arrives");
        if n == 0 {
            break;
        }
        head.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// A listener on `127.0.0.1:0` that accepts exactly one connection, reads its head, writes
/// `reply`, shuts down, and returns the head.
async fn answering(reply: Vec<u8>) -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let head = read_head(&mut stream).await;
        stream.write_all(&reply).await.expect("write the answer");
        stream.shutdown().await.expect("shutdown");
        head
    });
    (port, task)
}

/// A listener that accepts one connection, reads its head, runs `then` on the stream.
async fn listening<F, Fut>(then: F) -> (u16, JoinHandle<()>)
where
    F: FnOnce(TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let _head = read_head(&mut stream).await;
        then(stream).await;
    });
    (port, task)
}

/// Validates [`good`] against [`advertised`] and delivers it under `limits`.
async fn deliver_good(port: u16, limits: DeliverLimits) -> Result<ListenerReply, DeliverError> {
    let delivery = validate(&paste(&good(port)), &advertised(port)).expect("a valid paste");
    deliver(delivery, limits).await
}

fn assert_no_sentinel(text: &str) {
    assert!(!text.contains(CODE), "the code sentinel leaked");
    assert!(!text.contains(STATE), "the state sentinel leaked");
}

// ---- Advertised ------------------------------------------------------------------------------

#[test]
fn advertised_is_read_from_a_percent_encoded_redirect_uri() {
    let found = Advertised::from_auth_url(
        "https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F\
         &state=S1",
    )
    .expect("a loopback redirect");
    assert_eq!(found.host(), "127.0.0.1");
    assert_eq!(found.port(), 39879);
    assert_eq!(found.path(), "/");
    assert_eq!(found.target(), "127.0.0.1:39879");

    let s1 = paste("http://127.0.0.1:39879/?code=c1&state=S1");
    assert!(validate(&s1, &found).is_ok());
    let s2 = paste("http://127.0.0.1:39879/?code=c1&state=S2");
    assert_eq!(
        validate(&s2, &found).expect_err("another state"),
        PasteError::StaleState
    );
}

#[test]
fn advertised_accepts_localhost_and_the_ipv6_loopback() {
    for (host_port, host) in [
        ("LOCALHOST:5000", "localhost"),
        ("localhost:5000", "localhost"),
        ("127.0.0.2:5000", "127.0.0.2"),
        ("%5B%3A%3A1%5D:5000", "[::1]"),
    ] {
        let found = Advertised::from_auth_url(&link(host_port, "s")).expect(host_port);
        assert_eq!(found.host(), host, "{host_port}");
        assert_eq!(found.port(), 5000, "{host_port}");
        assert_eq!(found.target(), format!("{host}:5000"));
    }

    let no_port = Advertised::from_auth_url(&link("127.0.0.1", "s")).expect("no port");
    assert_eq!(no_port.port(), 80);

    let callback = Advertised::from_auth_url(
        "https://auth.example.invalid/o?redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2Foauth2callback",
    )
    .expect("a path");
    assert_eq!(callback.path(), "/oauth2callback");
}

#[test]
fn advertised_is_none_for_https_a_lan_host_a_name_no_redirect_uri_or_no_url() {
    for link in [
        "https://auth.example.invalid/o?redirect_uri=https%3A%2F%2F127.0.0.1%3A5000%2F&state=s",
        "https://auth.example.invalid/o?redirect_uri=http%3A%2F%2F192.168.1.2%3A5000%2F&state=s",
        "https://auth.example.invalid/o?redirect_uri=http%3A%2F%2Fexample.invalid%3A5000%2F",
        "https://auth.example.invalid/o?redirect_uri=http%3A%2F%2F%5B%3A%3Affff%3A127.0.0.1%5D%3A5000%2F",
        "https://auth.example.invalid/o?client_id=c&state=s",
        "https://auth.example.invalid/o?redirect_uri=not%20a%20url",
        "not a url",
    ] {
        assert_eq!(Advertised::from_auth_url(link), None, "{link}");
    }
}

#[test]
fn the_newest_link_is_not_this_modules_business() {
    let first = link("127.0.0.1:39879", "s1");
    let second = link("127.0.0.1:50651", "s2");
    let a = Advertised::from_auth_url(&first).expect("first");
    let b = Advertised::from_auth_url(&second).expect("second");
    assert_eq!(a.port(), 39879);
    assert_eq!(b.port(), 50651);
    assert_eq!(
        Advertised::from_auth_url(&first).expect("again").port(),
        39879
    );
    assert_eq!(Advertised::from_auth_url(&first), Some(a));
}

// ---- validate --------------------------------------------------------------------------------

#[test]
fn a_valid_paste_becomes_a_delivery_for_the_advertised_target() {
    let delivery = validate(
        &paste(&format!("  \t{}\n ", good(39879))),
        &advertised(39879),
    )
    .expect("a valid paste");
    assert_eq!(delivery.target(), "127.0.0.1:39879");
}

#[test]
fn a_paste_without_a_scheme_is_read_as_http() {
    let text = format!("127.0.0.1:39879/?code={CODE}&state={STATE}");
    let delivery = validate(&paste(&text), &advertised(39879)).expect("read as http");
    assert_eq!(delivery.target(), "127.0.0.1:39879");
}

/// Review L-6: a scheme is detected only at the start. A scheme-less paste whose query carries a
/// `://` (a `next=http://…` echoed by a vendor) is still the `http` address it looks like.
#[test]
fn a_scheme_less_paste_whose_query_carries_a_scheme_is_still_read_as_http() {
    let text = format!("127.0.0.1:39879/?code={CODE}&state={STATE}&next=http://example.com/");
    let delivery = validate(&paste(&text), &advertised(39879)).expect("read as http");
    assert_eq!(delivery.target(), "127.0.0.1:39879");

    let at = Advertised::from_auth_url(&link("localhost:39879", STATE)).expect("link");
    let text = format!("localhost:39879/?code={CODE}&state={STATE}&back=https://x.invalid/");
    let delivery = validate(&paste(&text), &at).expect("`localhost:` is a host, not a scheme");
    assert_eq!(delivery.target(), "localhost:39879");
}

/// Review L-6: an `error=` with nothing usable in it is refused with a sentence of its own, not
/// with a pair of empty backticks.
#[test]
fn an_empty_error_value_is_refused_without_empty_backticks() {
    let at = advertised(39879);
    for query in [
        format!("error=&state={STATE}"),
        format!("error=%3C%3E&code={CODE}&state={STATE}"),
    ] {
        let err = validate(&paste(&format!("http://127.0.0.1:39879/?{query}")), &at)
            .expect_err("an error redirect");
        assert_eq!(err, PasteError::BrowserErrorUnnamed, "{query}");
        let sentence = err.to_string();
        assert!(!sentence.contains("``"), "{sentence}");
        assert!(sentence.contains("the login was not granted"), "{sentence}");
    }
}

/// Review L-6 (maintainer OQ-3 keeps `state` required): when the login's own link carried no
/// state either, the refusal says that is why the redirect cannot be checked, rather than asking
/// for a state the address never had.
#[test]
fn a_stateless_paste_for_a_stateless_link_says_the_link_carried_no_state() {
    let stateless = Advertised::from_auth_url(
        "https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F",
    )
    .expect("a link without state");
    let err = validate(
        &paste(&format!("http://127.0.0.1:39879/?code={CODE}")),
        &stateless,
    )
    .expect_err("state stays required");
    assert_eq!(err, PasteError::UnverifiableWithoutState);
    let sentence = err.to_string();
    assert!(
        sentence.contains("the login link carried no state"),
        "{sentence}"
    );
    assert_no_sentinel(&sentence);

    // With a state on the link, a stateless paste is still just missing its state.
    assert_eq!(
        validate(
            &paste(&format!("http://127.0.0.1:39879/?code={CODE}")),
            &advertised(39879)
        )
        .expect_err("no state"),
        PasteError::MissingState
    );
}

/// Review L-8: the pane's courtesy check reads the host and the port and nothing else — no URL
/// parse — and refuses with `validate`'s own sentences.
#[test]
fn precheck_reads_the_host_and_the_port_only() {
    let at = advertised(39879);
    for fine in [
        good(39879),
        format!("  127.0.0.1:39879/?code={CODE}&state=earlier\n"),
        "http://127.0.0.1:39879".to_owned(),
        "HTTP://127.0.0.1:39879/x#y".to_owned(),
        format!("http://127.0.0.1:39879?state={STATE}"),
        "127.0.0.1:39879/?next=http://example.com:1/".to_owned(),
    ] {
        assert_eq!(precheck(&paste(&fine), &at), Ok(()), "{fine}");
    }
    let refused = |text: &str, at: &Advertised| precheck(&paste(text), at).expect_err(text);
    assert_eq!(refused(" ", &at), PasteError::Empty);
    assert_eq!(
        refused(&"a".repeat(PASTE_MAX + 1), &at),
        PasteError::TooLong
    );
    assert_eq!(
        refused(&good(50651), &at),
        PasteError::WrongPort {
            pasted: 50651,
            advertised: 39879
        }
    );
    assert_eq!(
        refused("http://127.0.0.1/?code=c", &at),
        PasteError::WrongPort {
            pasted: 80,
            advertised: 39879
        }
    );
    assert_eq!(
        refused("http://localhost:39879/", &at),
        PasteError::WrongHost {
            advertised: "127.0.0.1:39879".to_owned()
        }
    );

    let v6 = Advertised::from_auth_url(&link("[::1]:39879", STATE)).expect("v6");
    assert_eq!(precheck(&paste("http://[::1]:39879/?code=c"), &v6), Ok(()));
    assert_eq!(
        refused("http://[::1]:39880/", &v6),
        PasteError::WrongPort {
            pasted: 39880,
            advertised: 39879
        }
    );
    let named = Advertised::from_auth_url(&link("localhost:39879", STATE)).expect("localhost");
    assert_eq!(precheck(&paste("LocalHost:39879/?x"), &named), Ok(()));
}

/// Review R2-L2: extra or backward slashes before the host are read the way `validate`'s URL
/// parser reads them, so the pane's courtesy check refuses exactly what the worker would, and
/// lets through exactly what it would take.
#[test]
fn precheck_and_validate_agree_on_slashes_before_the_host() {
    let at = advertised(39879);
    let query = format!("?code={CODE}&state={STATE}");
    for (spelling, accepted) in [
        ("//127.0.0.1:39879/", true),
        ("\\\\127.0.0.1:39879/", true),
        ("/\\127.0.0.1:39879/", true),
        ("http:///127.0.0.1:39879/", true),
        ("http://\\127.0.0.1:39879/", true),
        ("HTTP:////127.0.0.1:39879\\", true),
        ("//127.0.0.1:1/", false),
        ("http:///127.0.0.2:39879/", false),
        ("http:/127.0.0.1:39879/", false),
        ("http:\\\\127.0.0.1:39879\\", false),
    ] {
        let text = format!("{spelling}{query}");
        let worker = validate(&paste(&text), &at).map(|_| ());
        let pane = precheck(&paste(&text), &at);
        assert_eq!(
            worker.is_ok(),
            accepted,
            "{spelling}: validate said {worker:?}"
        );
        assert_eq!(pane, worker, "{spelling}: precheck and validate disagree");
    }
}

#[test]
fn wrong_port_names_both_ports_and_nothing_else() {
    let err = validate(&paste(&good(50651)), &advertised(39879)).expect_err("another port");
    assert_eq!(
        err.to_string(),
        "the address is for port 50651; this login is listening on 39879"
    );
}

#[test]
fn stale_state_is_refused() {
    let text = format!("http://127.0.0.1:39879/?code={CODE}&state=earlier");
    assert_eq!(
        validate(&paste(&text), &advertised(39879)).expect_err("stale"),
        PasteError::StaleState
    );

    let stateless = Advertised::from_auth_url(
        "https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F",
    )
    .expect("a link without state");
    assert!(validate(&paste(&text), &stateless).is_ok());
    let empty = Advertised::from_auth_url(&link("127.0.0.1:39879", "")).expect("an empty state");
    assert!(validate(&paste(&text), &empty).is_ok());
}

#[test]
fn an_error_redirect_is_named_by_its_error_value() {
    let at = advertised(39879);
    let refused = |query: &str| {
        validate(&paste(&format!("http://127.0.0.1:39879/?{query}")), &at).expect_err(query)
    };
    assert_eq!(
        refused(&format!("error=access_denied&state={STATE}")),
        PasteError::BrowserError {
            error: "access_denied".to_owned()
        }
    );
    assert_eq!(
        refused("error=a%3Cb%3Ec"),
        PasteError::BrowserError {
            error: "abc".to_owned()
        }
    );
    assert_eq!(
        refused(&format!("error={}", "x".repeat(100))),
        PasteError::BrowserError {
            error: "x".repeat(64)
        }
    );
}

#[test]
fn each_rule_refuses_with_its_own_variant() {
    let at = advertised(39879);
    let q = format!("code={CODE}&state={STATE}");
    let rows: Vec<(String, PasteError)> = vec![
        (String::new(), PasteError::Empty),
        (" \n\t ".to_owned(), PasteError::Empty),
        ("a".repeat(PASTE_MAX + 1), PasteError::TooLong),
        ("http://[oops/?x".to_owned(), PasteError::NotAUrl),
        (format!("https://127.0.0.1:39879/?{q}"), PasteError::NotHttp),
        (
            format!("http://u:p@127.0.0.1:39879/?{q}"),
            PasteError::HasUserinfo,
        ),
        (
            format!("http://127.0.0.2:39879/?{q}"),
            PasteError::WrongHost {
                advertised: "127.0.0.1:39879".to_owned(),
            },
        ),
        (
            format!("http://localhost:39879/?{q}"),
            PasteError::WrongHost {
                advertised: "127.0.0.1:39879".to_owned(),
            },
        ),
        (
            format!("http://127.0.0.1:1/?{q}"),
            PasteError::WrongPort {
                pasted: 1,
                advertised: 39879,
            },
        ),
        (
            format!("http://127.0.0.1/?{q}"),
            PasteError::WrongPort {
                pasted: 80,
                advertised: 39879,
            },
        ),
        (
            format!("http://127.0.0.1:39879/x?{q}"),
            PasteError::WrongPath {
                advertised: "127.0.0.1:39879/".to_owned(),
            },
        ),
        (
            format!("http://127.0.0.1:39879/?error=access_denied&{q}"),
            PasteError::BrowserError {
                error: "access_denied".to_owned(),
            },
        ),
        (
            format!("http://127.0.0.1:39879/?state={STATE}"),
            PasteError::MissingCode,
        ),
        (
            format!("http://127.0.0.1:39879/?code=&state={STATE}"),
            PasteError::MissingCode,
        ),
        (
            format!("http://127.0.0.1:39879/?code=a&code=b&state={STATE}"),
            PasteError::RepeatedParameter("code"),
        ),
        (
            format!("http://127.0.0.1:39879/?code={CODE}"),
            PasteError::MissingState,
        ),
        (
            format!("http://127.0.0.1:39879/?code={CODE}&state="),
            PasteError::MissingState,
        ),
        (
            format!("http://127.0.0.1:39879/?{q}&state={STATE}"),
            PasteError::RepeatedParameter("state"),
        ),
        (
            format!("http://127.0.0.1:39879/?code={CODE}&state=other"),
            PasteError::StaleState,
        ),
    ];
    for (i, (text, expected)) in rows.into_iter().enumerate() {
        let got = validate(&paste(&text), &at).expect_err("refused");
        assert_eq!(got, expected, "row {i}");
    }

    // Review L-6: the sentence is derived from the constant, not a copy of it.
    let too_long = PasteError::TooLong.to_string();
    assert_eq!(
        too_long,
        format!("the pasted text is longer than {PASTE_MAX} bytes; paste the address bar only")
    );
}

#[test]
fn every_paste_error_sentence_is_free_of_pasted_text() {
    let at = advertised(39879);
    let other_state = Advertised::from_auth_url(&link("127.0.0.1:39879", "S1")).expect("S1");
    let stateless = Advertised::from_auth_url(
        "https://auth.example.invalid/o?client_id=c&redirect_uri=http%3A%2F%2F127.0.0.1%3A39879%2F",
    )
    .expect("a link without state");
    let q = format!("code={CODE}&state={STATE}");
    let cases: Vec<(String, &Advertised)> = vec![
        (" ".to_owned(), &at),
        (format!("{}{}", good(39879), "a".repeat(PASTE_MAX)), &at),
        (format!("http://[{CODE}/?state={STATE}"), &at),
        (format!("https://127.0.0.1:39879/?{q}"), &at),
        (format!("http://{CODE}:{STATE}@127.0.0.1:39879/?{q}"), &at),
        (format!("http://127.0.0.2:39879/?{q}"), &at),
        (format!("http://127.0.0.1:50651/?{q}"), &at),
        (format!("http://127.0.0.1:39879/{CODE}?{q}"), &at),
        (
            format!("http://127.0.0.1:39879/?error=access_denied&{q}"),
            &at,
        ),
        (format!("http://127.0.0.1:39879/?state={STATE}"), &at),
        (format!("http://127.0.0.1:39879/?code={CODE}"), &at),
        (format!("http://127.0.0.1:39879/?{q}&code={CODE}"), &at),
        (format!("http://127.0.0.1:39879/?{q}"), &other_state),
        (format!("http://127.0.0.1:39879/?error=&{q}"), &at),
        (format!("http://127.0.0.1:39879/?code={CODE}"), &stateless),
    ];
    let mut variants = HashSet::new();
    for (text, advertised) in cases {
        let err = validate(&paste(&text), advertised).expect_err("refused");
        variants.insert(std::mem::discriminant(&err));
        assert_no_sentinel(&err.to_string());
        assert_no_sentinel(&format!("{err:?}"));
    }
    // RepeatedParameter("state") shares its discriminant with "code"; both are covered above and
    // in `each_rule_refuses_with_its_own_variant`.
    assert_eq!(variants.len(), 15, "every PasteError variant is exercised");
    let twice = format!("http://127.0.0.1:39879/?{q}&state={STATE}");
    let err = validate(&paste(&twice), &at).expect_err("state twice");
    assert_no_sentinel(&err.to_string());
    assert_no_sentinel(&format!("{err:?}"));
}

// ---- redaction -------------------------------------------------------------------------------

#[test]
fn redirect_url_debug_is_redacted() {
    let url = paste(&good(39879));
    assert_eq!(format!("{url:?}"), "RedirectUrl(<redacted>)");
    let copy = url.clone();
    assert_eq!(format!("{copy:?}"), "RedirectUrl(<redacted>)");
    assert_eq!(format!("{url:#?}"), "RedirectUrl(<redacted>)");
}

#[test]
fn delivery_and_advertised_debug_print_host_and_port_only() {
    let at = advertised(39879);
    let shown = format!("{at:?}");
    assert_no_sentinel(&shown);
    assert!(shown.contains("<redacted>"), "{shown}");
    assert!(shown.contains("127.0.0.1"), "{shown}");
    assert!(shown.contains("39879"), "{shown}");

    let delivery = validate(&paste(&good(39879)), &at).expect("valid");
    let shown = format!("{delivery:?}");
    assert_no_sentinel(&shown);
    assert!(shown.contains("127.0.0.1:39879"), "{shown}");
    assert_no_sentinel(&format!("{delivery:#?}"));
}

// ---- deliver ---------------------------------------------------------------------------------

#[tokio::test]
async fn deliver_sends_one_get_with_the_pasted_path_query_and_the_advertised_host() {
    let (port, head) = answering(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec()).await;
    let text = format!("  http://127.0.0.1:{port}/?code={CODE}&state={STATE}#frag \n");
    let delivery = validate(&paste(&text), &advertised(port)).expect("valid");
    let reply = deliver(delivery, DeliverLimits::default())
        .await
        .expect("an answer");
    assert_eq!(reply.status, 200);
    assert_eq!(
        head.await.expect("the listener"),
        format!(
            "GET /?code=CODE-SENTINEL-4f1c&state=STATE-SENTINEL-9a2e HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             User-Agent: htui\r\n\
             Accept: text/html, text/plain, */*\r\n\
             Connection: close\r\n\r\n"
        )
    );
}

#[tokio::test]
async fn deliver_reports_the_status_and_the_title_of_an_html_answer() {
    let (port, _head) = answering(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
          <html><head><TITLE>Signed  in</TITLE></head><body><p>You may close this tab.</p>\
          </body></html>"
            .to_vec(),
    )
    .await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert_eq!(reply.target, format!("127.0.0.1:{port}"));
    assert_eq!(reply.status, 200);
    assert_eq!(reply.reason, "OK");
    assert_eq!(reply.said.as_deref(), Some("Signed in"));
    assert_eq!(reply.location_host, None);
    assert_eq!(
        reply.summary(),
        format!("127.0.0.1:{port} answered 200 OK: \"Signed in\"")
    );
}

#[tokio::test]
async fn deliver_reports_the_first_text_line_when_there_is_no_title() {
    let (port, _head) = answering(
        b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\n\n\n  <b>invalid</b> state\nmore"
            .to_vec(),
    )
    .await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert_eq!(reply.status, 400);
    assert_eq!(reply.reason, "Bad Request");
    assert_eq!(reply.said.as_deref(), Some("invalid state"));
}

#[tokio::test]
async fn deliver_decodes_a_chunked_body() {
    let (port, _head) = answering(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
          a\r\n<title>Sig\r\n3\r\nned\r\nb\r\n in</title>\r\n0\r\n\r\n"
            .to_vec(),
    )
    .await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert_eq!(reply.said.as_deref(), Some("Signed in"));
}

#[tokio::test]
async fn deliver_blanks_an_echoed_code_and_state_in_the_excerpt() {
    let raw = "CODE-SENTINEL%2D4f1c";
    let body = format!("<title>got {raw} and {CODE} for {STATE}</title>");
    let answer = format!(
        "HTTP/1.1 400 Bad {CODE}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let (port, head) = answering(answer.into_bytes()).await;
    let text = format!("http://127.0.0.1:{port}/?code={raw}&state={STATE}");
    let delivery = validate(&paste(&text), &advertised(port)).expect("valid");
    let reply = deliver(delivery, DeliverLimits::default())
        .await
        .expect("an answer");
    assert!(
        head.await.expect("the listener").contains(raw),
        "the raw query is sent as pasted"
    );
    let said = reply.said.clone().expect("an excerpt");
    let summary = reply.summary();
    for shown in [&said, &summary, &reply.reason] {
        assert!(!shown.contains(raw), "the raw code leaked");
        assert_no_sentinel(shown);
    }
    assert!(said.contains('…'), "the echo is blanked, not dropped");
    assert!(summary.contains('…'));
}

/// Delivers [`good`] to a listener whose `<title>` is `title` and returns what it said.
async fn said_of_title(title: &str) -> Option<String> {
    let body = format!("<title>{title}</title>");
    let answer = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let (port, _head) = answering(answer.into_bytes()).await;
    deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer")
        .said
}

/// Review L-5: an echo is blanked however the listener spells it — percent-encoded in either
/// case of hex, in whole or in part, and as HTML character references, decimal, hex or named.
#[tokio::test]
async fn deliver_blanks_percent_encoded_and_html_escaped_echoes() {
    let every_byte = |text: &str, upper: bool| -> String {
        text.bytes()
            .map(|b| {
                if upper {
                    format!("%{b:02X}")
                } else {
                    format!("%{b:02x}")
                }
            })
            .collect()
    };
    let decimal: String = CODE
        .chars()
        .map(|c| format!("&#{};", u32::from(c)))
        .collect();
    let hex: String = CODE
        .chars()
        .map(|c| format!("&#X{:x};", u32::from(c)))
        .collect();
    let echoes = [
        CODE.replace('-', "%2d"),
        CODE.replace('-', "%2D"),
        every_byte(CODE, false),
        every_byte(CODE, true),
        every_byte(STATE, false),
        decimal,
        hex,
        CODE.replacen('C', "&#67;", 1).replace('-', "&#x2d;"),
        STATE.replacen('S', "&#83;", 1),
    ];
    for echo in echoes {
        let said = said_of_title(&format!("got {echo}!")).await;
        assert_eq!(said.as_deref(), Some("got …!"), "{echo}");
    }
}

/// Review L-5: an answer cut at [`RESPONSE_CAP`] part-way through an echoed code leaves no prefix
/// of it standing, spelled as is or percent-encoded, even when the cut splits an escape.
#[tokio::test]
async fn deliver_blanks_a_code_cut_at_the_response_cap() {
    let head = "HTTP/1.1 200 OK\r\n\r\n";
    let cut_escaped = CODE.replace('-', "%2D");
    let tails = [
        CODE[..9].to_owned(),
        cut_escaped[..6].to_owned(),
        cut_escaped[..5].to_owned(),
    ];
    for tail in tails {
        let mut answer = head.to_owned();
        answer.push_str(&"\n".repeat(RESPONSE_CAP - head.len() - tail.len()));
        answer.push_str(&tail);
        assert_eq!(answer.len(), RESPONSE_CAP);
        answer.push_str("-the-rest-of-it\n");
        let (port, _server) = listening(move |mut stream| async move {
            let _ = stream.write_all(answer.as_bytes()).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(stream);
        })
        .await;
        let reply = deliver_good(port, DeliverLimits::default())
            .await
            .expect("an answer");
        let said = reply.said.expect("the cut line is the first non-blank one");
        assert!(!said.contains("CODE"), "{tail} left {said:?}");
        assert_eq!(said, "…", "{tail}");
    }
}

/// Review L-5: control characters and Unicode format characters — the bidi overrides and
/// isolates, the marks, zero-width spaces, the BOM and the soft hyphen — are stripped from the
/// excerpt and the reason, so a listener cannot reorder or hide what the note line reads.
#[tokio::test]
async fn deliver_strips_bidi_and_other_format_characters() {
    let said = said_of_title(
        "ok\u{202e}evil\u{2066}x\u{2069}\u{200b}y\u{200e}\u{200f}\u{feff}z\u{ad}\u{7}!",
    )
    .await;
    assert_eq!(said.as_deref(), Some("okevilxyz!"));

    let (port, _head) = answering(
        "HTTP/1.1 200 O\u{202a}K\u{202c}\u{61c}\r\nContent-Length: 0\r\n\r\n"
            .as_bytes()
            .to_vec(),
    )
    .await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert_eq!(reply.reason, "OK");
}

#[tokio::test]
async fn deliver_cuts_the_excerpt_at_eighty_columns() {
    let answer = format!("HTTP/1.1 200 OK\r\n\r\n<title>{}</title>", "x".repeat(300));
    let (port, _head) = answering(answer.into_bytes()).await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    let said = reply.said.expect("an excerpt");
    assert_eq!(said.chars().count(), EXCERPT_WIDTH);
    assert_eq!(said.chars().last(), Some('…'));
    assert!(said.starts_with("xxxx"));
}

#[tokio::test]
async fn deliver_does_not_follow_a_redirect_and_names_the_location_host() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let answer = format!(
        "HTTP/1.1 302 Found\r\nLocation: https://example.com/done?code={CODE}\r\n\
         Content-Length: 0\r\n\r\n"
    );
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let _head = read_head(&mut stream).await;
        stream.write_all(answer.as_bytes()).await.expect("write");
        stream.shutdown().await.expect("shutdown");
        tokio::time::timeout(Duration::from_millis(200), listener.accept())
            .await
            .is_err()
    });
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert_eq!(reply.status, 302);
    assert_eq!(reply.location_host.as_deref(), Some("example.com"));
    let summary = reply.summary();
    assert!(
        summary.ends_with(", redirecting to example.com"),
        "{summary}"
    );
    assert_no_sentinel(&summary);
    assert_no_sentinel(&format!("{reply:?}"));
    assert!(
        server.await.expect("the listener"),
        "a second connection was made"
    );
}

#[tokio::test]
async fn deliver_to_a_closed_port_is_nothing_listening() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.local_addr().expect("addr").port()
    };
    let err = deliver_good(port, DeliverLimits::default())
        .await
        .expect_err("nothing listens");
    assert_eq!(
        err,
        DeliverError::NothingListening {
            target: format!("127.0.0.1:{port}")
        }
    );
    assert_eq!(
        err.to_string(),
        format!(
            "nothing is listening on 127.0.0.1:{port} \u{2014} the login may have ended; x \
             cancels it and a new attempt listens elsewhere"
        )
    );
}

#[tokio::test]
async fn deliver_times_out_on_a_silent_listener() {
    let (port, _server) = listening(|stream| async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        drop(stream);
    })
    .await;
    let limits = DeliverLimits {
        connect: Duration::from_millis(500),
        response: Duration::from_millis(100),
    };
    let started = Instant::now();
    let err = deliver_good(port, limits).await.expect_err("silence");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        err,
        DeliverError::Timeout {
            target: format!("127.0.0.1:{port}"),
            after: Duration::from_millis(100),
        }
    );
}

#[tokio::test]
async fn deliver_treats_a_close_after_the_status_line_as_the_answer() {
    let (port, _head) = answering(b"HTTP/1.1 200 OK\r\n".to_vec()).await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("R-10: a close after the status line is the end of the answer");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.said, None);
}

#[tokio::test]
async fn deliver_treats_a_reset_after_the_status_line_as_the_answer() {
    let (port, _server) = listening(|mut stream| async move {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\n")
            .await
            .expect("write");
        stream.flush().await.expect("flush");
        stream.set_zero_linger().expect("zero linger");
        drop(stream);
    })
    .await;
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("R-10: a reset after the status line is the end of the answer");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.said, None);
}

#[tokio::test]
async fn a_listener_that_closes_without_answering_is_closed_without_answer() {
    let (port, _server) = listening(|stream| async move { drop(stream) }).await;
    let err = deliver_good(port, DeliverLimits::default())
        .await
        .expect_err("no answer");
    assert_eq!(
        err,
        DeliverError::ClosedWithoutAnswer {
            target: format!("127.0.0.1:{port}")
        }
    );
    assert!(err.to_string().contains("if the login completed"));
}

#[tokio::test]
async fn deliver_stops_at_content_length_without_waiting_for_close() {
    let (port, _server) = listening(|mut stream| async move {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello")
            .await
            .expect("write");
        tokio::time::sleep(Duration::from_secs(10)).await;
        drop(stream);
    })
    .await;
    let started = Instant::now();
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(reply.status, 200);
    assert_eq!(reply.said.as_deref(), Some("hello"));
}

#[tokio::test]
async fn deliver_reads_no_more_than_the_cap() {
    let (port, _server) = listening(|mut stream| async move {
        let mut answer = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        answer.extend(std::iter::repeat_n(b'a', 64 * 1024));
        let _ = stream.write_all(&answer).await;
        tokio::time::sleep(Duration::from_secs(10)).await;
        drop(stream);
    })
    .await;
    let started = Instant::now();
    let reply = deliver_good(port, DeliverLimits::default())
        .await
        .expect("an answer");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(reply.status, 200);
    let said = reply.said.expect("an excerpt");
    assert!(said.chars().count() <= EXCERPT_WIDTH);
}

#[tokio::test]
async fn a_non_http_answer_is_not_http() {
    let (port, _head) = answering(b"SSH-2.0-x\r\n".to_vec()).await;
    let err = deliver_good(port, DeliverLimits::default())
        .await
        .expect_err("not http");
    assert_eq!(
        err,
        DeliverError::NotHttp {
            target: format!("127.0.0.1:{port}")
        }
    );
}

/// Review L-9: a port on `[::1]` whose `127.0.0.1` twin this test holds bound — not listening,
/// and without `SO_REUSEADDR` — so a connect there is refused, and no other test binding
/// `127.0.0.1:0` in parallel can be handed the same port and answer the delivery instead. A port
/// whose twin someone else already holds is let go and another drawn. `None` when `[::1]` cannot
/// be bound at all.
async fn ipv6_only_port() -> Option<(TcpListener, tokio::net::TcpSocket)> {
    for _ in 0..32 {
        let listener = match TcpListener::bind("[::1]:0").await {
            Ok(listener) => listener,
            Err(err) => {
                // Written past the harness's capture, so a skip is seen rather than read as a pass.
                use std::io::Write as _;
                let _ = writeln!(
                    std::io::stderr(),
                    "NOTE: localhost_falls_back_to_the_ipv6_loopback SKIPPED: the IPv6 loopback \
                     cannot be bound here ({err})"
                );
                return None;
            }
        };
        let port = listener.local_addr().expect("addr").port();
        let twin = tokio::net::TcpSocket::new_v4().expect("an IPv4 socket");
        twin.set_reuseaddr(false).expect("no SO_REUSEADDR");
        if twin
            .bind(std::net::SocketAddr::from(([127, 0, 0, 1], port)))
            .is_ok()
        {
            return Some((listener, twin));
        }
    }
    panic!("no [::1] port in 32 draws had a free 127.0.0.1 twin");
}

#[tokio::test]
async fn localhost_falls_back_to_the_ipv6_loopback() {
    let Some((listener, _twin)) = ipv6_only_port().await else {
        return;
    };
    let port = listener.local_addr().expect("addr").port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let head = read_head(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\n\r\n")
            .await
            .expect("write");
        stream.shutdown().await.expect("shutdown");
        head
    });
    let at = Advertised::from_auth_url(&link(&format!("localhost:{port}"), STATE)).expect("link");
    let text = format!("http://localhost:{port}/?code={CODE}&state={STATE}");
    let delivery = validate(&paste(&text), &at).expect("valid");
    let reply = deliver(delivery, DeliverLimits::default())
        .await
        .expect("the IPv6 listener answers");
    assert_eq!(reply.status, 204);
    assert_eq!(reply.target, format!("localhost:{port}"));
    let head = server.await.expect("the listener");
    assert!(
        head.contains(&format!("\r\nHost: localhost:{port}\r\n")),
        "the Host header names the advertised host"
    );
}

#[test]
fn the_listener_reply_summary_reads_status_reason_redirect_and_excerpt() {
    let reply = |status, reason: &str, said: Option<&str>, location: Option<&str>| ListenerReply {
        target: "127.0.0.1:39879".to_owned(),
        status,
        reason: reason.to_owned(),
        said: said.map(str::to_owned),
        location_host: location.map(str::to_owned),
    };
    assert_eq!(
        reply(200, "OK", Some("Signed in"), None).summary(),
        "127.0.0.1:39879 answered 200 OK: \"Signed in\""
    );
    assert_eq!(
        reply(204, "", None, None).summary(),
        "127.0.0.1:39879 answered 204"
    );
    assert_eq!(
        reply(302, "Found", None, Some("example.com")).summary(),
        "127.0.0.1:39879 answered 302 Found, redirecting to example.com"
    );
    assert_eq!(
        reply(303, "See Other", Some("Moved"), Some("example.com")).summary(),
        "127.0.0.1:39879 answered 303 See Other, redirecting to example.com: \"Moved\""
    );
}
