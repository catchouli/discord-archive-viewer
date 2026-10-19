//! Search across every channel, with Discord-style filters:
//! `from:` `in:` `has:` `before:` `after:` `during:` `pinned:` and "quoted phrases".

use chrono::{Local, NaiveDate, TimeZone};
use rayon::prelude::*;

use crate::model::{Archive, flags};

#[derive(Default, Debug)]
pub struct Query {
    pub terms: Vec<String>,
    pub from: Vec<String>,
    pub channels: Vec<String>,
    pub has: u8,
    pub before: Option<i64>,
    pub after: Option<i64>,
    pub pinned: bool,
    pub errors: Vec<String>,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.from.is_empty()
            && self.channels.is_empty()
            && self.has == 0
            && self.before.is_none()
            && self.after.is_none()
            && !self.pinned
    }
}

fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn day_start(d: NaiveDate) -> i64 {
    Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .map(|t| t.timestamp_millis())
        .unwrap_or(0)
}

/// Parses YYYY-MM-DD, YYYY-MM or YYYY into a [start, end) range of local days.
fn date_range(s: &str) -> Option<(i64, i64)> {
    let parts: Vec<&str> = s.split(['-', '/']).collect();
    let y: i32 = parts.first()?.parse().ok()?;
    match parts.len() {
        1 => Some((day_start(NaiveDate::from_ymd_opt(y, 1, 1)?), day_start(NaiveDate::from_ymd_opt(y + 1, 1, 1)?))),
        2 => {
            let m: u32 = parts[1].parse().ok()?;
            let next = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(y, m + 1, 1)? };
            Some((day_start(NaiveDate::from_ymd_opt(y, m, 1)?), day_start(next)))
        }
        3 => {
            let d = NaiveDate::from_ymd_opt(y, parts[1].parse().ok()?, parts[2].parse().ok()?)?;
            Some((day_start(d), day_start(d.succ_opt()?)))
        }
        _ => None,
    }
}

pub fn parse(q: &str) -> Query {
    let mut out = Query::default();
    for tok in tokens(q) {
        let lower = tok.to_lowercase();
        let Some((key, val)) = lower.split_once(':').filter(|(k, v)| {
            !v.is_empty() && matches!(*k, "from" | "in" | "has" | "before" | "after" | "during" | "on" | "pinned")
        }) else {
            out.terms.push(lower);
            continue;
        };
        match key {
            "from" => out.from.push(val.trim_start_matches('@').to_string()),
            "in" => out.channels.push(val.trim_start_matches('#').to_string()),
            "has" => {
                out.has |= match val {
                    "image" | "images" | "picture" => flags::IMAGE,
                    "video" | "videos" => flags::VIDEO,
                    "audio" | "sound" => flags::AUDIO,
                    "file" | "files" => flags::FILE,
                    "link" | "links" | "url" => flags::LINK,
                    "embed" | "embeds" => flags::EMBED,
                    "sticker" | "stickers" => flags::STICKER,
                    "reaction" | "reactions" => flags::REACTION,
                    "attachment" | "media" => flags::IMAGE | flags::VIDEO | flags::AUDIO | flags::FILE,
                    other => {
                        out.errors.push(format!("unknown has:{other}"));
                        0
                    }
                }
            }
            "pinned" => out.pinned = val == "true" || val == "yes",
            _ => match date_range(val) {
                Some((start, end)) => match key {
                    "before" => out.before = Some(start),
                    "after" => out.after = Some(end),
                    _ => {
                        out.after = Some(start);
                        out.before = Some(end);
                    }
                },
                None => out.errors.push(format!("couldn't read date \"{val}\"")),
            },
        }
    }
    out
}

/// Matches, newest first, as (channel, message index).
pub fn run(archive: &Archive, q: &Query) -> Vec<(u32, u32)> {
    if q.is_empty() {
        return Vec::new();
    }
    let users: Option<Vec<bool>> = (!q.from.is_empty()).then(|| {
        archive
            .users
            .iter()
            .map(|u| {
                let (a, b) = (u.name.to_lowercase(), u.nickname.as_deref().unwrap_or("").to_lowercase());
                q.from.iter().any(|f| a.contains(f.as_str()) || b.contains(f.as_str()))
            })
            .collect()
    });
    let has = q.has;
    let mut hits: Vec<(i64, u32, u32)> = archive
        .channels
        .par_iter()
        .enumerate()
        .filter(|(_, c)| q.channels.is_empty() || q.channels.iter().any(|n| c.name.to_lowercase().contains(n.as_str())))
        .flat_map_iter(|(ci, c)| {
            let users = &users;
            c.messages.iter().enumerate().filter_map(move |(mi, m)| {
                let ok = (has == 0 || m.flags & has == has)
                    && (!q.pinned || m.pinned)
                    && q.before.is_none_or(|b| m.ts < b)
                    && q.after.is_none_or(|a| m.ts >= a)
                    && users.as_ref().is_none_or(|u| u[m.author as usize])
                    && q.terms.iter().all(|t| m.haystack.contains(t.as_str()));
                ok.then_some((m.ts, ci as u32, mi as u32))
            })
        })
        .collect();
    hits.sort_unstable_by_key(|h| std::cmp::Reverse(h.0));
    hits.into_iter().map(|(_, c, m)| (c, m)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filters() {
        let q = parse("from:juni in:#art has:image \"two words\" during:2024-05 hello");
        assert_eq!(q.from, ["juni"]);
        assert_eq!(q.channels, ["art"]);
        assert_eq!(q.has, flags::IMAGE);
        assert_eq!(q.terms, ["two words", "hello"]);
        assert!(q.after.unwrap() < q.before.unwrap());
        assert!(q.errors.is_empty());
    }

    #[test]
    fn plain_colons_are_terms() {
        let q = parse("time: 10:30");
        assert_eq!(q.terms, ["time:", "10:30"]);
    }
}
