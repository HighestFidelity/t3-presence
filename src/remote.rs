// One long-lived SSH session per linked machine, streaming JSON lines from src/remote-state.py.
// Quiet retries when the machine is away.
use crate::config::RemoteConfig;
use crate::state::{self, T3State};
use crate::util::*;
use base64::Engine as _;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SCRIPT: &str = include_str!("remote-state.py");
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct RemoteLink {
    pub cfg: RemoteConfig,
    stop: AtomicBool,
    running: AtomicBool,
    child: Mutex<Option<Child>>,
    latest: Mutex<Option<(T3State, Instant)>>,
}

impl RemoteLink {
    pub fn new(cfg: RemoteConfig) -> Arc<Self> {
        Arc::new(RemoteLink { cfg, stop: AtomicBool::new(false), running: AtomicBool::new(false), child: Mutex::new(None), latest: Mutex::new(None) })
    }
    pub fn online(&self) -> bool {
        self.latest.lock().unwrap().as_ref().map_or(false, |(s, at)| s.error.is_none() && at.elapsed().as_secs() < 45)
    }
    pub fn state(&self) -> Option<T3State> {
        if !self.online() { return None; }
        self.latest.lock().unwrap().as_ref().map(|x| x.0.clone())
    }
    pub fn start(self: &Arc<Self>) {
        if self.cfg.ssh.as_deref().unwrap_or("").is_empty() || self.running.swap(true, Ordering::SeqCst) { return; }
        self.stop.store(false, Ordering::SeqCst);
        let me = self.clone();
        std::thread::Builder::new().name(format!("remote-{}", self.cfg.name)).spawn(move || { me.run(); me.running.store(false, Ordering::SeqCst); }).ok();
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        *self.latest.lock().unwrap() = None;
        if let Some(c) = self.child.lock().unwrap().as_mut() { let _ = c.kill(); }
    }
    fn ssh() -> String {
        let p = std::path::PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into())).join(r"System32\OpenSSH\ssh.exe");
        if p.exists() { p.to_string_lossy().into() } else { "ssh".into() }
    }
    fn run(&self) {
        let mut announced = false;
        while !self.stop.load(Ordering::SeqCst) {
            if let Err(e) = self.session(&mut announced) { log(&format!("{} link error: {e}", self.cfg.name)); }
            *self.child.lock().unwrap() = None;
            if announced { announced = false; log(&format!("{} link down", self.cfg.name)); }
            for _ in 0..60 { if self.stop.load(Ordering::SeqCst) { break; } std::thread::sleep(Duration::from_secs(1)); }
        }
    }
    fn session(&self, announced: &mut bool) -> std::io::Result<()> {
        let mut key = self.cfg.key.clone().unwrap_or_default();
        if let Some(rest) = key.strip_prefix('~') { key = format!("{}{}", home().display(), rest); }
        let b64 = base64::engine::general_purpose::STANDARD.encode(SCRIPT);
        let mut cmd = Command::new(Self::ssh());
        cmd.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=2", "-p", &self.cfg.port.to_string()]);
        if !key.is_empty() { cmd.args(["-i", &key]); }
        cmd.arg(self.cfg.ssh.as_deref().unwrap_or(""))
            .arg(format!("python3 -u -c 'import base64;exec(base64.b64decode(\"{b64}\"))'"))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).creation_flags(CREATE_NO_WINDOW);
        let mut child = cmd.spawn()?;
        let out = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
        *self.child.lock().unwrap() = Some(child);
        if self.stop.load(Ordering::SeqCst) { if let Some(c) = self.child.lock().unwrap().as_mut() { let _ = c.kill(); } }
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if self.stop.load(Ordering::SeqCst) { break; }
            let Ok(v) = serde_json::from_str(&line) else { continue };
            *self.latest.lock().unwrap() = Some((state::from_json(&v), Instant::now()));
            if !*announced { *announced = true; log(&format!("{} link up", self.cfg.name)); }
        }
        if let Some(c) = self.child.lock().unwrap().as_mut() { let _ = c.kill(); let _ = c.wait(); }
        Ok(())
    }
}
