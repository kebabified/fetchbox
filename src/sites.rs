//! "Can yt-dlp download this link?" — answered offline from yt-dlp's own URL patterns
//! (assets/extractors.json, exported by tools/dump_extractors.py).
//!
//! Memory matters here: the browser helper keeps these for as long as the browser runs. Compiled one
//! by one with Unicode classes, the ~1,800 patterns took hundreds of MB. Instead, the ones the plain
//! regex engine understands go into a single set with ASCII classes (links from the browser are
//! percent-encoded ASCII, so `\w` and `\d` mean the same thing there), and only the few that need
//! lookarounds or backreferences are compiled on their own with fancy_regex. Together: about 16 MB.

use regex::bytes::{RegexSet, RegexSetBuilder};
use serde::{Deserialize, Serialize};
use std::sync::{mpsc, Mutex, OnceLock};

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
}

struct Matcher {
    sites: Vec<Site>,
    set: Option<RegexSet>,
    set_site: Vec<usize>,                     // set pattern -> index in `sites`
    fancy: Vec<(usize, fancy_regex::Regex)>, // patterns the set can't take, with their site
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

/// Python's re.match anchors at the start only.
fn anchored(p: &str) -> String {
    format!(r"\A(?:{p})")
}

fn build() -> Matcher {
    let raw: Raw = serde_json::from_str(DATA).expect("extractors.json is valid");
    let mut parser = regex_syntax::ParserBuilder::new();
    parser.unicode(false).utf8(false);
    let (mut sites, mut plain, mut set_site, mut odd) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (i, ie) in raw.extractors.into_iter().enumerate() {
        for p in &ie.p {
            let p = anchored(p);
            if parser.build().parse(&p).is_ok() {
                plain.push(p);
                set_site.push(i);
            } else {
                odd.push((i, p));
            }
        }
        sites.push(Site { collection: is_collection(&ie.k), key: ie.k, custom: ie.c });
    }
    let set = match RegexSetBuilder::new(&plain).unicode(false).size_limit(64 << 20).dfa_size_limit(2 << 20).build() {
        Ok(set) => Some(set),
        Err(e) => {
            // never expected; checking every link one pattern at a time still works, it just costs more memory
            crate::util::log("host", &format!("site pattern set didn't build, using single patterns: {e}"));
            odd.extend(set_site.drain(..).zip(plain));
            None
        }
    };
    let fancy = odd.into_iter().filter_map(|(i, p)| fancy_regex::Regex::new(&p).ok().map(|re| (i, re))).collect();
    Matcher { sites, set, set_site, fancy }
}

fn matcher() -> &'static Matcher {
    static M: OnceLock<Matcher> = OnceLock::new();
    M.get_or_init(build)
}

/// Start compiling the patterns in the background.
pub fn warm_up() {
    std::thread::spawn(|| {
        matcher();
    });
}

pub fn check(url: &str) -> Verdict {
    if !(url.starts_with("http://") || url.starts_with("https://")) || url.len() > 4096 {
        return Verdict::default();
    }
    // All checks run on one long-lived thread: the regex engine keeps scratch space per thread it has
    // seen, and the helper answers each message on a new thread, so that would add up to ~100 MB.
    // A link takes well under a millisecond.
    type Job = (String, mpsc::Sender<Verdict>);
    static WORKER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::spawn(move || {
            for (url, reply) in rx {
                let _ = reply.send(check_here(&url));
            }
        });
        Mutex::new(tx)
    });
    let (tx, rx) = mpsc::channel();
    let sent = worker.lock().unwrap_or_else(|e| e.into_inner()).send((url.to_string(), tx));
    if sent.is_err() {
        return check_here(url); // the worker is gone (never expected): answer here
    }
    rx.recv().unwrap_or_else(|_| check_here(url))
}

fn check_here(url: &str) -> Verdict {
    let m = matcher();
    let mut hits: Vec<usize> = m.set.as_ref().map(|s| s.matches(url.as_bytes()).iter().map(|i| m.set_site[i]).collect()).unwrap_or_default();
    hits.extend(m.fancy.iter().filter(|(_, re)| re.is_match(url).unwrap_or(false)).map(|(i, _)| *i));
    hits.sort_unstable();
    hits.dedup();

    let mut fallback: Option<&Site> = None;
    for site in hits.into_iter().map(|i| &m.sites[i]) {
        // Extractors with their own extra rules usually defer to a more specific one: prefer a plain match.
        if !site.custom {
            return verdict(site);
        }
        // …and among those, a list page (channel, playlist) wins over a generic single-item match.
        if fallback.is_none() || (site.collection && !fallback.is_some_and(|f| f.collection)) {
            fallback = Some(site);
        }
    }
    fallback.map(verdict).unwrap_or_default()
}

fn verdict(s: &Site) -> Verdict {
    Verdict { supported: true, extractor: Some(s.key.clone()), collection: s.collection }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        let raw: Raw = serde_json::from_str(DATA).unwrap();
        let total: usize = raw.extractors.iter().map(|ie| ie.p.len()).sum();
        let m = matcher();
        assert!(m.set.is_some(), "the pattern set should build");
        // one of yt-dlp's patterns has never compiled in Rust (it didn't before either)
        assert!(m.set_site.len() + m.fancy.len() + 1 >= total, "{} + {} of {total}", m.set_site.len(), m.fancy.len());
    }

    #[test]
    fn known_links() {
        let ex = |u: &str| check(u).extractor.unwrap_or_default();
        assert_eq!(ex("https://www.youtube.com/watch?v=IoDheeVFlCE"), "Youtube");
        assert_eq!(ex("https://youtu.be/IoDheeVFlCE"), "Youtube");
        assert_eq!(ex("https://www.youtube.com/shorts/iVEaSFZhCX8"), "Youtube");
        assert_eq!(ex("https://x.com/someone/status/2125199652509372571"), "Twitter");
        assert_eq!(ex("https://vimeo.com/76979871"), "Vimeo");
        assert_eq!(ex("https://www.tiktok.com/@user.name/video/7312345678901234567"), "TikTok");
        assert_eq!(ex("https://www.youtube.com/@Ünïcödé/videos"), "YoutubeTab"); // non-ASCII links still match
        assert_eq!(ex("https://framatube.org/videos/watch/9c9de5e8-0a1e-484a-b099-e80766180a6d"), "PeerTube"); // a lookaround pattern
        assert!(check("https://www.youtube.com/@somechannel/videos").collection);
        assert_eq!(check("https://www.example.com/"), Verdict::default());
        assert_eq!(check("ftp://youtube.com/watch?v=x"), Verdict::default());
    }
}
