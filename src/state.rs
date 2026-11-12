// T3's state DB, read-only. Only counts, model choices, item kinds and project titles:
// never thread titles, messages, paths or commands.
use crate::util::*;
use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::{types::ValueRef, Connection, OpenFlags};
use serde_json::Value;
use std::sync::OnceLock;

const P: &str = "orchestration_v2_projection_";

#[derive(Clone, Debug, Default)]
pub struct T3State {
    pub error: Option<String>,
    pub project: Option<String>,
    pub model: String,
    pub live_type: Option<String>,
    pub live_tool: Option<String>,
    pub live_status: Option<String>,
    pub live_at: Option<String>,
    pub waiting: bool,
    pub running: i64,
    pub subagents: i64,
    pub sessions: i64,
    pub threads: i64,
    pub ctx_used: i64,
    pub ctx_max: i64,
    pub last_at: Option<DateTime<Utc>>,
    pub running_models: Vec<String>,
    pub running_projects: Vec<String>,
}

impl T3State {
    pub fn new() -> Self { T3State { model: "?".into(), ..Default::default() } }
    pub fn err(e: String) -> Self { T3State { error: Some(e), ..T3State::new() } }
    pub fn live(&self) -> Option<String> {
        if self.running <= 0 { return None; }
        if self.waiting { return Some("Waiting on you".into()); }
        Some(if self.live_status.as_deref() == Some("running") {
            live_word(self.live_type.as_deref(), self.live_tool.as_deref()).to_string()
        } else { "Thinking".into() })
    }
}

pub fn model_name(id: Option<&str>) -> String {
    let Some(id) = id.filter(|s| !s.is_empty()) else { return "?".into() };
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"claude-(\w+?)-(\d+)-(\d+)").unwrap());
    match re.captures(id) {
        Some(c) => {
            let n = &c[1];
            let mut ch = n.chars();
            let first = ch.next().map(|x| x.to_uppercase().collect::<String>()).unwrap_or_default();
            format!("{}{} {}.{}", first, ch.as_str(), &c[2], &c[3])
        }
        None => id.to_string(),
    }
}
fn effort_of(sel: &Value) -> Option<String> {
    for o in get(sel, &["options"])?.as_array()? {
        if gs(o, &["id"]).as_deref() == Some("effort") { return gs(o, &["value"]); }
    }
    None
}
pub fn label(selection_json: Option<&str>) -> String {
    let Some(j) = selection_json.filter(|s| !s.is_empty()) else { return "?".into() };
    let Ok(sel) = serde_json::from_str::<Value>(j) else { return "?".into() };
    let e = effort_of(&sel);
    model_name(gs(&sel, &["model"]).as_deref()) + &e.filter(|x| !x.is_empty()).map(|x| format!(" {x}")).unwrap_or_default()
}

// Fixed wording per item KIND. Raw tool names, titles, paths and commands never get through here.
pub fn live_word(kind: Option<&str>, tool: Option<&str>) -> &'static str {
    if kind == Some("dynamic_tool") {
        let t = tool.unwrap_or("").to_ascii_lowercase();
        return match t.as_str() {
            "read" | "notebookread" => "Reading a file",
            "grep" | "glob" | "ls" => "Searching the code",
            "toolsearch" => "Loading tools",
            "edit" | "multiedit" | "write" | "notebookedit" => "Editing a file",
            "bash" | "powershell" => "Running a command",
            "webfetch" => "Reading the web",
            "websearch" => "Searching the web",
            "agent" | "task" => "Running a sub-agent",
            "skill" => "Using a skill",
            "askuserquestion" => "Waiting on you",
            _ => "Using a tool",
        };
    }
    match kind.unwrap_or("") {
        "command_execution" => "Running a command",
        "file_change" => "Editing a file",
        "web_search" => "Searching the web",
        "reasoning" | "user_message" => "Thinking",
        "assistant_message" => "Writing a reply",
        "subagent" => "Running a sub-agent",
        "user_input_request" => "Waiting on you",
        "compaction" => "Compacting context",
        "error" => "Hit an error",
        "checkpoint" => "Saving a checkpoint",
        _ => "Working",
    }
}

/// One line from src/remote-state.py.
pub fn from_json(o: &Value) -> T3State {
    if let Some(e) = gs(o, &["error"]) { return T3State::err(e); }
    let mut s = T3State::new();
    s.running = gc(o, &["running"]); s.subagents = gc(o, &["subagents"]);
    s.sessions = gc(o, &["sessions"]); s.threads = gc(o, &["threads"]);
    s.ctx_used = gc(o, &["ctxUsed"]); s.ctx_max = gc(o, &["ctxMax"]);
    s.project = gs(o, &["project"]);
    s.model = label(gs(o, &["lastSelection"]).as_deref());
    s.last_at = gs(o, &["lastAt"]).and_then(|x| parse_utc(&x));
    if let Some(a) = get(o, &["selections"]).and_then(|x| x.as_array()) {
        for x in a { s.running_models.push(label(x.as_str())); }
    }
    if let Some(a) = get(o, &["projects"]).and_then(|x| x.as_array()) {
        for x in a { if let Some(t) = x.as_str() { s.running_projects.push(t.into()); } }
    }
    s.waiting = gb(o, &["waiting"]).unwrap_or(false);
    s.live_type = gs(o, &["liveType"]); s.live_tool = gs(o, &["liveTool"]);
    s.live_status = gs(o, &["liveStatus"]); s.live_at = gs(o, &["liveAt"]);
    s
}

/// All environments as one: counts add up, project, model and context come from whichever ran last.
pub fn merge(all: Vec<T3State>) -> T3State {
    let ok: Vec<&T3State> = all.iter().filter(|x| x.error.is_none()).collect();
    if ok.is_empty() { return all.into_iter().next().unwrap_or_else(|| T3State::err("no data".into())); }
    let mut newest = ok[0];
    for x in &ok { if x.last_at > newest.last_at { newest = x; } }
    let mut m = T3State::new();
    m.project = newest.project.clone(); m.model = newest.model.clone(); m.last_at = newest.last_at;
    m.ctx_used = newest.ctx_used; m.ctx_max = newest.ctx_max;
    for x in &ok {
        m.running += x.running; m.subagents += x.subagents; m.sessions += x.sessions; m.threads += x.threads;
        m.running_models.extend(x.running_models.iter().cloned());
        for p in &x.running_projects { if !m.running_projects.contains(p) { m.running_projects.push(p.clone()); } }
        m.waiting |= x.waiting && x.running > 0;
    }
    let mut live: Option<&T3State> = None;
    for x in ok.iter().filter(|x| x.running > 0) {
        let k = x.live_at.as_deref().unwrap_or("");
        if live.map_or(true, |l| k > l.live_at.as_deref().unwrap_or("")) { live = Some(x); }
    }
    if let Some(l) = live {
        m.live_type = l.live_type.clone(); m.live_tool = l.live_tool.clone();
        m.live_status = l.live_status.clone(); m.live_at = l.live_at.clone();
    }
    m
}

fn text(v: ValueRef) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) | ValueRef::Blob(t) => Some(String::from_utf8_lossy(t).into_owned()),
    }
}
fn query(db: &Connection, sql: &str, cols: usize) -> rusqlite::Result<Vec<Vec<Option<String>>>> {
    let mut st = db.prepare(sql)?;
    let mut rows = st.query([])?;
    let mut out = vec![];
    while let Some(r) = rows.next()? {
        out.push((0..cols).map(|i| r.get_ref(i).ok().and_then(text)).collect());
    }
    Ok(out)
}
fn one(db: &Connection, sql: &str) -> rusqlite::Result<i64> {
    Ok(query(db, sql, 1)?.first().and_then(|r| r[0].as_ref()).and_then(|x| x.parse().ok()).unwrap_or(0))
}

pub fn read() -> T3State {
    match read_inner() { Ok(s) => s, Err(e) => T3State::err(e.to_string()) }
}
fn read_inner() -> rusqlite::Result<T3State> {
    let db = Connection::open_with_flags(t3_userdata().join("statev2.sqlite"), OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(std::time::Duration::from_millis(1500))?;
    let mut s = T3State::new();
    s.running = one(&db, &format!("select count(*) from {P}runs where status='running'"))?;
    s.subagents = one(&db, &format!("select count(*) from {P}subagents where status='running'"))?;
    s.sessions = one(&db, &format!("select count(*) from {P}provider_sessions where status='ready'"))?;
    s.threads = one(&db, &format!("select count(*) from {P}threads where archived_at is null and deleted_at is null"))?;
    let last = query(&db, &format!("select json_extract(r.payload_json,'$.modelSelection'), coalesce(r.completed_at,r.requested_at), coalesce(p.title,'No project') from {P}runs r join {P}threads t on t.thread_id=r.thread_id left join projection_projects p on p.project_id=t.project_id order by r.requested_at desc limit 1"), 3)?;
    if let Some(r) = last.first() {
        s.model = label(r[0].as_deref()); s.project = r[2].clone();
        s.last_at = r[1].as_deref().and_then(parse_utc);
    }
    for r in query(&db, &format!("select json_extract(payload_json,'$.modelSelection') from {P}runs where status='running'"), 1)? {
        s.running_models.push(label(r[0].as_deref()));
    }
    // Project titles of running runs, newest first: the activity page lists every one, not just the last.
    for r in query(&db, &format!("select coalesce(p.title,'No project') from {P}runs r join {P}threads t on t.thread_id=r.thread_id left join projection_projects p on p.project_id=t.project_id where r.status='running' group by 1 order by max(r.requested_at) desc"), 1)? {
        if let Some(t) = r[0].clone() { s.running_projects.push(t); }
    }
    let ctx = query(&db, &format!("select json_extract(payload_json,'$.tokenUsage.usedTokens'), json_extract(payload_json,'$.tokenUsage.maxTokens') from {P}provider_turns where provider_turn_id in (select provider_turn_id from {P}provider_turns order by started_at desc limit 40) and json_extract(payload_json,'$.tokenUsage.usedTokens') is not null order by started_at desc limit 1"), 2)?;
    if let Some(r) = ctx.first() {
        s.ctx_used = r[0].as_deref().and_then(|x| x.parse().ok()).unwrap_or(0);
        s.ctx_max = r[1].as_deref().and_then(|x| x.parse().ok()).unwrap_or(0);
    }
    if s.running > 0 {
        let live = format!("from {P}turn_items i where i.run_id in (select run_id from {P}runs where status='running')");
        s.waiting = one(&db, &format!("select count(*) {live} and i.status='running' and (i.type='user_input_request' or (i.type='dynamic_tool' and json_extract(i.payload_json,'$.toolName')='AskUserQuestion'))"))? > 0
            || one(&db, &format!("select count(*) from {P}runtime_requests where status not in ('resolved','cancelled','expired','failed') and thread_id in (select thread_id from {P}runs where status='running')"))? > 0;
        // Only the type column and, for tool items, the tool's name are read. Mapped to fixed words above.
        let it = query(&db, &format!("select i.type, case when i.type='dynamic_tool' then json_extract(i.payload_json,'$.toolName') end, i.status, i.updated_at {live} and i.type not in ('notification','system_notice','run_interrupt_request','run_interrupt_result','user_input_request') order by i.updated_at desc limit 1"), 4)?;
        if let Some(r) = it.first() {
            s.live_type = r[0].clone(); s.live_tool = r[1].clone(); s.live_status = r[2].clone(); s.live_at = r[3].clone();
        }
    }
    Ok(s)
}
