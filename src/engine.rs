// Page text, the presence loop, and --dump.
use crate::config::{Config, ALL_PAGES};
use crate::discord::Discord;
use crate::remote::RemoteLink;
use crate::state::{self, T3State};
use crate::usage::{PlanData, PlanUsage, UsageScan, UsageSource};
use crate::util::*;
use chrono::{DateTime, Local, Utc};
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct PageText { pub name: String, pub details: Option<String>, pub state: Option<String>, pub hover: Option<String> }

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub conn: String,
    pub user: Option<String>,
    pub page_name: Option<String>,
    pub none_reason: Option<String>,
    pub project: Option<String>,
    pub details: Option<String>,
    pub state: Option<String>,
    pub hover: Option<String>,
    pub image: Option<String>,
    pub overlay: Option<String>,
    pub type_word: String,
    pub coverage: String,
    pub paused: bool,
    pub start: Option<DateTime<Local>>,
    pub pages: Vec<String>,
}

#[derive(Default)]
struct Loop {
    page: i64,
    session_start: Option<DateTime<Local>>,
    last_key: String,
    sent_any: bool,
    last_send: Option<Instant>,
    next_connect: Option<Instant>,
}

pub struct Engine {
    pub cfg: Mutex<Config>,
    pub rpc: Arc<Discord>,
    pub plan: Arc<PlanUsage>,
    pub scan: Arc<UsageScan>,
    links: Vec<Arc<RemoteLink>>,
    pub paused: AtomicBool,
    jump: Mutex<Option<String>>,
    lp: Mutex<Loop>,
    busy: AtomicBool,
    stopping: AtomicBool,
    timer_gen: (Mutex<u64>, Condvar),
    retry_gen: AtomicU64,
    pub snapshot: Mutex<Option<Snapshot>>,
    pub on_update: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    pub test_drop_after: AtomicU64,
    started: Instant,
    dropped: AtomicBool,
}

pub fn type_word(t: i64) -> &'static str {
    match t { 0 => "Playing", 2 => "Listening to", 5 => "Competing in", _ => "Watching" }
}
fn shown(p: &str) -> bool {
    // The card already says T3 Code, so a project with that name adds nothing.
    !p.is_empty() && p != "No project" && !p.eq_ignore_ascii_case("T3 Code") && !p.eq_ignore_ascii_case("t3code")
}
fn agents(s: &T3State) -> String {
    format!("{} agent{} running{}", s.running, if s.running == 1 { "" } else { "s" }, if s.subagents > 0 { format!(" + {} sub", s.subagents) } else { String::new() })
}
/// "2x Opus 5.5 high, Sonnet 5.5" cut to max chars with "+N more".
pub fn group_models(list: &[String], max: usize) -> String {
    let mut groups: Vec<(String, usize)> = vec![];
    for x in list {
        match groups.iter_mut().find(|g| &g.0 == x) { Some(g) => g.1 += 1, None => groups.push((x.clone(), 1)) }
    }
    groups.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())).then_with(|| a.0.cmp(&b.0)));
    let parts: Vec<String> = groups.iter().map(|(k, n)| if *n > 1 { format!("{n}x {k}") } else { k.clone() }).collect();
    let mut sb = String::new(); let mut i = 0;
    while i < parts.len() {
        let reserve = if i + 1 < parts.len() { 12 } else { 0 };
        let len = sb.chars().count();
        if len + if len > 0 { 2 } else { 0 } + parts[i].chars().count() + reserve > max { break; }
        if !sb.is_empty() { sb.push_str(", "); }
        sb.push_str(&parts[i]); i += 1;
    }
    if i < parts.len() { sb.push_str(&format!("{}+{} more", if sb.is_empty() { "" } else { ", " }, parts.len() - i)); }
    sb
}

impl Engine {
    pub fn new(c: Config) -> Arc<Engine> {
        let (t3_url, mut key) = PlanUsage::from_t3_settings();
        let url = if c.hub_url.trim().is_empty() { t3_url } else { Some(c.hub_url.trim().to_string()) };
        if let Ok(k) = std::fs::read_to_string(path_of("hub.key")) { let k = k.trim().to_string(); if !k.is_empty() { key = Some(k); } }
        let plan = PlanUsage::new(url, key);
        let mut scan = UsageScan::new();
        let mut links = vec![];
        for r in &c.remotes {
            links.push(RemoteLink::new(r.clone()));
            if let (Some(ud), Some(tr)) = (r.user_data.as_deref().filter(|x| !x.is_empty()), r.transcripts.as_deref().filter(|x| !x.is_empty())) {
                scan.sources.push(Arc::new(UsageSource::new(PathBuf::from(ud), Some(PathBuf::from(tr)))));
                plan.user_data_dirs.lock().unwrap().push(PathBuf::from(ud));
            }
        }
        let e = Arc::new(Engine {
            cfg: Mutex::new(c), rpc: Discord::new(), plan: Arc::new(plan), scan: Arc::new(scan), links,
            paused: AtomicBool::new(false), jump: Mutex::new(None), lp: Mutex::new(Loop { page: -1, ..Default::default() }),
            busy: AtomicBool::new(false), stopping: AtomicBool::new(false), timer_gen: (Mutex::new(0), Condvar::new()),
            retry_gen: AtomicU64::new(0), snapshot: Mutex::new(None), on_update: Mutex::new(None),
            test_drop_after: AtomicU64::new(0), started: Instant::now(), dropped: AtomicBool::new(false),
        });
        let w = Arc::downgrade(&e);
        *e.rpc.changed.lock().unwrap() = Some(Box::new(move || {
            if let Some(e) = w.upgrade() {
                { let mut l = e.lp.lock().unwrap(); l.last_key.clear(); l.sent_any = false; }
                e.poke();
            }
        }));
        e
    }
    pub fn cfg(&self) -> Config { self.cfg.lock().unwrap().clone() }
    pub fn has_remotes(&self) -> bool { !self.links.is_empty() }
    /// Follows the Settings toggle: applies to live state, token and value totals. Plan limits are per account anyway.
    pub fn sync_remotes(&self) {
        let inc = self.cfg.lock().unwrap().include_remotes;
        self.scan.include_remotes.store(inc, Ordering::Relaxed);
        for l in &self.links { if inc { l.start(); } else { l.stop(); } }
    }
    fn remotes_counted(&self, c: &Config) -> bool { c.include_remotes && !self.links.is_empty() }
    fn coverage(&self, c: &Config) -> String {
        if !self.remotes_counted(c) { return "Covers this PC".into(); }
        let away: Vec<&str> = self.links.iter().filter(|l| !l.online()).map(|l| l.cfg.name.as_str()).collect();
        let names = self.links.iter().map(|l| l.cfg.name.as_str()).collect::<Vec<_>>().join(", ");
        if !away.is_empty() { return format!("{} offline, its agents aren't counted right now", away.join(", ")); }
        if !self.scan.all_loaded() { return format!("Loading usage from {names}"); }
        format!("Covers all environments: this PC, {names}")
    }
    fn read_all(&self, c: &Config) -> T3State {
        let mut all = vec![state::read()];
        if self.remotes_counted(c) { for l in &self.links { if let Some(s) = l.state() { all.push(s); } } }
        state::merge(all)
    }

    fn project_name(c: &Config, s: &T3State) -> Option<String> {
        if !c.show_project { return None; }
        let run = if s.running > 0 { s.running_projects.iter().find(|p| shown(p)).cloned() } else { None };
        run.or_else(|| s.project.clone().filter(|p| shown(p)))
    }
    /// Every project with a running agent, cut to fit Discord's 128 chars.
    fn projects_text(c: &Config, s: &T3State) -> Option<String> {
        if !c.show_project { return None; }
        let list: Vec<String> = if s.running > 0 { s.running_projects.iter().filter(|p| shown(p)).cloned().collect() } else { vec![] };
        if list.is_empty() { return Self::project_name(c, s); }
        Some(group_models(&list, 128 - "Working in ".len()))
    }

    /// Each page says a thing once: no complements, no totals repeating the parts, no app name (the card header has it).
    pub fn page_for(&self, c: &Config, plan: &PlanData, name: &str, s: &T3State) -> Option<PageText> {
        let pt = |d: Option<String>, st: Option<String>, h: Option<String>| Some(PageText { name: name.into(), details: d, state: st, hover: h });
        match name {
            "activity" => {
                let proj = Self::projects_text(c, s);
                let word = if s.running > 0 { "Working" } else { "Reviewing" };
                let state = if s.running > 0 {
                    if c.live_line { Some(s.live().unwrap_or_else(|| "Thinking".into())) }
                    else if c.pages.iter().any(|p| p == "agents") { None } else { Some(agents(s)) }
                } else { Some(format!("{} session{} open", s.sessions, if s.sessions == 1 { "" } else { "s" })) };
                pt(Some(match proj { Some(p) => format!("{word} in {p}"), None => word.into() }), state, Some(s.model.clone()))
            }
            "agents" => {
                if s.running <= 0 { return None; }
                // His rotation may be Agents only, so the running projects show here too.
                let list: Vec<String> = if c.show_project { s.running_projects.iter().filter(|p| shown(p)).cloned().collect() } else { vec![] };
                let head = agents(s);
                let details = if list.is_empty() { head } else { let pre = format!("{head} in "); format!("{pre}{}", group_models(&list, 128 - pre.chars().count())) };
                pt(Some(details), Some(group_models(&s.running_models, 128)), if s.ctx_max > 0 { Some(format!("Context {} of {}", tok(s.ctx_used), tok(s.ctx_max))) } else { None })
            }
            "tokens" => {
                let (t, w) = (self.scan.range(1), self.scan.range(7));
                if t.total() == 0 && w.total() == 0 { return None; }
                pt(Some(format!("Today: {} in, {} out", tok(t.input), tok(t.output))), Some(format!("Cache: {} read, {} write", tok(t.cache_read), tok(t.cache_write))), Some(format!("Last 7 days: {} tokens", tok(w.total()))))
            }
            "value" => {
                let (t, r) = (self.scan.range(1), self.scan.range(c.value_days));
                if r.usd <= 0.0 { return None; }
                let cov = if !self.remotes_counted(c) { "This PC." } else if self.scan.all_loaded() { "All environments." } else { "This PC only right now." };
                pt(Some(format!("Token value today: {}", usd(t.usd))), Some(format!("Last {} days: {}", c.value_days, usd(r.usd))), Some(format!("API list prices, not a bill. {cov}")))
            }
            "usage" => {
                let acc = &plan.accounts;
                if acc.len() > 1 {
                    let line = |label: &str, pick: &dyn Fn(&crate::usage::PlanAccount) -> Option<crate::usage::PlanWindow>| -> Option<String> {
                        let ws: Vec<_> = acc.iter().map(pick).collect();
                        if ws.iter().all(|x| x.is_none()) { return None; }
                        let used = ws.iter().map(|x| x.as_ref().map_or("?".into(), |w| format!("{}%", w.used))).collect::<Vec<_>>().join(" / ");
                        let reset = ws.iter().map(|x| x.as_ref().map_or("?".into(), |w| dur(w.in_min))).collect::<Vec<_>>().join(" / ");
                        Some(format!("{label} limits: {used} used, resets {reset}"))
                    };
                    return pt(line("5h", &|a| a.five_hour.clone()), line("Week", &|a| a.week.clone()), Some(format!("{} accounts pooled through the hub", acc.len())));
                }
                let (f, w) = (&plan.five_hour, &plan.week);
                if f.is_none() && w.is_none() { return None; }
                pt(f.as_ref().map(|f| format!("5h limit: {}% used, resets in {}", f.used, dur(f.in_min))), w.as_ref().map(|w| format!("Week limit: {}% used, resets in {}", w.used, dur(w.in_min))), Some(s.model.clone()))
            }
            _ => None,
        }
    }
    fn image_for(c: &Config, project: Option<&str>, state: &str) -> String {
        let m = &c.project_images;
        let nonblank = |v: Option<&String>| v.filter(|x| !x.trim().is_empty()).cloned();
        if let Some(p) = project {
            if let Some(v) = nonblank(m.get(&format!("{p}|{state}"))).or_else(|| nonblank(m.get(p))) { return v; }
        }
        nonblank(m.get(&format!("*|{state}"))).unwrap_or_else(|| c.large_image_key.clone())
    }
    fn state_key(c: &Config, plan: &PlanData, s: &T3State) -> &'static str {
        let near = plan.five_hour.as_ref().map_or(false, |w| w.used >= c.limit_percent) || plan.week.as_ref().map_or(false, |w| w.used >= c.limit_percent);
        if near { "limit" } else if s.running > 0 { "working" } else { "idle" }
    }
    fn activity(c: &Config, plan: &PlanData, p: &PageText, s: &T3State, start: DateTime<Local>) -> Value {
        let mut a = Map::new();
        a.insert("type".into(), json!(c.activity_type));
        if let Some(d) = p.details.as_deref().filter(|x| !x.is_empty()) { a.insert("details".into(), json!(cut(d))); }
        if let Some(d) = p.state.as_deref().filter(|x| !x.is_empty()) { a.insert("state".into(), json!(cut(d))); }
        a.insert("timestamps".into(), json!({"start": start.timestamp()}));
        let state = Self::state_key(c, plan, s);
        let mut assets = Map::new();
        let img = Self::image_for(c, Self::project_name(c, s).as_deref(), state);
        if !img.is_empty() { assets.insert("large_image".into(), json!(img)); }
        if let Some(h) = p.hover.as_deref().filter(|x| x.chars().count() >= 2) { assets.insert("large_text".into(), json!(cut(h))); }
        if let Some(small) = c.state_images.get(state).filter(|x| !x.trim().is_empty()) {
            assets.insert("small_image".into(), json!(small));
            if state == "limit" { assets.insert("small_text".into(), json!("Near plan limit")); }
        }
        if !assets.is_empty() { a.insert("assets".into(), Value::Object(assets)); }
        Value::Object(a)
    }

    pub fn dump(self: &Arc<Self>) -> String {
        let c = self.cfg();
        self.plan.refresh();
        self.scan.include_remotes.store(c.include_remotes, Ordering::Relaxed);
        self.scan.refresh_now();
        if self.remotes_counted(&c) {
            // wait for the first line from each linked machine, 20 s at most
            self.sync_remotes();
            for _ in 0..200 { if self.links.iter().all(|l| l.online()) { break; } std::thread::sleep(Duration::from_millis(100)); }
        }
        let s = self.read_all(&c);
        let plan = self.plan.get();
        let (t, w, v) = (self.scan.range(1), self.scan.range(7), self.scan.range(c.value_days));
        let head = json!({"error": s.error, "running": s.running, "subagents": s.subagents, "sessions": s.sessions, "threads": s.threads,
            "project": s.project, "model": s.model, "live": s.live(), "runningModels": s.running_models, "runningProjects": s.running_projects,
            "ctx": format!("{}/{}", s.ctx_used, s.ctx_max), "today": t.to_json(), "week": w.to_json(), "valueWindow": v.to_json(),
            "scanError": self.scan.error(), "plan5h": plan.five_hour, "planWeek": plan.week, "planSource": if plan.from_hub() { "hub" } else { "logs" },
            "planAccounts": plan.accounts, "hubError": plan.hub_error, "coverage": self.coverage(&c)});
        let mut out = head.to_string() + "\n";
        for n in ALL_PAGES {
            let p = self.page_for(&c, &plan, n, &s);
            out += &format!("{} {}\n", n, p.map_or("null".to_string(), |p| Self::activity(&c, &plan, &p, &s, Local::now()).to_string()));
        }
        for l in &self.links { l.stop(); }
        out
    }

    // --- loop ---
    pub fn start(self: &Arc<Self>) {
        self.sync_remotes();
        let me = self.clone();
        std::thread::Builder::new().name("presence-timer".into()).spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            me.tick(true);
            loop {
                let secs = me.cfg.lock().unwrap().page_seconds.max(15) as u64;
                let g = me.timer_gen.0.lock().unwrap();
                let gen0 = *g;
                let (g, res) = me.timer_gen.1.wait_timeout_while(g, Duration::from_secs(secs), |x| *x == gen0).unwrap();
                drop(g);
                if me.stopping.load(Ordering::SeqCst) { return; }
                if res.timed_out() { me.tick(true); }
            }
        }).ok();
    }
    pub fn reschedule(self: &Arc<Self>) {
        { *self.timer_gen.0.lock().unwrap() += 1; } self.timer_gen.1.notify_all();
        self.poke();
    }
    pub fn poke(self: &Arc<Self>) { let me = self.clone(); std::thread::spawn(move || me.tick(false)); }
    /// Chip click: show that page now and give it a full pageSeconds before rotating on.
    pub fn jump_to(self: &Arc<Self>, name: &str) { *self.jump.lock().unwrap() = Some(name.into()); self.reschedule(); }
    pub fn set_paused(self: &Arc<Self>, p: bool) { self.paused.store(p, Ordering::SeqCst); self.poke(); }
    pub fn is_paused(&self) -> bool { self.paused.load(Ordering::SeqCst) }
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        { *self.timer_gen.0.lock().unwrap() += 1; } self.timer_gen.1.notify_all();
        for l in &self.links { l.stop(); }
        if self.rpc.is_ready() && self.rpc.set_activity(None).is_ok() && !self.rpc.wait_clear_ack(1500) { log("no clear ack from discord"); }
        self.rpc.close();
    }

    fn tick(self: &Arc<Self>, advance: bool) {
        if self.stopping.load(Ordering::SeqCst) || self.busy.swap(true, Ordering::SeqCst) { return; }
        self.tick_inner(advance);
        self.busy.store(false, Ordering::SeqCst);
    }
    fn tick_inner(self: &Arc<Self>, advance: bool) {
        let c = self.cfg();
        let td = self.test_drop_after.load(Ordering::SeqCst);
        if td > 0 && !self.dropped.load(Ordering::SeqCst) && self.started.elapsed().as_secs() >= td {
            self.dropped.store(true, Ordering::SeqCst); log("TEST: dropping own pipe"); self.rpc.close(); self.lp.lock().unwrap().next_connect = None;
        }
        let connect = { let mut l = self.lp.lock().unwrap(); if !self.rpc.is_open() && l.next_connect.map_or(true, |t| Instant::now() >= t) { l.next_connect = Some(Instant::now() + Duration::from_secs(c.page_seconds.max(15) as u64)); true } else { false } };
        if connect && !c.client_id.is_empty() { self.rpc.try_connect(&c.client_id); }
        let plan_ref = self.plan.clone();
        std::thread::spawn(move || plan_ref.refresh());
        self.scan.refresh_in_background();
        let s = self.read_all(&c);
        let plan = self.plan.get();
        let paused = self.is_paused();
        let mut snap = Snapshot { coverage: self.coverage(&c), paused, type_word: type_word(c.activity_type).into(), project: Self::project_name(&c, &s), ..Default::default() };
        let mut act: Option<Value> = None; let mut shown_page: Option<PageText> = None;
        let mut l = self.lp.lock().unwrap();
        if s.error.is_some() { snap.none_reason = Some("Can't read T3's data right now".into()); }
        else if s.running <= 0 && s.last_at.map_or(true, |t| (Utc::now() - t).num_seconds() as f64 / 60.0 > c.idle_minutes as f64) {
            l.session_start = None;
            snap.none_reason = Some(format!("No agents for {} min, presence cleared", c.idle_minutes));
        } else {
            let start = *l.session_start.get_or_insert_with(Local::now);
            for n in &c.pages { if self.page_for(&c, &plan, n, &s).is_some() { snap.pages.push(n.clone()); } }
            if snap.pages.is_empty() { snap.none_reason = Some("No pages to show".into()); }
            else {
                let j = self.jump.lock().unwrap().take();
                match j.and_then(|j| snap.pages.iter().position(|p| *p == j)) {
                    Some(ji) => l.page = ji as i64,
                    None => if advance { l.page += 1; },
                }
                let name = snap.pages[(l.page.max(0) as usize) % snap.pages.len()].clone();
                if let Some(p) = self.page_for(&c, &plan, &name, &s) {
                    act = Some(Self::activity(&c, &plan, &p, &s, start));
                    snap.start = Some(start);
                    shown_page = Some(p);
                }
            }
        }
        if let Some(p) = &shown_page {
            snap.page_name = Some(p.name.clone()); snap.details = p.details.clone(); snap.state = p.state.clone(); snap.hover = p.hover.clone();
            let sk = Self::state_key(&c, &plan, &s);
            snap.image = Some(Self::image_for(&c, snap.project.as_deref(), sk)); snap.overlay = Some(sk.into());
        }
        let wanted = if paused { None } else { act };
        let key = match &wanted {
            None => "null".to_string(),
            Some(v) => { let mut x = v.clone(); if let Some(o) = x.as_object_mut() { o.remove("timestamps"); } x.to_string() }
        };
        // Discord allows 5 updates per 20 s: settings pokes wait for the next tick unless they clear or follow a connect.
        let since = l.last_send.map_or(f64::MAX, |t| t.elapsed().as_secs_f64());
        if self.rpc.is_ready() && (key != l.last_key || !l.sent_any) && (!l.sent_any || wanted.is_none() || since >= 14.0) {
            match self.rpc.set_activity(wanted.as_ref()) {
                Ok(()) => {
                    if !l.sent_any || wanted.is_none() {
                        log(&match &shown_page { Some(p) if wanted.is_some() => format!("presence on: {}", p.name), _ => "presence cleared".into() });
                    }
                    l.last_key = key; l.sent_any = true; l.last_send = Some(Instant::now());
                }
                Err(e) => { log(&format!("send failed: {e}")); drop(l); self.rpc.close(); l = self.lp.lock().unwrap(); }
            }
        } else if self.rpc.is_ready() && key != l.last_key {
            // rate limited: resend once the 14 s gap is up so a chip click still reaches Discord
            let wait = ((14.0 - since).max(0.0) * 1000.0) as u64 + 200;
            let gen = self.retry_gen.fetch_add(1, Ordering::SeqCst) + 1;
            let me = self.clone();
            std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(wait.max(500))); if me.retry_gen.load(Ordering::SeqCst) == gen { me.tick(false); } });
        }
        drop(l);
        snap.conn = if self.rpc.is_ready() { "connected" } else if self.rpc.is_open() { "connecting" } else { "down" }.into();
        snap.user = self.rpc.user();
        *self.snapshot.lock().unwrap() = Some(snap);
        if let Some(u) = self.on_update.lock().unwrap().as_ref() { u(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_like_the_csharp_app() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(group_models(&v(&["T3 Presence", "PC Tools"]), 117), "PC Tools, T3 Presence");
        assert_eq!(group_models(&v(&["Opus 5.5 high", "Haiku 5.5 low", "Opus 5.5 high"]), 128), "2x Opus 5.5 high, Haiku 5.5 low");
        let many: Vec<String> = (0..20).map(|i| format!("Project number {i:02}")).collect();
        let s = group_models(&many, 117);
        assert!(s.chars().count() <= 117 && s.ends_with("more"), "{s}");
        assert_eq!(crate::util::tok(93210), "93.2k");
        assert_eq!(crate::util::tok(377283), "377k");
        assert_eq!(crate::util::usd(1202.554), "$1,202.55");
        assert_eq!(crate::util::cut(&"é".repeat(200)).chars().count(), 128);
    }
}
