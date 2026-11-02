use crate::commands::dispatch::{DispatchInner, DispatchOptions, MAIN_POOL};
use crate::commands::pool_api::{
    find_pool, host_allowed, parse_target, select, serve, ApiQuery, PoolSource,
};
use crate::commands::pools::PoolMeta;
use crate::transport::Upstream;
use base64::Engine;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn up(scheme: &str, host: &str, country: &str, latency: u64) -> Upstream {
    let url = format!("{scheme}://{host}:1080");
    Upstream {
        raw: url.clone(),
        url,
        scheme: scheme.into(),
        host: host.into(),
        port: 1080,
        user: None,
        pass: None,
        latency: Some(latency),
        country: Some(country.into()),
    }
}

fn meta(id: &str, name: &str) -> PoolMeta {
    PoolMeta {
        id: id.into(),
        name: name.into(),
    }
}

#[test]
fn request_targets_are_parsed() {
    let (path, q) = parse_target(
        "/random?country=DE,nl;United%20States&proto=SOCKS5&limit=0&format=json&pool=%D1%80%D0%B0%D0%B1%D0%BE%D1%82%D0%B0",
    )
    .unwrap();
    assert_eq!(path, "/random");
    assert_eq!(
        q,
        ApiQuery {
            pool: Some("работа".into()),
            countries: vec!["DE".into(), "nl".into(), "United States".into()],
            protos: vec!["socks5".into()],
            limit: Some(1),
            json: true,
        }
    );
    assert_eq!(
        parse_target("/").unwrap(),
        ("/".into(), ApiQuery::default())
    );
    assert!(parse_target("/list?format=xml").is_err());
    assert!(parse_target("/list?limit=many").is_err());
}

#[test]
fn proxies_are_filtered_by_country_and_protocol() {
    let pool = vec![
        up("http", "10.0.0.1", "DE", 100),
        up("socks5h", "10.0.0.2", "Germany", 50),
        up("socks4", "10.0.0.3", "NL", 30),
    ];
    let none = HashSet::new();
    let hosts = |q: &ApiQuery, cooling: &HashSet<String>| {
        select(&pool, q, cooling)
            .into_iter()
            .map(|u| u.host)
            .collect::<Vec<_>>()
    };
    let q = ApiQuery {
        countries: vec!["de".into(), "germany".into()],
        ..ApiQuery::default()
    };
    assert_eq!(hosts(&q, &none), ["10.0.0.1", "10.0.0.2"]);
    let q = ApiQuery {
        protos: vec!["socks5".into()],
        ..ApiQuery::default()
    };
    assert_eq!(hosts(&q, &none), ["10.0.0.2"]);
    let q = ApiQuery {
        protos: vec!["socks".into()],
        ..ApiQuery::default()
    };
    assert_eq!(hosts(&q, &none), ["10.0.0.2", "10.0.0.3"]);

    // Upstreams in cooldown are left out unless nothing else matches.
    let cooling: HashSet<String> = [pool[0].url.clone()].into();
    let q = ApiQuery {
        countries: vec!["DE".into()],
        ..ApiQuery::default()
    };
    assert_eq!(hosts(&q, &cooling), ["10.0.0.1"]);
    assert_eq!(
        hosts(&ApiQuery::default(), &cooling),
        ["10.0.0.2", "10.0.0.3"]
    );
}

#[test]
fn pools_are_found_by_id_or_name() {
    let pools = vec![(meta(MAIN_POOL, ""), 0), (meta("p1a2b3", "Работа"), 1)];
    assert_eq!(find_pool(&pools, None).unwrap().1, 0);
    assert_eq!(find_pool(&pools, Some("p1a2b3")).unwrap().1, 1);
    assert_eq!(find_pool(&pools, Some("работа")).unwrap().1, 1);
    assert!(find_pool(&pools, Some("other")).is_none());
    assert!(find_pool(&pools, Some("")).is_none());
}

#[test]
fn only_local_host_names_are_served() {
    let head = |host: &str| format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n");
    assert!(host_allowed(&head("127.0.0.1:1090")));
    assert!(host_allowed(&head("LOCALHOST:1090")));
    assert!(host_allowed(&head("[::1]:1090")));
    assert!(!host_allowed(&head("rebind.example.com:1090")));
    assert!(!host_allowed(&head("127.0.0.1.example.com")));
    assert!(host_allowed("GET / HTTP/1.0\r\n\r\n"));
}

async fn get(port: u16, request: &str) -> (String, String) {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    (head.lines().next().unwrap().to_string(), body.to_string())
}

#[tokio::test]
async fn the_api_serves_the_pools() {
    let main = Arc::new(DispatchInner::new(MAIN_POOL));
    *main.pool.lock().await = vec![
        up("http", "10.0.0.1", "DE", 100),
        up("socks5", "10.0.0.2", "NL", 40),
    ];
    let work = Arc::new(DispatchInner::new("p1"));
    *work.pool.lock().await = vec![up("http", "10.0.0.9", "US", 10)];
    *work.options.lock().await = DispatchOptions {
        auth_user: "me".into(),
        auth_pass: "secret".into(),
        ..DispatchOptions::default()
    };
    let pools = vec![(meta(MAIN_POOL, ""), main), (meta("p1", "Work"), work)];
    let source: PoolSource = Arc::new(move || pools.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(serve(listener, source));
    let req = |path: &str| format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");

    let (status, body) = get(port, &req("/fastest")).await;
    assert_eq!(
        (status.as_str(), body.as_str()),
        ("HTTP/1.1 200 OK", "socks5://10.0.0.2:1080\n")
    );

    let (_, body) = get(port, &req("/list?country=DE&format=json")).await;
    let list: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(list[0]["host"], "10.0.0.1");
    assert_eq!(list.as_array().unwrap().len(), 1);

    let (_, body) = get(port, &req("/random?proto=socks5")).await;
    assert_eq!(body, "socks5://10.0.0.2:1080\n");

    let (status, _) = get(port, &req("/random?country=JP")).await;
    assert!(status.contains("404"));

    // A pool with a dispatcher login needs it.
    let (status, _) = get(port, &req("/random?pool=work")).await;
    assert!(status.contains("401"));
    let token = base64::engine::general_purpose::STANDARD.encode("me:secret");
    let (status, body) = get(
        port,
        &format!("GET /random?pool=work HTTP/1.1\r\nHost: localhost:{port}\r\nAuthorization: Basic {token}\r\n\r\n"),
    )
    .await;
    assert_eq!(
        (status.as_str(), body.as_str()),
        ("HTTP/1.1 200 OK", "http://10.0.0.9:1080\n")
    );

    // A pool with a login is listed only with it.
    let (_, body) = get(port, &req("/pools")).await;
    assert_eq!(body, "main\t\t2\n");
    let (_, body) = get(
        port,
        &format!(
            "GET /pools HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Basic {token}\r\n\r\n"
        ),
    )
    .await;
    assert_eq!(body, "main\t\t2\np1\tWork\t1\n");

    let (status, _) = get(
        port,
        &format!("GET /list HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n"),
    )
    .await;
    assert!(status.contains("403"));
    let (status, _) = get(
        port,
        &format!("POST /list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"),
    )
    .await;
    assert!(status.contains("405"));
    let (status, _) = get(port, &req("/nope")).await;
    assert!(status.contains("404"));
    server.abort();
}
