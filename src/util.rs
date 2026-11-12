// JSON helpers with the C# Json class semantics, logging, paths, number formats.
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub fn get<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut cur = v;
    for k in path {
        cur = cur.as_object()?.get(*k)?;
    }
    Some(cur)
}
pub fn gs(v: &Value, path: &[&str]) -> Option<String> {
    get(v, path).and_then(|x| x.as_str()).map(|s| s.to_string())
}
/// Numbers only: strings, bools, objects and arrays are not numbers.
pub fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_f64())
}
pub fn gn(v: &Value, path: &[&str]) -> Option<f64> {
    num(get(v, path))
}
/// Positive whole count, anything else is 0.
pub fn count(v: Option<&Value>) -> i64 {
    match num(v) {
        Some(n) if n > 0.0 => n as i64,
        _ => 0,
    }
}
pub fn gc(v: &Value, path: &[&str]) -> i64 {
    count(get(v, path))
}
pub fn gb(v: &Value, path: &[&str]) -> Option<bool> {
    get(v, path).and_then(|x| x.as_bool())
}

pub fn parse_utc(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return Some(n.and_utc());
        }
    }
    None
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}
pub fn t3_userdata() -> PathBuf {
    home().join(".t3").join("userdata")
}
/// Config, hub.key and the log live next to the exe. T3PRESENCE_HOME overrides it for dev builds.
pub fn base_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("T3PRESENCE_HOME") {
        return PathBuf::from(d);
    }
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|x| x.to_path_buf()));
    // dev builds keep their own config, hub.key and log in dev/ at the repo root, away from the real app's.
    if cfg!(feature = "dev") { if let Some(d) = &exe_dir { return d.join("..").join("..").join("dev"); } }
    std::env::current_exe().ok().and_then(|p| p.parent().map(|x| x.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}
pub fn path_of(name: &str) -> PathBuf {
    base_dir().join(name)
}

static LOG: Mutex<()> = Mutex::new(());
pub fn log(s: &str) {
    let _g = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let p = path_of("T3Presence.log");
    if let Ok(m) = std::fs::metadata(&p) {
        if m.len() > 256 * 1024 {
            let _ = std::fs::remove_file(&p);
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), s);
    }
}

// --- number formats, same output as the C# ones ---
fn one_decimal(x: f64) -> String {
    let r = (x * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{:.1}", r) }
}
pub fn tok(n: i64) -> String {
    if n >= 1_000_000_000 { return one_decimal(n as f64 / 1e9) + "B"; }
    if n >= 1_000_000 { return one_decimal(n as f64 / 1e6) + "M"; }
    if n >= 1000 {
        let x = n as f64 / 1e3;
        return if n >= 100_000 { format!("{}", x.round() as i64) } else { one_decimal(x) } + "k";
    }
    n.to_string()
}
pub fn usd(v: f64) -> String {
    let s = format!("{:.2}", v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 { out.push(','); }
        out.push(c);
    }
    format!("{}${}.{}", if v < 0.0 { "-" } else { "" }, out, frac)
}
pub fn dur(m: Option<i64>) -> String {
    match m {
        None => "?".into(),
        Some(v) if v >= 1440 => format!("{}d{}h", v / 1440, v % 1440 / 60),
        Some(v) if v >= 60 => format!("{}h{:02}m", v / 60, v % 60),
        Some(v) => format!("{}m", v),
    }
}
/// Discord cuts at 128 characters; slicing bytes would panic on non-ASCII names.
pub fn cut(s: &str) -> String {
    s.chars().take(128).collect()
}
