//! Exit IP reputation through DNS blocklists (DNSBL): `d.c.b.a.<zone>`
//! resolving to 127.0.0.x means the IPv4 address `a.b.c.d` is listed.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Zones queried, with the name shown in the UI.
pub(crate) const ZONES: [(&str, &str); 4] = [
    ("zen.spamhaus.org", "Spamhaus"),
    ("b.barracudacentral.org", "Barracuda"),
    ("bl.spamcop.net", "SpamCop"),
    ("dnsbl.dronebl.org", "DroneBL"),
];

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(4);
const CACHE_TTL: Duration = Duration::from_secs(3600);
const CACHE_MAX: usize = 20_000;
/// How long a zone's health check (see [`zone_works`]) is trusted.
const ZONE_CHECK_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DnsblResult {
    /// Lists that contain the IP.
    pub listed: Vec<String>,
    /// Lists that did not answer usefully (timeout, or a refusal such as
    /// Spamhaus answering queries from public resolvers with 127.255.255.x).
    pub unknown: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Answer {
    Listed,
    NotListed,
    Unknown,
}

pub(crate) fn query_name(ip: Ipv4Addr, zone: &str) -> String {
    let [a, b, c, d] = ip.octets();
    format!("{d}.{c}.{b}.{a}.{zone}")
}

/// Classify the addresses a DNSBL name resolved to.
pub(crate) fn interpret(addrs: &[IpAddr]) -> Answer {
    let mut listed = false;
    for a in addrs {
        match a {
            // 127.255.255.x are error codes (refused, rate limited), not listings.
            IpAddr::V4(v4) if v4.octets()[..3] == [127, 255, 255] => return Answer::Unknown,
            IpAddr::V4(v4) if v4.octets()[0] == 127 => listed = true,
            _ => {}
        }
    }
    if listed {
        Answer::Listed
    } else if addrs.is_empty() {
        Answer::NotListed
    } else {
        Answer::Unknown
    }
}

async fn lookup(name: String) -> Answer {
    match tokio::time::timeout(LOOKUP_TIMEOUT, tokio::net::lookup_host((name.as_str(), 0))).await {
        Ok(Ok(addrs)) => interpret(&addrs.map(|s| s.ip()).collect::<Vec<_>>()),
        // NXDOMAIN: not listed. Resolver errors cannot be told apart here.
        Ok(Err(_)) => Answer::NotListed,
        Err(_) => Answer::Unknown,
    }
}

/// Whether a zone answers through this machine's resolver. A resolver error
/// cannot be told from NXDOMAIN, which means "not listed", so a broken
/// resolver, or a zone refusing it, would show every IP clean. RFC 5782 has
/// every DNSBL list 127.0.0.2 as a test entry: a zone that does not list it
/// here gives no answers for this run.
async fn zone_works(zone: &'static str) -> bool {
    static CHECKED: OnceLock<Mutex<HashMap<&'static str, (bool, Instant)>>> = OnceLock::new();
    let checked = CHECKED.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((ok, at)) = checked.lock().ok().and_then(|c| c.get(zone).copied()) {
        if at.elapsed() < ZONE_CHECK_TTL {
            return ok;
        }
    }
    let ok = lookup(query_name(Ipv4Addr::new(127, 0, 0, 2), zone)).await == Answer::Listed;
    if let Ok(mut c) = checked.lock() {
        c.insert(zone, (ok, Instant::now()));
    }
    ok
}

fn cache() -> &'static Mutex<HashMap<Ipv4Addr, (DnsblResult, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<Ipv4Addr, (DnsblResult, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Check an exit IP against every zone. None for IPv6 or a non-IP value:
/// the lists queried here cover IPv4 only.
pub(crate) async fn check_ip(ip: &str) -> Option<DnsblResult> {
    let v4: Ipv4Addr = ip.trim().parse().ok()?;
    if let Some((r, at)) = cache().lock().ok()?.get(&v4) {
        if at.elapsed() < CACHE_TTL {
            return Some(r.clone());
        }
    }
    let mut set = tokio::task::JoinSet::new();
    for (zone, label) in ZONES {
        set.spawn(async move {
            if !zone_works(zone).await {
                return (label, Answer::Unknown);
            }
            (label, lookup(query_name(v4, zone)).await)
        });
    }
    let mut result = DnsblResult::default();
    while let Some(Ok((label, answer))) = set.join_next().await {
        match answer {
            Answer::Listed => result.listed.push(label.to_string()),
            Answer::Unknown => result.unknown.push(label.to_string()),
            Answer::NotListed => {}
        }
    }
    result.listed.sort();
    result.unknown.sort();
    // A lookup that failed is asked again next time, not remembered for an
    // hour.
    if !result.unknown.is_empty() {
        return Some(result);
    }
    if let Ok(mut c) = cache().lock() {
        if c.len() >= CACHE_MAX {
            c.clear();
        }
        c.insert(v4, (result.clone(), Instant::now()));
    }
    Some(result)
}
