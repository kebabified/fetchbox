//! "Can yt-dlp download this link?" — answered offline from yt-dlp's own URL patterns
//! (assets/extractors.json, exported by tools/dump_extractors.py).

use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static DATA: &str = include_str!("../assets/extractors.json");

#[derive(Deserialize)]
struct Raw {
    extractors: Vec<RawIe>,
}
#[derive(Deserialize)]
struct RawIe {
    k: String,
    p: Vec<String>,
    c: bool,
}

struct Site {
    key: String,
    custom: bool,
    collection: bool,
    patterns: Vec<Regex>,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Verdict {
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extractor: Option<String>,
    pub collection: bool,
}

/// Extractors that return a list (channel, playlist, feed…) rather than one post.
fn is_collection(key: &str) -> bool {
    const TAILS: &[&str] = &[
        "Tab", "Playlist", "Playlists", "Feed", "Feeds", "Recommended", "Subscriptions", "History", "Search",
        "SearchURL", "User", "Users", "Channel", "Channels", "Profile", "Album", "Albums", "Collection",
        "Collections", "Favorites", "Likes", "WatchLater", "Series", "Season", "Seasons", "Category", "Hashtag",
        "Tag", "Topic", "Set", "Sets", "List", "Show", "Shows", "Videos", "Clips", "Sound", "Effect", "Challenge",
    ];
    TAILS.iter().any(|t| key.ends_with(t))
}

fn sites() -> &'static [Site] {
    static SITES: OnceLock<Vec<Site>> = OnceLock::new();
    SITES.get_or_init(|| {
        let raw: Raw = serde_json::from_str(DATA).expect("extractors.json is valid");
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        let chunk = raw.extractors.len().div_ceil(n).max(1);
        // Compile on several threads: ~1,700 patterns, some of them large.
        std::thread::scope(|s| {
            let handles: Vec<_> = raw
                .extractors
                .chunks(chunk)
                .map(|part| {
                    s.spawn(move || {
                        part.iter()
                            .map(|ie| Site {
                                key: ie.k.clone(),
                                custom: ie.c,
                                collection: is_collection(&ie.k),
                                // Python's re.match anchors at the start only.
                                patterns: ie.p.iter().filter_map(|p| Regex::new(&format!(r"\A(?:{p})")).ok()).collect(),
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        })
    })
}

/// Start compiling the patterns in the background.
pub fn warm_up() {
    std::thread::spawn(|| {
        sites();
    });
}

pub fn check(url: &str) -> Verdict {
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 4096 {
        return Verdict::default();
    }
    let mut fallback: Option<&Site> = None;
    for site in sites() {
        if site.patterns.iter().any(|re| re.is_match(url).unwrap_or(false)) {
            // Extractors with their own extra rules usually defer to a more specific one: prefer a plain match.
            if !site.custom {
                return verdict(site);
            }
            // …and among those, a list page (channel, playlist) wins over a generic single-item match.
            if fallback.is_none() || (site.collection && !fallback.is_some_and(|f| f.collection)) {
                fallback = Some(site);
            }
        }
    }
    fallback.map(verdict).unwrap_or_default()
}

fn verdict(s: &Site) -> Verdict {
    Verdict { supported: true, extractor: Some(s.key.clone()), collection: s.collection }
}

/// (extractors, patterns that failed to compile) — used by the self-test.
pub fn stats() -> (usize, Vec<String>) {
    let raw: Raw = serde_json::from_str(DATA).expect("extractors.json is valid");
    let bad = raw
        .extractors
        .iter()
        .filter(|ie| ie.p.iter().any(|p| Regex::new(&format!(r"\A(?:{p})")).is_err()))
        .map(|ie| ie.k.clone())
        .collect();
    (raw.extractors.len(), bad)
}
