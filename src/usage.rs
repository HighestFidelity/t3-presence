// Plan limits (CLIProxy hub, else T3 provider logs) and token totals (T3's usage scan cache + transcript tails).
use crate::util::*;
use chrono::{Local, TimeZone, Utc};
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlanWindow { pub used: i64, pub in_min: Option<i64> }
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlanAccount { pub five_hour: Option<PlanWindow>, pub week: Option<PlanWindow> }

#[derive(Clone, Debug, Default)]
pub struct PlanData {
    pub five_hour: Option<PlanWindow>,
    pub week: Option<PlanWindow>,
    pub accounts: Vec<PlanAccount>,
    pub hub_error: Option<String>,
}
impl PlanData { pub fn from_hub(&self) -> bool { !self.accounts.is_empty() } }

pub struct PlanUsage {
    pub hub_url: Option<String>,
    pub hub_key: Option<String>,
    pub user_data_dirs: Mutex<Vec<PathBuf>>,
    data: Mutex<PlanData>,
    at: Mutex<Option<Instant>>,
}

/// .NET's Math.Round: midpoints go to the even number.
fn round_even(x: f64) -> i64 { x.round_ties_even() as i64 }

impl PlanUsage {
    pub fn new(hub_url: Option<String>, hub_key: Option<String>) -> Self {
        PlanUsage { hub_url, hub_key, user_data_dirs: Mutex::new(vec![t3_userdata()]), data: Mutex::new(PlanData::default()), at: Mutex::new(None) }
    }
    pub fn get(&self) -> PlanData { self.data.lock().unwrap().clone() }

    /// Hub url (and key, when T3 stores it in the clear) from T3's own usageLimitSources; T3 normally masks the key.
    pub fn from_t3_settings() -> (Option<String>, Option<String>) {
        let Ok(t) = std::fs::read_to_string(t3_userdata().join("settings.json")) else { return (None, None) };
        let Ok(d) = serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')) else { return (None, None) };
        let Some(srcs) = get(&d, &["usageLimitSources"]).and_then(|x| x.as_object()) else { return (None, None) };
        for v in srcs.values() {
            if gs(v, &["kind"]).as_deref() != Some("cliproxy") || gb(v, &["enabled"]) == Some(false) { continue; }
            let key = gs(v, &["managementKey"]).filter(|k| !k.is_empty() && !k.contains('•'));
            return (gs(v, &["url"]), key);
        }
        (None, None)
    }

    pub fn refresh(&self) {
        {
            let mut at = self.at.lock().unwrap();
            let gap = if self.hub_key.as_deref().unwrap_or("").is_empty() { 120 } else { 180 };
            if at.map_or(false, |t| t.elapsed().as_secs() < gap) { return; }
            *at = Some(Instant::now());
        }
        if let (Some(u), Some(k)) = (self.hub_url.as_deref().filter(|x| !x.is_empty()), self.hub_key.as_deref().filter(|x| !x.is_empty())) {
            match refresh_hub(u, k) {
                Ok(list) => {
                    let mut d = self.data.lock().unwrap();
                    // The pool is only stuck when every account is: headline numbers are the account with the most room.
                    d.five_hour = list.iter().filter_map(|a| a.five_hour.clone()).min_by_key(|w| w.used);
                    d.week = list.iter().filter_map(|a| a.week.clone()).min_by_key(|w| w.used);
                    d.accounts = list; d.hub_error = None;
                    return;
                }
                Err(e) => { let mut d = self.data.lock().unwrap(); d.hub_error = Some(e); d.accounts.clear(); }
            }
        }
        self.refresh_logs();
    }

    fn refresh_logs(&self) {
        let dirs = self.user_data_dirs.lock().unwrap().clone();
        let mut all: Vec<(SystemTime, PathBuf)> = vec![];
        for ud in dirs {
            let Ok(rd) = std::fs::read_dir(ud.join("logs").join("provider")) else { continue };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("events.") && n.ends_with(".log") {
                    if let Ok(m) = e.metadata().and_then(|m| m.modified()) { all.push((m, e.path())); }
                }
            }
        }
        all.sort_by(|a, b| b.0.cmp(&a.0));
        let re = Regex::new(r"^\[([^\]]+)\]").unwrap();
        let mut best: Option<(chrono::DateTime<Utc>, Value)> = None;
        for (_, f) in all.into_iter().take(4) {
            let Ok(mut fh) = std::fs::File::open(&f) else { continue };
            let len = fh.metadata().map(|m| m.len()).unwrap_or(0).min(1 << 20);
            if fh.seek(SeekFrom::End(-(len as i64))).is_err() { continue; }
            let mut b = Vec::with_capacity(len as usize);
            let _ = fh.take(len).read_to_end(&mut b);
            let txt = String::from_utf8_lossy(&b);
            for l in txt.split('\n').rev() {
                if !l.contains("\"rate_limit_event\"") || !l.contains("\"incoming\"") { continue; }
                let Some(ts) = re.captures(l).and_then(|c| parse_utc(&c[1])) else { continue };
                let start = l.find("NTIVE: ").map(|k| k + 7).or_else(|| l.find('{'));
                let Some(o) = start.and_then(|s| serde_json::from_str::<Value>(&l[s..]).ok()) else { continue };
                if let Some(info) = get(&o, &["event", "payload", "rate_limit_info"]) {
                    if best.as_ref().map_or(true, |b| ts > b.0) { best = Some((ts, info.clone())); }
                }
                break;
            }
        }
        if let Some((_, info)) = best {
            let mut d = self.data.lock().unwrap();
            d.five_hour = log_window(&info, "five_hour");
            d.week = log_window(&info, "seven_day");
        }
    }
}

fn hub(url: &str, key: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(native_tls::TlsConnector::new().map_err(|e| e.to_string())?))
        .timeout(std::time::Duration::from_secs(20)).build();
    let u = format!("{}/v0/management/{}", url.trim_end_matches('/'), path);
    let auth = format!("Bearer {key}");
    let r = match body {
        Some(b) => agent.post(&u).set("Authorization", &auth).send_json(b),
        None => agent.get(&u).set("Authorization", &auth).call(),
    };
    match r {
        Ok(resp) => resp.into_json::<Value>().map_err(|e| e.to_string()),
        Err(ureq::Error::Status(c, _)) => Err(format!("hub answered HTTP {c}")),
        Err(e) => Err(format!("hub unreachable: {}", e.kind())),
    }
}
fn refresh_hub(url: &str, key: &str) -> Result<Vec<PlanAccount>, String> {
    let files = hub(url, key, "auth-files", None)?;
    let mut files: Vec<Value> = get(&files, &["files"]).and_then(|x| x.as_array()).cloned().ok_or("hub sent no account list")?;
    files.sort_by(|a, b| gs(a, &["id"]).cmp(&gs(b, &["id"])));
    let mut list = vec![];
    for f in files {
        if gs(&f, &["provider"]).as_deref() != Some("claude") || gb(&f, &["disabled"]) == Some(true) { continue; }
        let call = json!({"auth_index": gs(&f, &["auth_index"]), "method": "GET", "url": "https://api.anthropic.com/api/oauth/usage",
            "header": {"Authorization": "Bearer $TOKEN$", "anthropic-beta": "oauth-2025-04-20", "User-Agent": "claude-code/2.1.273"}});
        let r = hub(url, key, "api-call", Some(call))?;
        let code = gn(&r, &["status_code"]).unwrap_or(0.0);
        let Some(body) = gs(&r, &["body"]) else { continue };
        if !(200.0..300.0).contains(&code) { continue; }
        let Ok(u) = serde_json::from_str::<Value>(&body) else { continue };
        list.push(PlanAccount { five_hour: hub_window(get(&u, &["five_hour"])), week: hub_window(get(&u, &["seven_day"])) });
    }
    if list.is_empty() { return Err("hub could not read any account's usage".into()); }
    Ok(list)
}
fn hub_window(x: Option<&Value>) -> Option<PlanWindow> {
    let x = x.filter(|v| !v.is_null())?;
    let mut in_min = None; let mut stale = false;
    if let Some(at) = gs(x, &["resets_at"]).and_then(|s| parse_utc(&s)) {
        let m = (at - Utc::now()).num_milliseconds() as f64 / 60000.0;
        stale = m <= 0.0; in_min = Some(round_even(m).max(0));
    }
    Some(PlanWindow { used: if stale { 0 } else { round_even(gn(x, &["utilization"]).unwrap_or(0.0)) }, in_min })
}
fn log_window(info: &Value, key: &str) -> Option<PlanWindow> {
    let x = get(info, &["unifiedWindows", key]).filter(|v| !v.is_null())?;
    let resets = gn(x, &["resetsAt"]).unwrap_or(0.0);
    let in_ms = resets * 1000.0 - Utc::now().timestamp_millis() as f64;
    let stale = resets > 0.0 && in_ms <= 0.0;
    Some(PlanWindow {
        used: if stale { 0 } else { round_even(gn(x, &["utilization"]).unwrap_or(0.0) * 100.0) },
        in_min: if resets > 0.0 { Some(round_even(in_ms / 60000.0).max(0)) } else { None },
    })
}

// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DayTotals { pub input: i64, pub output: i64, pub cache_read: i64, pub cache_write: i64, pub usd: f64 }
impl DayTotals {
    pub fn total(&self) -> i64 { self.input + self.output + self.cache_read + self.cache_write }
    pub fn to_json(&self) -> Value {
        json!({"Input": self.input, "Output": self.output, "CacheRead": self.cache_read, "CacheWrite": self.cache_write, "Usd": self.usd, "Total": self.total()})
    }
    fn add(&mut self, o: &DayTotals) { self.input += o.input; self.output += o.output; self.cache_read += o.cache_read; self.cache_write += o.cache_write; self.usd += o.usd; }
}

/// Same arithmetic as T3's Usage page: its scan-cache rows plus whatever the transcripts have grown since, deduped on
/// "message id:request id" (first copy wins), priced from T3's cached LiteLLM rates unless a row carries a reported cost.
pub struct UsageScan { pub sources: Vec<Arc<UsageSource>>, pub include_remotes: AtomicBool }
impl UsageScan {
    pub fn new() -> Self { UsageScan { sources: vec![Arc::new(UsageSource::new(t3_userdata(), None))], include_remotes: AtomicBool::new(true) } }
    fn active(&self) -> Vec<Arc<UsageSource>> {
        let inc = self.include_remotes.load(Ordering::Relaxed);
        self.sources.iter().filter(|s| !s.remote || inc).cloned().collect()
    }
    pub fn range(&self, days: i64) -> DayTotals { let mut t = DayTotals::default(); for s in self.active() { s.add_range(days, &mut t); } t }
    pub fn all_loaded(&self) -> bool { self.active().iter().all(|s| s.loaded.load(Ordering::Relaxed)) }
    pub fn error(&self) -> String { self.active().iter().filter_map(|s| s.error.lock().unwrap().clone()).collect::<Vec<_>>().join("; ") }
    pub fn refresh_now(&self) { for s in self.active() { s.refresh(); } }
    /// Off the presence loop: a slow network share must not stall page rotation.
    pub fn refresh_in_background(&self) {
        for s in self.active() {
            if s.stamp.lock().unwrap().map_or(false, |t| t.elapsed().as_secs() < 120) || s.running.swap(true, Ordering::SeqCst) { continue; }
            std::thread::Builder::new().name("usage-scan".into()).spawn(move || { s.refresh(); s.running.store(false, Ordering::SeqCst); }).ok();
        }
    }
}

struct Rate { input: f64, output: f64, read: f64, write: f64, fast: f64 }
#[derive(Default)]
struct Work {
    rates: HashMap<String, Rate>,
    offsets: HashMap<String, u64>,
    seen: HashSet<String>,
    transcript_dirs: Vec<PathBuf>,
    remote_dir: Option<String>,
    cache_mtime: Option<SystemTime>,
    rates_mtime: Option<SystemTime>,
}

pub struct UsageSource {
    user_data: PathBuf,
    transcript_root: Option<PathBuf>,
    pub remote: bool,
    pub loaded: AtomicBool,
    running: AtomicBool,
    stamp: Mutex<Option<Instant>>,
    pub error: Mutex<Option<String>>,
    days: Mutex<HashMap<String, DayTotals>>,
    work: Mutex<Work>,
}

fn day_key(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}
fn path_key(p: &str) -> String { p.replace('/', "\\").to_lowercase() }

struct Row<'a> { ms: i64, model: Option<&'a str>, unc: i64, cached: i64, write: i64, output: i64, dedupe: Option<String>, reported: Option<f64>, fast: bool }

impl UsageSource {
    pub fn new(user_data: PathBuf, remote_transcripts: Option<PathBuf>) -> Self {
        UsageSource {
            remote: remote_transcripts.is_some(), user_data, transcript_root: remote_transcripts,
            loaded: AtomicBool::new(false), running: AtomicBool::new(false), stamp: Mutex::new(None),
            error: Mutex::new(None), days: Mutex::new(HashMap::new()), work: Mutex::new(Work::default()),
        }
    }
    fn add_range(&self, back: i64, t: &mut DayTotals) {
        let d = self.days.lock().unwrap();
        let today = Local::now().date_naive();
        for i in 0..back {
            let k = (today - chrono::Duration::days(i)).format("%Y-%m-%d").to_string();
            if let Some(x) = d.get(&k) { t.add(x); }
        }
    }
    pub fn refresh(&self) {
        {
            let mut st = self.stamp.lock().unwrap();
            if st.map_or(false, |t| t.elapsed().as_secs() < 120) { return; }
            *st = Some(Instant::now());
        }
        let mut w = self.work.lock().unwrap();
        match self.refresh_inner(&mut w) {
            Ok(()) => { *self.error.lock().unwrap() = None; self.loaded.store(w.cache_mtime.is_some(), Ordering::Relaxed); }
            Err(e) => *self.error.lock().unwrap() = Some(e),
        }
    }
    fn refresh_inner(&self, w: &mut Work) -> Result<(), String> {
        let rates_file = self.user_data.join("usage-model-rates.json");
        let rates_m = std::fs::metadata(&rates_file).and_then(|m| m.modified()).ok();
        let cache = self.newest_cache()?;
        if let Some((cf, cm)) = &cache {
            let reload = Some(*cm) != w.cache_mtime || (rates_m.is_some() && rates_m != w.rates_mtime);
            if reload {
                if rates_m.is_some() && rates_m != w.rates_mtime { load_rates(w, &rates_file, rates_m)?; }
                self.load_cache(w, cf, *cm)?;
            }
        }
        self.tail_transcripts(w);
        Ok(())
    }
    fn newest_cache(&self) -> Result<Option<(PathBuf, SystemTime)>, String> {
        let re = Regex::new(r"^usage-scan-cache-v(\d+)\.json$").unwrap();
        let mut best: Option<(i64, PathBuf)> = None;
        for e in std::fs::read_dir(&self.user_data).map_err(|e| e.to_string())?.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(v) = re.captures(&n).and_then(|c| c[1].parse::<i64>().ok()) {
                if best.as_ref().map_or(true, |b| v > b.0) { best = Some((v, e.path())); }
            }
        }
        Ok(best.and_then(|(_, p)| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|m| (p, m))))
    }
    fn load_cache(&self, w: &mut Work, f: &Path, mtime: SystemTime) -> Result<(), String> {
        let raw = std::fs::read(f).map_err(|e| e.to_string())?;
        let doc: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        drop(raw);
        let models: Vec<Option<String>> = get(&doc, &["models"]).and_then(|x| x.as_array()).map(|a| a.iter().map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        let Some(files) = get(&doc, &["files"]).and_then(|x| x.as_object()) else { return Ok(()) };
        let mut days = HashMap::new();
        w.seen.clear(); w.offsets.clear(); w.transcript_dirs.clear();
        if let Some(src) = get(&doc, &["sources"]).and_then(|x| x.as_object()) {
            for (k, v) in src {
                if let (Some(dir), true) = (gs(v, &["dir"]), k.starts_with("claude")) {
                    w.transcript_dirs.push(match &self.transcript_root { Some(r) if self.remote => r.clone(), _ => PathBuf::from(&dir) });
                    w.remote_dir = Some(dir);
                }
            }
        }
        for (key, v) in files {
            // A linked machine's cache holds its own paths: translate them to where its transcripts are reachable from here.
            let local = match (&self.transcript_root, &w.remote_dir) {
                (Some(root), Some(rd)) if self.remote && key.starts_with(rd.as_str()) => format!("{}{}", root.display(), key[rd.len()..].replace('/', "\\")),
                _ => key.clone(),
            };
            w.offsets.insert(path_key(&local), gc(v, &["o"]) as u64);
            for part in ["r", "t"] {
                let Some(rows) = get(v, &[part]).and_then(|x| x.as_array()) else { continue };
                for r in rows {
                    let Some(r) = r.as_array().filter(|r| r.len() >= 11) else { continue };
                    let pick = |i: i64| models.get(i as usize).and_then(|m| m.as_deref());
                    let model = pick(count(Some(&r[1])));
                    if model == Some("<synthetic>") { continue; }
                    let mut rate = model;
                    if r.len() > 11 && !r[11].is_null() { let ri = count(Some(&r[11])); if (ri as usize) < models.len() { rate = pick(ri); } }
                    let row = Row { ms: num(Some(&r[0])).unwrap_or(0.0) as i64, model: rate, unc: count(Some(&r[3])), cached: count(Some(&r[4])), write: count(Some(&r[5])), output: count(Some(&r[6])),
                        dedupe: r[8].as_str().map(|s| s.to_string()), reported: num(Some(&r[9])), fast: count(Some(&r[10])) == 1 };
                    add(w, &mut days, row);
                }
            }
        }
        *self.days.lock().unwrap() = days;
        w.cache_mtime = Some(mtime);
        Ok(())
    }
    fn tail_transcripts(&self, w: &mut Work) {
        if w.transcript_dirs.is_empty() {
            w.transcript_dirs.push(self.transcript_root.clone().unwrap_or_else(|| home().join(".claude").join("projects")));
        }
        let cutoff = SystemTime::now() - std::time::Duration::from_secs(100 * 86400);
        let mut files = vec![];
        for d in w.transcript_dirs.clone() { walk(&d, &mut files); }
        for (p, len, m) in files {
            if m < cutoff { continue; }
            let k = path_key(&p.to_string_lossy());
            let mut off = *w.offsets.get(&k).unwrap_or(&0);
            if len == off { continue; }
            if len < off { off = 0; }
            if let Ok(n) = self.tail_one(w, &p, off) { w.offsets.insert(k, n); }
        }
    }
    fn tail_one(&self, w: &mut Work, p: &Path, off: u64) -> std::io::Result<u64> {
        let mut f = std::fs::File::open(p)?;
        f.seek(SeekFrom::Start(off))?;
        let mut buf = vec![];
        f.read_to_end(&mut buf)?;
        let mut start = 0; let mut line_start = off;
        let mut local: HashMap<String, DayTotals> = HashMap::new();
        for i in 0..buf.len() {
            if buf[i] != b'\n' { continue; }
            parse_line(w, &mut local, &buf[start..i]);
            start = i + 1; line_start = off + i as u64 + 1;
        }
        if !local.is_empty() {
            let mut d = self.days.lock().unwrap();
            for (k, v) in local { d.entry(k).or_default().add(&v); }
        }
        Ok(line_start) // an unfinished last line gets re-read next time
    }
}

fn walk(d: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() { walk(&e.path(), out); }
        else if e.file_name().to_string_lossy().ends_with(".jsonl") {
            if let Ok(m) = e.metadata() { out.push((e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH))); }
        }
    }
}

fn load_rates(w: &mut Work, f: &Path, m: Option<SystemTime>) -> Result<(), String> {
    let t = std::fs::read_to_string(f).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let Some(doc) = get(&v, &["document"]).and_then(|x| x.as_object()) else { return Ok(()) };
    w.rates.clear();
    for (k, x) in doc {
        let (Some(i), Some(o)) = (gn(x, &["input_cost_per_token"]), gn(x, &["output_cost_per_token"])) else { continue };
        let fast = gn(x, &["provider_specific_entry", "fast"]).filter(|f| *f > 0.0).unwrap_or(1.0);
        w.rates.insert(k.trim().to_lowercase(), Rate { input: i, output: o, read: gn(x, &["cache_read_input_token_cost"]).unwrap_or(i), write: gn(x, &["cache_creation_input_token_cost"]).unwrap_or(i), fast });
    }
    w.rates_mtime = m;
    Ok(())
}
fn price(w: &Work, r: &Row) -> f64 {
    if let Some(x) = r.reported { return x; }
    let mut key = r.model.unwrap_or("").trim().to_lowercase();
    if let Some(b) = key.find('[') { key.truncate(b); }
    let Some(rt) = w.rates.get(&key) else { return 0.0 };
    (r.unc as f64 * rt.input + r.cached as f64 * rt.read + r.write as f64 * rt.write + r.output as f64 * rt.output) * if r.fast { rt.fast } else { 1.0 }
}
fn add(w: &mut Work, days: &mut HashMap<String, DayTotals>, r: Row) {
    if let Some(k) = &r.dedupe { if !w.seen.insert(k.clone()) { return; } }
    let usd = price(w, &r);
    let d = days.entry(day_key(r.ms)).or_default();
    d.input += r.unc; d.cache_read += r.cached; d.cache_write += r.write; d.output += r.output; d.usd += usd;
}
fn has(h: &[u8], n: &[u8]) -> bool { h.windows(n.len()).any(|x| x == n) }
fn parse_line(w: &mut Work, days: &mut HashMap<String, DayTotals>, line: &[u8]) {
    if line.is_empty() || !has(line, b"\"usage\"") || !has(line, b"\"assistant\"") { return; }
    let Ok(o) = serde_json::from_slice::<Value>(line) else { return };
    if gs(&o, &["type"]).as_deref() != Some("assistant") { return; }
    let Some(msg) = get(&o, &["message"]) else { return };
    let Some(u) = get(msg, &["usage"]).filter(|x| !x.is_null()) else { return };
    let Some(model) = gs(msg, &["model"]).filter(|m| !m.is_empty() && m != "<synthetic>") else { return };
    let Some(ts) = gs(&o, &["timestamp"]).and_then(|t| parse_utc(&t)) else { return };
    let ms = ts.timestamp_millis();
    let (id, req) = (gs(msg, &["id"]), gs(&o, &["requestId"]));
    let key = if id.is_none() && req.is_none() { None } else { Some(format!("{}:{}", id.unwrap_or_default(), req.unwrap_or_default())) };
    let fast = gs(u, &["speed"]).as_deref() == Some("fast");
    let tok = |x: &Value, k: &str| gc(x, &[k]);
    add(w, days, Row { ms, model: Some(&model), unc: tok(u, "input_tokens"), cached: tok(u, "cache_read_input_tokens"), write: tok(u, "cache_creation_input_tokens"), output: tok(u, "output_tokens"), dedupe: key.clone(), reported: gn(&o, &["costUSD"]), fast });
    let Some(its) = get(u, &["iterations"]).and_then(|x| x.as_array()) else { return };
    for (i, it) in its.iter().enumerate() {
        if gs(it, &["type"]).as_deref() != Some("advisor_message") { continue; }
        let Some(am) = gs(it, &["model"]).filter(|m| !m.is_empty() && m != "<synthetic>") else { continue };
        add(w, days, Row { ms, model: Some(&am), unc: tok(it, "input_tokens"), cached: tok(it, "cache_read_input_tokens"), write: tok(it, "cache_creation_input_tokens"), output: tok(it, "output_tokens"),
            dedupe: key.as_ref().map(|k| format!("{k}:advisor:{i}")), reported: None, fast });
    }
}
