use crate::util::*;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const DEFAULT_CLIENT_ID: &str = "1567247850780364901";
pub const DEFAULT_IMAGE: &str = "https://cdn.discordapp.com/app-icons/1567247850780364901/1ae8932aa95a52e47a30a977496626e7.png?size=512";
pub const ALL_PAGES: [&str; 5] = ["activity", "agents", "tokens", "value", "usage"];
const TWEMOJI: &str = "https://cdn.jsdelivr.net/gh/jdecked/twemoji@15.1.0/assets/72x72/";

/// A linked machine: ssh/port/key for live state, userData/transcripts are its folders as this PC reads them.
#[derive(Clone, Debug, Default)]
pub struct RemoteConfig {
    pub name: String,
    pub ssh: Option<String>,
    pub key: Option<String>,
    pub user_data: Option<String>,
    pub transcripts: Option<String>,
    pub port: i64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub client_id: String,
    pub large_image_key: String,
    pub hub_url: String,
    pub show_project: bool,
    pub live_line: bool,
    pub idle_minutes: i64,
    pub page_seconds: i64,
    pub activity_type: i64,
    pub limit_percent: i64,
    pub value_days: i64,
    pub pages: Vec<String>,
    pub include_remotes: bool,
    pub remotes: Vec<RemoteConfig>,
    pub project_images: BTreeMap<String, String>,
    pub state_images: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut st = BTreeMap::new();
        st.insert("working".into(), format!("{TWEMOJI}1f7e2.png"));
        st.insert("idle".into(), format!("{TWEMOJI}26aa.png"));
        st.insert("limit".into(), format!("{TWEMOJI}1f534.png"));
        Config {
            client_id: DEFAULT_CLIENT_ID.into(),
            large_image_key: DEFAULT_IMAGE.into(),
            hub_url: String::new(),
            show_project: true,
            live_line: true,
            idle_minutes: 15,
            page_seconds: 15,
            activity_type: 3,
            limit_percent: 80,
            value_days: 30,
            pages: ALL_PAGES.iter().map(|s| s.to_string()).collect(),
            include_remotes: true,
            remotes: vec![],
            project_images: BTreeMap::new(),
            state_images: st,
        }
    }
}

fn clamp(v: Option<f64>, dflt: i64, lo: i64, hi: i64) -> i64 {
    match v { Some(x) => (x as i64).clamp(lo, hi), None => dflt }
}
fn map_into(v: Option<&Value>, into: &mut BTreeMap<String, String>) {
    if let Some(o) = v.and_then(|x| x.as_object()) {
        for (k, x) in o { if let Some(s) = x.as_str() { into.insert(k.clone(), s.to_string()); } }
    }
}

impl Config {
    pub fn file_path() -> std::path::PathBuf { path_of("config.json") }

    pub fn load() -> Config {
        let mut c = Config::default();
        let p = Config::file_path();
        if p.exists() {
            let parsed = std::fs::read_to_string(&p).map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()));
            match parsed {
                Ok(d) => {
                    if let Some(s) = gs(&d, &["clientId"]) { c.client_id = s; }
                    if let Some(s) = gs(&d, &["largeImageKey"]) { c.large_image_key = s; }
                    if let Some(s) = gs(&d, &["hubUrl"]) { c.hub_url = s; }
                    if let Some(b) = gb(&d, &["showProject"]) { c.show_project = b; }
                    if let Some(b) = gb(&d, &["liveLine"]) { c.live_line = b; }
                    c.idle_minutes = clamp(gn(&d, &["idleMinutes"]), c.idle_minutes, 1, 1440);
                    c.page_seconds = clamp(gn(&d, &["pageSeconds"]).or(gn(&d, &["pollSec"])), c.page_seconds, 15, 300);
                    c.activity_type = clamp(gn(&d, &["activityType"]), c.activity_type, 0, 5);
                    c.limit_percent = clamp(gn(&d, &["limitPercent"]), c.limit_percent, 1, 100);
                    c.value_days = clamp(gn(&d, &["valueDays"]), c.value_days, 1, 365);
                    if let Some(b) = gb(&d, &["includeRemotes"]) { c.include_remotes = b; }
                    if let Some(rs) = get(&d, &["remotes"]).and_then(|x| x.as_array()) {
                        for r in rs {
                            c.remotes.push(RemoteConfig {
                                name: gs(r, &["name"]).unwrap_or_else(|| "Remote".into()),
                                ssh: gs(r, &["ssh"]),
                                port: clamp(gn(r, &["port"]), 22, 1, 65535),
                                key: gs(r, &["key"]),
                                user_data: gs(r, &["userData"]),
                                transcripts: gs(r, &["transcripts"]),
                            });
                        }
                    }
                    if let Some(p) = get(&d, &["pages"]).and_then(|x| x.as_array()) {
                        c.pages = p.iter().filter_map(|x| x.as_str()).filter(|x| ALL_PAGES.contains(x)).map(|x| x.to_string()).collect();
                    }
                    map_into(get(&d, &["projectImages"]), &mut c.project_images);
                    map_into(get(&d, &["stateImages"]), &mut c.state_images);
                }
                Err(e) => {
                    // Never write defaults over a file the user can still fix.
                    log(&format!("config.json unreadable, using defaults: {e}"));
                    return c;
                }
            }
        }
        c.save();
        c
    }

    pub fn to_json(&self) -> Value {
        let remotes: Vec<Value> = self.remotes.iter().map(|r| json!({
            "name": r.name, "ssh": r.ssh, "key": r.key, "userData": r.user_data, "transcripts": r.transcripts, "port": r.port
        })).collect();
        let mut m = Map::new();
        m.insert("clientId".into(), json!(self.client_id));
        m.insert("activityType".into(), json!(self.activity_type));
        m.insert("pages".into(), json!(self.pages));
        m.insert("showProject".into(), json!(self.show_project));
        m.insert("liveLine".into(), json!(self.live_line));
        m.insert("idleMinutes".into(), json!(self.idle_minutes));
        m.insert("pageSeconds".into(), json!(self.page_seconds));
        m.insert("limitPercent".into(), json!(self.limit_percent));
        m.insert("valueDays".into(), json!(self.value_days));
        m.insert("largeImageKey".into(), json!(self.large_image_key));
        m.insert("hubUrl".into(), json!(self.hub_url));
        m.insert("projectImages".into(), json!(self.project_images));
        m.insert("stateImages".into(), json!(self.state_images));
        m.insert("includeRemotes".into(), json!(self.include_remotes));
        m.insert("remotes".into(), Value::Array(remotes));
        Value::Object(m)
    }

    pub fn save(&self) {
        let text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_default() + "\n";
        if let Err(e) = std::fs::write(Config::file_path(), text) {
            log(&format!("config save failed: {e}"));
        }
    }
}
