// Discord IPC over the local named pipe. Application ID only, no token.
// A synchronous pipe handle blocks writes while a read is pending, so the reader peeks first and only reads
// whole frames that have already arrived, under the same lock the writer uses.
use crate::util::*;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

struct Pipe { id: u64, file: Mutex<File> }

pub struct Discord {
    pipe: Mutex<Option<Arc<Pipe>>>,
    next_id: AtomicU64,
    pub ready: AtomicBool,
    pub user: Mutex<Option<String>>,
    clear_ack: (Mutex<bool>, Condvar),
    pub changed: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

fn frame(op: u32, body: &Value) -> Vec<u8> {
    let j = serde_json::to_vec(body).unwrap_or_default();
    let mut b = Vec::with_capacity(8 + j.len());
    b.extend_from_slice(&op.to_le_bytes());
    b.extend_from_slice(&(j.len() as u32).to_le_bytes());
    b.extend_from_slice(&j);
    b
}
fn available(f: &File) -> Option<u32> {
    let mut avail = 0u32;
    let ok = unsafe { PeekNamedPipe(f.as_raw_handle() as _, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut()) };
    if ok == 0 { None } else { Some(avail) }
}

impl Discord {
    pub fn new() -> Arc<Self> {
        Arc::new(Discord { pipe: Mutex::new(None), next_id: AtomicU64::new(1), ready: AtomicBool::new(false), user: Mutex::new(None),
            clear_ack: (Mutex::new(false), Condvar::new()), changed: Mutex::new(None) })
    }
    pub fn is_open(&self) -> bool { self.pipe.lock().unwrap().is_some() }
    pub fn is_ready(&self) -> bool { self.ready.load(Ordering::SeqCst) }
    pub fn user(&self) -> Option<String> { self.user.lock().unwrap().clone() }
    fn fire_changed(&self) { if let Some(c) = self.changed.lock().unwrap().as_ref() { c(); } }

    pub fn try_connect(self: &Arc<Self>, client_id: &str) -> bool {
        for i in 0..10 {
            let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(format!(r"\\.\pipe\discord-ipc-{i}")) else { continue };
            let p = Arc::new(Pipe { id: self.next_id.fetch_add(1, Ordering::SeqCst), file: Mutex::new(f) });
            *self.pipe.lock().unwrap() = Some(p.clone());
            let me = self.clone(); let rp = p.clone();
            std::thread::Builder::new().name("discord-read".into()).spawn(move || me.read_loop(rp)).ok();
            if self.send(0, &json!({"v": 1, "client_id": client_id})).is_ok() { return true; }
            self.drop_pipe(&p);
        }
        false
    }
    pub fn send(&self, op: u32, body: &Value) -> std::io::Result<()> {
        let p = self.pipe.lock().unwrap().clone().ok_or_else(|| std::io::Error::other("no pipe"))?;
        let mut f = p.file.lock().unwrap();
        f.write_all(&frame(op, body))?;
        f.flush()
    }
    pub fn set_activity(&self, activity: Option<&Value>) -> std::io::Result<()> {
        let nonce = format!("{:x}{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(), self.next_id.fetch_add(1, Ordering::SeqCst));
        if activity.is_none() { *self.clear_ack.0.lock().unwrap() = false; }
        self.send(1, &json!({"cmd": "SET_ACTIVITY", "args": {"pid": std::process::id(), "activity": activity}, "nonce": nonce}))
    }
    /// After set_activity(None): wait for Discord to confirm the clear.
    pub fn wait_clear_ack(&self, ms: u64) -> bool {
        let g = self.clear_ack.0.lock().unwrap();
        let (g, _) = self.clear_ack.1.wait_timeout_while(g, Duration::from_millis(ms), |done| !*done).unwrap();
        *g
    }

    fn read_frame(f: &mut File) -> std::io::Result<(u32, Value)> {
        let mut head = [0u8; 8];
        f.read_exact(&mut head)?;
        let op = u32::from_le_bytes(head[0..4].try_into().unwrap());
        let len = i32::from_le_bytes(head[4..8].try_into().unwrap());
        if !(0..=(1 << 24)).contains(&len) { return Err(std::io::Error::other("bad frame")); }
        let mut body = vec![0u8; len as usize];
        f.read_exact(&mut body)?;
        Ok((op, serde_json::from_slice(&body).unwrap_or(Value::Null)))
    }
    fn read_loop(self: Arc<Self>, p: Arc<Pipe>) {
        loop {
            // close() swapped the pipe out: stop polling so the handle drops and Discord sees the disconnect.
            if self.pipe.lock().unwrap().as_ref().map_or(true, |c| c.id != p.id) { break; }
            let got = {
                let mut f = p.file.lock().unwrap();
                match available(&f) {
                    None => break,
                    Some(n) if n >= 8 => Self::read_frame(&mut f).map(Some),
                    Some(_) => Ok(None),
                }
            };
            let (op, msg) = match got {
                Err(_) => break,
                Ok(None) => { std::thread::sleep(Duration::from_millis(40)); continue; }
                Ok(Some(x)) => x,
            };
            if op == 3 {
                if self.send(4, &msg).is_err() { break; }
            } else if op == 2 {
                log(&format!("discord closed: {}", gs(&msg, &["message"]).unwrap_or_default()));
                break;
            } else {
                let evt = gs(&msg, &["evt"]); let cmd = gs(&msg, &["cmd"]);
                if evt.as_deref() == Some("READY") {
                    let u = gs(&msg, &["data", "user", "username"]);
                    log(&format!("connected to Discord as {}", u.clone().unwrap_or_default()));
                    *self.user.lock().unwrap() = u;
                    self.ready.store(true, Ordering::SeqCst);
                    self.fire_changed();
                } else if evt.as_deref() == Some("ERROR") {
                    log(&format!("discord error: {}", gs(&msg, &["data", "message"]).unwrap_or_default()));
                } else if cmd.as_deref() == Some("SET_ACTIVITY") && get(&msg, &["data"]).map_or(true, |d| d.is_null()) {
                    log("discord confirmed presence cleared");
                    *self.clear_ack.0.lock().unwrap() = true;
                    self.clear_ack.1.notify_all();
                }
            }
        }
        self.drop_pipe(&p);
    }
    fn drop_pipe(&self, p: &Arc<Pipe>) {
        let was = {
            let mut cur = self.pipe.lock().unwrap();
            if cur.as_ref().map_or(false, |c| c.id == p.id) {
                *cur = None; self.ready.store(false, Ordering::SeqCst); *self.user.lock().unwrap() = None; true
            } else { false }
        };
        if was { log("pipe closed"); self.fire_changed(); }
    }
    pub fn close(&self) {
        let p = self.pipe.lock().unwrap().clone();
        if let Some(p) = p { self.drop_pipe(&p); }
    }
}
