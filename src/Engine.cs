// Config, Discord IPC (local pipe, Application ID only, no token), page text, presence loop.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.IO.Pipes;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace T3Presence
{
    public class Config
    {
        public const string DefaultClientId = "1567247850780364901";
        public const string DefaultImage = "https://cdn.discordapp.com/app-icons/1567247850780364901/1ae8932aa95a52e47a30a977496626e7.png?size=512";
        public static readonly string[] AllPages = { "activity", "agents", "tokens", "value", "usage" };
        const string Twemoji = "https://cdn.jsdelivr.net/gh/jdecked/twemoji@15.1.0/assets/72x72/";

        public string clientId = DefaultClientId, largeImageKey = DefaultImage;
        public bool showProject = true, liveLine = true;
        public int idleMinutes = 15, pageSeconds = 15, activityType = 3, limitPercent = 80, valueDays = 30;
        public List<string> pages = new List<string>(AllPages);
        public bool includeRemotes = true;
        public List<RemoteConfig> remotes = new List<RemoteConfig>();
        public Dictionary<string, string> projectImages = new Dictionary<string, string>();
        public Dictionary<string, string> stateImages = new Dictionary<string, string> { { "working", Twemoji + "1f7e2.png" }, { "idle", Twemoji + "26aa.png" }, { "limit", Twemoji + "1f534.png" } };

        static readonly string ExeDir = AppDomain.CurrentDomain.BaseDirectory;
        public static string PathOf(string name) { return Path.Combine(ExeDir, name); }
        public static string FilePath { get { return PathOf("config.json"); } }

        public static Config Load()
        {
            var c = new Config(); bool fresh = !File.Exists(FilePath);
            if (!fresh)
            {
                try
                {
                    var d = Json.Parse(File.ReadAllText(FilePath));
                    c.clientId = Json.Str(Json.Get(d, "clientId")) ?? c.clientId;
                    c.largeImageKey = Json.Str(Json.Get(d, "largeImageKey")) ?? c.largeImageKey;
                    var b = Json.Get(d, "showProject"); if (b is bool) c.showProject = (bool)b;
                    b = Json.Get(d, "liveLine"); if (b is bool) c.liveLine = (bool)b;
                    c.idleMinutes = Clamp(Json.Num(Json.Get(d, "idleMinutes")), c.idleMinutes, 1, 1440);
                    c.pageSeconds = Clamp(Json.Num(Json.Get(d, "pageSeconds")) ?? Json.Num(Json.Get(d, "pollSec")), c.pageSeconds, 15, 300);
                    c.activityType = Clamp(Json.Num(Json.Get(d, "activityType")), c.activityType, 0, 5);
                    c.limitPercent = Clamp(Json.Num(Json.Get(d, "limitPercent")), c.limitPercent, 1, 100);
                    c.valueDays = Clamp(Json.Num(Json.Get(d, "valueDays")), c.valueDays, 1, 365);
                    b = Json.Get(d, "includeRemotes"); if (b is bool) c.includeRemotes = (bool)b;
                    var rs = Json.Get(d, "remotes") as object[];
                    if (rs != null) foreach (var r in rs)
                        {
                            var rc = new RemoteConfig { name = Json.Str(Json.Get(r, "name")) ?? "Remote", ssh = Json.Str(Json.Get(r, "ssh")), port = Clamp(Json.Num(Json.Get(r, "port")), 22, 1, 65535), key = Json.Str(Json.Get(r, "key")), userData = Json.Str(Json.Get(r, "userData")), transcripts = Json.Str(Json.Get(r, "transcripts")) };
                            c.remotes.Add(rc);
                        }
                    var p = Json.Get(d, "pages") as object[];
                    if (p != null) c.pages = p.OfType<string>().Where(x => AllPages.Contains(x)).ToList();
                    Map(Json.Get(d, "projectImages"), c.projectImages);
                    Map(Json.Get(d, "stateImages"), c.stateImages);
                }
                catch (Exception e) { Log.Write("config.json unreadable, using defaults: " + e.Message); }
            }
            c.Save();
            return c;
        }
        static void Map(object o, Dictionary<string, string> into)
        {
            var d = o as IDictionary<string, object>;
            if (d == null) return;
            foreach (var kv in d) { var s = kv.Value as string; if (s != null) into[kv.Key] = s; }
        }
        static int Clamp(double? v, int dflt, int lo, int hi) { return v.HasValue ? Math.Max(lo, Math.Min(hi, (int)v.Value)) : dflt; }
        public void Save()
        {
            try
            {
                var d = new Dictionary<string, object> { {"clientId",clientId},{"activityType",activityType},{"pages",pages},{"showProject",showProject},{"liveLine",liveLine},
                    {"idleMinutes",idleMinutes},{"pageSeconds",pageSeconds},{"limitPercent",limitPercent},{"valueDays",valueDays},{"largeImageKey",largeImageKey},
                    {"projectImages",projectImages},{"stateImages",stateImages},{"includeRemotes",includeRemotes},{"remotes",remotes} };
                File.WriteAllText(FilePath, Pretty(Json.Write(d)));
            }
            catch (Exception e) { Log.Write("config save failed: " + e.Message); }
        }
        static string Pretty(string j)
        {
            var sb = new StringBuilder(); int ind = 0; bool q = false;
            for (int i = 0; i < j.Length; i++)
            {
                char c = j[i];
                if (c == '"' && (i == 0 || j[i - 1] != '\\')) q = !q;
                if (q) { sb.Append(c); continue; }
                if (c == '{' || c == '[') { if (i + 1 < j.Length && (j[i + 1] == '}' || j[i + 1] == ']')) { sb.Append(c).Append(j[++i]); continue; } sb.Append(c).Append('\n').Append(' ', ++ind * 2); }
                else if (c == '}' || c == ']') sb.Append('\n').Append(' ', --ind * 2).Append(c);
                else if (c == ',') sb.Append(c).Append('\n').Append(' ', ind * 2);
                else if (c == ':') sb.Append(": ");
                else sb.Append(c);
            }
            return sb.ToString() + "\n";
        }
    }

    public static class Log
    {
        static readonly object G = new object();
        public static void Write(string s)
        {
            lock (G)
            {
                try
                {
                    var p = Config.PathOf("T3Presence.log");
                    var fi = new FileInfo(p);
                    if (fi.Exists && fi.Length > 256 * 1024) fi.Delete();
                    File.AppendAllText(p, DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + " " + s + "\n");
                }
                catch { }
            }
        }
    }

    // A linked machine: "ssh" + "port" + "key" for live state (run through the Windows ssh client), "userData" + "transcripts"
    // are that machine's T3 userdata and Claude projects folders as this PC can read them, normally a network share.
    public class RemoteConfig { public string name, ssh, key, userData, transcripts; public int port = 22; }

    // One long-lived SSH session streaming JSON lines from src/remote-state.py. Quiet retries when the machine is away.
    public class RemoteLink
    {
        public readonly RemoteConfig Cfg; Thread thread; volatile bool stop; Process proc; volatile T3State latest; DateTime latestAt; bool announced;
        public RemoteLink(RemoteConfig c) { Cfg = c; }
        public bool Online { get { return latest != null && latest.Error == null && (DateTime.UtcNow - latestAt).TotalSeconds < 45; } }
        public T3State State { get { return Online ? latest : null; } }
        static string Script()
        {
            using (var s = System.Reflection.Assembly.GetExecutingAssembly().GetManifestResourceStream("remote-state.py"))
            using (var r = new StreamReader(s)) return Convert.ToBase64String(Encoding.UTF8.GetBytes(r.ReadToEnd()));
        }
        static string Ssh()
        {
            var p = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "OpenSSH", "ssh.exe");
            return File.Exists(p) ? p : "ssh";
        }
        public void Start()
        {
            if (thread != null || string.IsNullOrEmpty(Cfg.ssh)) return;
            stop = false;
            thread = new Thread(Loop) { IsBackground = true, Name = "remote-" + Cfg.name };
            thread.Start();
        }
        public void Stop()
        {
            stop = true; thread = null; latest = null;
            var p = proc; if (p != null) try { p.Kill(); } catch { }
        }
        void Loop()
        {
            while (!stop)
            {
                try
                {
                    var key = Cfg.key;
                    if (!string.IsNullOrEmpty(key) && key.StartsWith("~")) key = T3.Home + key.Substring(1);
                    var args = "-T -o BatchMode=yes -o ConnectTimeout=8 -o ServerAliveInterval=30 -o ServerAliveCountMax=2 -p " + Cfg.port
                        + (string.IsNullOrEmpty(key) ? "" : " -i \"" + key + "\"") + " " + Cfg.ssh
                        + " \"python3 -u -c 'import base64;exec(base64.b64decode(\\\"" + Script() + "\\\"))'\"";
                    var psi = new ProcessStartInfo(Ssh(), args) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true, RedirectStandardInput = true };
                    using (var p = Process.Start(psi))
                    {
                        proc = p;
                        p.ErrorDataReceived += (a, b) => { };
                        p.BeginErrorReadLine();
                        string line;
                        while (!stop && (line = p.StandardOutput.ReadLine()) != null)
                        {
                            try { latest = T3.FromJson(Json.Parse(line)); latestAt = DateTime.UtcNow; } catch { continue; }
                            if (!announced) { announced = true; Log.Write(Cfg.name + " link up"); }
                        }
                        if (!stop) try { p.Kill(); } catch { }
                    }
                }
                catch (Exception e) { Log.Write(Cfg.name + " link error: " + e.Message); }
                proc = null;
                if (announced) { announced = false; Log.Write(Cfg.name + " link down"); }
                for (int i = 0; i < 60 && !stop; i++) Thread.Sleep(1000);
            }
        }
    }

    public class Discord
    {
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern bool WaitNamedPipe(string name, int ms);
        NamedPipeClientStream pipe; Thread reader; readonly object wl = new object();
        public volatile bool Ready; public volatile string User; public event Action Changed;
        public AutoResetEvent ClearAck = new AutoResetEvent(false);
        public bool Open { get { var p = pipe; return p != null && p.IsConnected; } }

        public bool TryConnect(string clientId)
        {
            for (int i = 0; i < 10; i++)
            {
                var name = "discord-ipc-" + i;
                if (!WaitNamedPipe(@"\\.\pipe\" + name, 1) && Marshal.GetLastWin32Error() == 2) continue;
                NamedPipeClientStream p = null;
                try
                {
                    p = new NamedPipeClientStream(".", name, PipeDirection.InOut, PipeOptions.Asynchronous);
                    p.Connect(300);
                    pipe = p;
                    reader = new Thread(() => ReadLoop(p)) { IsBackground = true, Name = "discord-read" };
                    reader.Start();
                    Send(0, new Dictionary<string, object> { { "v", 1 }, { "client_id", clientId } });
                    return true;
                }
                catch { if (p != null) try { p.Dispose(); } catch { } pipe = null; }
            }
            return false;
        }
        public void Send(int op, object body)
        {
            var j = Encoding.UTF8.GetBytes(Json.Write(body));
            var b = new byte[8 + j.Length];
            BitConverter.GetBytes(op).CopyTo(b, 0); BitConverter.GetBytes(j.Length).CopyTo(b, 4); j.CopyTo(b, 8);
            lock (wl) { var p = pipe; if (p == null) throw new IOException("no pipe"); p.Write(b, 0, b.Length); p.Flush(); }
        }
        public void SetActivity(object activity)
        {
            Send(1, new Dictionary<string, object> { { "cmd", "SET_ACTIVITY" }, { "args", new Dictionary<string, object> { { "pid", Process.GetCurrentProcess().Id }, { "activity", activity } } }, { "nonce", Guid.NewGuid().ToString("N") } });
        }
        void ReadLoop(NamedPipeClientStream p)
        {
            try
            {
                var head = new byte[8];
                while (true)
                {
                    if (!Fill(p, head, 8)) break;
                    int op = BitConverter.ToInt32(head, 0), len = BitConverter.ToInt32(head, 4);
                    if (len < 0 || len > (1 << 24)) break;
                    var body = new byte[len];
                    if (!Fill(p, body, len)) break;
                    object msg = null; try { msg = Json.Parse(Encoding.UTF8.GetString(body)); } catch { }
                    if (op == 3) Send(4, msg ?? new Dictionary<string, object>());
                    else if (op == 2) { Log.Write("discord closed: " + Json.Str(Json.Get(msg, "message"))); break; }
                    else
                    {
                        var evt = Json.Str(Json.Get(msg, "evt")); var cmd = Json.Str(Json.Get(msg, "cmd"));
                        if (evt == "READY")
                        {
                            User = Json.Str(Json.Get(msg, "data", "user", "username")); Ready = true;
                            Log.Write("connected to Discord as " + User);
                            var c = Changed; if (c != null) c();
                        }
                        else if (evt == "ERROR") Log.Write("discord error: " + Json.Str(Json.Get(msg, "data", "message")));
                        else if (cmd == "SET_ACTIVITY" && Json.Get(msg, "data") == null) { Log.Write("discord confirmed presence cleared"); ClearAck.Set(); }
                    }
                }
            }
            catch { }
            Drop(p);
        }
        static bool Fill(Stream s, byte[] b, int n)
        {
            int got = 0, r;
            while (got < n) { r = s.Read(b, got, n - got); if (r <= 0) return false; got += r; }
            return true;
        }
        void Drop(NamedPipeClientStream p)
        {
            bool was = false;
            lock (wl) { if (ReferenceEquals(pipe, p)) { pipe = null; was = true; Ready = false; User = null; } }
            try { p.Dispose(); } catch { }
            if (was) { Log.Write("pipe closed"); var c = Changed; if (c != null) c(); }
        }
        public void Close() { var p = pipe; if (p != null) Drop(p); }
    }

    public class PageText { public string Name, Details, State, Hover; }

    public class Snapshot
    {
        public string Conn, User, PageName, NoneReason, Project, Details, State, Hover, Image, Overlay, TypeWord, Error, Coverage;
        public bool Paused, Live;
        public DateTime Start;
        public List<string> Pages = new List<string>();
    }

    public class Engine
    {
        public Config Cfg; public readonly Discord Rpc = new Discord();
        public readonly PlanUsage Plan = new PlanUsage(); public readonly UsageScan Scan = new UsageScan();
        public event Action<Snapshot> Updated;
        public bool Paused;
        Timer timer; int page = -1; DateTime sessionStart = DateTime.MinValue; string lastKey = ""; bool sentAny; int busy; DateTime nextConnect = DateTime.MinValue, lastSend = DateTime.MinValue;
        public int TestDropAfter; DateTime started = DateTime.UtcNow; bool dropped;

        readonly List<RemoteLink> links = new List<RemoteLink>(); volatile bool stopping;

        public Engine(Config c)
        {
            Cfg = c;
            Rpc.Changed += () => { lastKey = ""; sentAny = false; ThreadPool.QueueUserWorkItem(_ => Tick(false)); };
            foreach (var r in c.remotes)
            {
                links.Add(new RemoteLink(r));
                if (!string.IsNullOrEmpty(r.userData) && !string.IsNullOrEmpty(r.transcripts))
                {
                    Scan.Sources.Add(new UsageSource(r.userData, r.transcripts));
                    Plan.UserDataDirs.Add(r.userData);
                }
            }
        }
        public bool HasRemotes { get { return links.Count > 0; } }
        // Follows the Settings toggle: applies to live state, token and value totals. Plan limits are per account anyway.
        public void SyncRemotes()
        {
            Scan.IncludeRemotes = Cfg.includeRemotes;
            foreach (var l in links) { if (Cfg.includeRemotes) l.Start(); else l.Stop(); }
        }
        bool RemotesCounted { get { return Cfg.includeRemotes && links.Count > 0; } }
        bool AllOnline { get { return !RemotesCounted || (links.All(l => l.Online) && Scan.AllLoaded); } }
        string Coverage()
        {
            if (!RemotesCounted) return "Covers this PC";
            var away = links.Where(l => !l.Online).Select(l => l.Cfg.name).ToList();
            if (away.Count > 0) return string.Join(", ", away) + " offline, its agents aren't counted right now";
            if (!Scan.AllLoaded) return "Loading usage from " + string.Join(", ", links.Select(l => l.Cfg.name));
            return "Covers all environments: this PC, " + string.Join(", ", links.Select(l => l.Cfg.name));
        }
        T3State ReadAll()
        {
            var all = new List<T3State> { T3.Read() };
            if (RemotesCounted) foreach (var l in links) { var st = l.State; if (st != null) all.Add(st); }
            return T3.Merge(all);
        }

        // --- number formats ---
        public static string Tok(long n)
        {
            var c = CultureInfo.InvariantCulture;
            if (n >= 1000000000) return (n / 1e9).ToString("0.#", c) + "B";
            if (n >= 1000000) return (n / 1e6).ToString("0.#", c) + "M";
            if (n >= 1000) return (n / 1e3).ToString(n >= 100000 ? "0" : "0.#", c) + "k";
            return n.ToString(c);
        }
        public static string Usd(double v) { return "$" + v.ToString("#,##0.00", CultureInfo.InvariantCulture); }
        public static string Dur(int? m)
        {
            if (!m.HasValue) return "?";
            int v = m.Value;
            if (v >= 1440) return (v / 1440) + "d" + (v % 1440 / 60) + "h";
            if (v >= 60) return (v / 60) + "h" + (v % 60).ToString("00") + "m";
            return v + "m";
        }
        public static string TypeWord(int t) { return t == 0 ? "Playing" : t == 2 ? "Listening to" : t == 5 ? "Competing in" : "Watching"; }

        string ProjectName(T3State s)
        {
            if (!Cfg.showProject || string.IsNullOrEmpty(s.Project) || s.Project == "No project") return null;
            // The card already says T3 Code, so a project with that name adds nothing.
            return s.Project.Equals("T3 Code", StringComparison.OrdinalIgnoreCase) || s.Project.Equals("t3code", StringComparison.OrdinalIgnoreCase) ? null : s.Project;
        }
        static string Agents(T3State s) { return s.Running + " agent" + (s.Running == 1 ? "" : "s") + " running" + (s.Subagents > 0 ? " + " + s.Subagents + " sub" : ""); }
        public static string GroupModels(List<string> list, int max)
        {
            var parts = list.GroupBy(x => x).OrderByDescending(g => g.Count()).ThenBy(g => g.Key).Select(g => (g.Count() > 1 ? g.Count() + "x " : "") + g.Key).ToList();
            var sb = new StringBuilder(); int i = 0;
            for (; i < parts.Count; i++)
            {
                int reserve = i + 1 < parts.Count ? 12 : 0;
                if (sb.Length + (sb.Length > 0 ? 2 : 0) + parts[i].Length + reserve > max) break;
                if (sb.Length > 0) sb.Append(", ");
                sb.Append(parts[i]);
            }
            if (i < parts.Count) sb.Append((sb.Length > 0 ? ", " : "") + "+" + (parts.Count - i) + " more");
            return sb.ToString();
        }

        // Each page says a thing once: no complements, no totals repeating the parts, no app name (the card header has it).
        public PageText PageFor(string name, T3State s)
        {
            var proj = ProjectName(s);
            if (name == "activity")
            {
                var word = s.Running > 0 ? "Working" : "Reviewing";
                string state;
                if (s.Running > 0)
                {
                    if (Cfg.liveLine) state = s.Live ?? "Thinking";
                    else state = Cfg.pages.Contains("agents") ? null : Agents(s);
                }
                else state = s.Sessions + " session" + (s.Sessions == 1 ? "" : "s") + " open";
                return new PageText { Name = name, Details = proj != null ? word + " in " + proj : word, State = state, Hover = s.Model };
            }
            if (name == "agents")
            {
                if (s.Running <= 0) return null;
                return new PageText { Name = name, Details = Agents(s), State = GroupModels(s.RunningModels, 128), Hover = s.CtxMax > 0 ? "Context " + Tok(s.CtxUsed) + " of " + Tok(s.CtxMax) : null };
            }
            if (name == "tokens")
            {
                var t = Scan.Range(1); var w = Scan.Range(7);
                if (t.Total == 0 && w.Total == 0) return null;
                return new PageText { Name = name, Details = "Today: " + Tok(t.Input) + " in, " + Tok(t.Output) + " out", State = "Cache: " + Tok(t.CacheRead) + " read, " + Tok(t.CacheWrite) + " write", Hover = "Last 7 days: " + Tok(w.Total) + " tokens" };
            }
            if (name == "value")
            {
                var t = Scan.Range(1); var r = Scan.Range(Cfg.valueDays);
                if (r.Usd <= 0) return null;
                return new PageText { Name = name, Details = "Token value today: " + Usd(t.Usd), State = "Last " + Cfg.valueDays + " days: " + Usd(r.Usd), Hover = "API list prices, not a bill. " + (!RemotesCounted ? "This PC." : Scan.AllLoaded ? "All environments." : "This PC only right now.") };
            }
            if (name == "usage")
            {
                var f = Plan.FiveHour; var w = Plan.Week;
                if (f == null && w == null) return null;
                return new PageText { Name = name, Details = f != null ? "5h limit: " + f.Used + "% used, resets in " + Dur(f.InMin) : null, State = w != null ? "Week limit: " + w.Used + "% used, resets in " + Dur(w.InMin) : null, Hover = s.Model };
            }
            return null;
        }
        string ImageFor(string project, string state)
        {
            var m = Cfg.projectImages; string v;
            if (project != null && (m.TryGetValue(project + "|" + state, out v) || m.TryGetValue(project, out v)) && !string.IsNullOrWhiteSpace(v)) return v;
            if (m.TryGetValue("*|" + state, out v) && !string.IsNullOrWhiteSpace(v)) return v;
            return Cfg.largeImageKey;
        }
        string StateKey(T3State s)
        {
            bool near = (Plan.FiveHour != null && Plan.FiveHour.Used >= Cfg.limitPercent) || (Plan.Week != null && Plan.Week.Used >= Cfg.limitPercent);
            return near ? "limit" : s.Running > 0 ? "working" : "idle";
        }
        static string Cut(string s) { return s == null ? null : s.Length > 128 ? s.Substring(0, 128) : s; }
        Dictionary<string, object> Activity(PageText p, T3State s, DateTime start)
        {
            var a = new Dictionary<string, object> { { "type", Cfg.activityType } };
            if (!string.IsNullOrEmpty(p.Details)) a["details"] = Cut(p.Details);
            if (!string.IsNullOrEmpty(p.State)) a["state"] = Cut(p.State);
            a["timestamps"] = new Dictionary<string, object> { { "start", (long)(start.ToUniversalTime() - new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc)).TotalSeconds } };
            var state = StateKey(s); var assets = new Dictionary<string, object>();
            var img = ImageFor(ProjectName(s), state);
            if (!string.IsNullOrEmpty(img)) assets["large_image"] = img;
            if (!string.IsNullOrEmpty(p.Hover) && p.Hover.Length >= 2) assets["large_text"] = Cut(p.Hover);
            string small;
            if (Cfg.stateImages.TryGetValue(state, out small) && !string.IsNullOrWhiteSpace(small))
            {
                assets["small_image"] = small;
                if (state == "limit") assets["small_text"] = "Near plan limit";
            }
            if (assets.Count > 0) a["assets"] = assets;
            return a;
        }
        public string Dump()
        {
            Plan.Refresh(); Scan.IncludeRemotes = Cfg.includeRemotes; Scan.RefreshNow();
            if (RemotesCounted)
            {   // wait for the first line from each linked machine, 20 s at most
                SyncRemotes();
                for (int i = 0; i < 200 && !links.All(l => l.Online); i++) Thread.Sleep(100);
            }
            var s = ReadAll(); var sb = new StringBuilder();
            var t = Scan.Range(1); var w = Scan.Range(7); var v = Scan.Range(Cfg.valueDays);
            sb.AppendLine(Json.Write(new Dictionary<string, object> { {"error",s.Error},{"running",s.Running},{"subagents",s.Subagents},{"sessions",s.Sessions},{"threads",s.Threads},{"project",s.Project},{"model",s.Model},{"live",s.Live},{"runningModels",s.RunningModels},{"ctx",s.CtxUsed+"/"+s.CtxMax},
                {"today",t},{"week",w},{"valueWindow",v},{"scanError",Scan.Error},{"plan5h",Plan.FiveHour},{"planWeek",Plan.Week},{"coverage",Coverage()} }));
            foreach (var n in Config.AllPages)
            {
                var p = PageFor(n, s);
                sb.AppendLine(n + " " + (p == null ? "null" : Json.Write(Activity(p, s, DateTime.Now))));
            }
            foreach (var l in links) l.Stop();
            return sb.ToString();
        }

        public void Start()
        {
            SyncRemotes();
            timer =new Timer(_ => Tick(true), null, 200, Math.Max(15, Cfg.pageSeconds) * 1000);
        }
        public void Reschedule() { if (timer != null) timer.Change(Math.Max(15, Cfg.pageSeconds) * 1000, Math.Max(15, Cfg.pageSeconds) * 1000); Poke(); }
        public void Poke() { ThreadPool.QueueUserWorkItem(_ => Tick(false)); }
        public void Stop()
        {
            stopping = true;
            if (timer != null) timer.Dispose();
            foreach (var l in links) l.Stop();
            try
            {
                if (Rpc.Ready) { Rpc.ClearAck.Reset(); Rpc.SetActivity(null); if (!Rpc.ClearAck.WaitOne(1500)) Log.Write("no clear ack from discord"); }
            }
            catch { }
            Rpc.Close();
        }
        void Tick(bool advance)
        {
            if (stopping || Interlocked.Exchange(ref busy, 1) == 1) return;
            try
            {
                if (TestDropAfter > 0 && !dropped && (DateTime.UtcNow - started).TotalSeconds >= TestDropAfter) { dropped = true; Log.Write("TEST: dropping own pipe"); Rpc.Close(); nextConnect = DateTime.MinValue; }
                if (!Rpc.Open && DateTime.UtcNow >= nextConnect)
                {
                    nextConnect = DateTime.UtcNow.AddSeconds(Math.Max(15, Cfg.pageSeconds));
                    if (!string.IsNullOrEmpty(Cfg.clientId)) Rpc.TryConnect(Cfg.clientId);
                }
                ThreadPool.QueueUserWorkItem(_ => { try { Plan.Refresh(); } catch { } });
                Scan.RefreshInBackground();
                var s = ReadAll();
                var snap = new Snapshot { Coverage = Coverage(), Paused = Paused, Conn = Rpc.Ready ? "connected" : Rpc.Open ? "connecting" : "down", User = Rpc.User, TypeWord = TypeWord(Cfg.activityType), Error = s.Error, Project = ProjectName(s), Live = Cfg.liveLine };
                Dictionary<string, object> act = null; PageText shown = null;
                if (s.Error != null) snap.NoneReason = "Can't read T3's data right now";
                else if (s.Running <= 0 && (DateTime.UtcNow - s.LastAt).TotalMinutes > Cfg.idleMinutes) { sessionStart = DateTime.MinValue; snap.NoneReason = "No agents for " + Cfg.idleMinutes + " min, presence cleared"; }
                else
                {
                    if (sessionStart == DateTime.MinValue) sessionStart = DateTime.Now;
                    foreach (var n in Cfg.pages) if (PageFor(n, s) != null) snap.Pages.Add(n);
                    if (snap.Pages.Count == 0) snap.NoneReason = "No pages to show";
                    else
                    {
                        if (advance) page++;
                        var name = snap.Pages[Math.Max(page, 0) % snap.Pages.Count];
                        shown = PageFor(name, s);
                        act = Activity(shown, s, sessionStart);
                    }
                }
                if (shown != null)
                {
                    snap.PageName = shown.Name; snap.Details = shown.Details; snap.State = shown.State; snap.Hover = shown.Hover; snap.Start = sessionStart;
                    snap.Image = ImageFor(snap.Project, StateKey(s)); snap.Overlay = StateKey(s);
                }
                var wanted = Paused ? null : act;
                var withoutTime = wanted == null ? null : wanted.Where(kv => kv.Key != "timestamps").ToDictionary(kv => kv.Key, kv => kv.Value);
                var key = Json.Write(withoutTime);
                // Discord allows 5 updates per 20 s: settings pokes wait for the next tick unless they clear or follow a connect.
                if (Rpc.Ready && (key != lastKey || !sentAny) && (!sentAny || wanted == null || (DateTime.UtcNow - lastSend).TotalSeconds >= 14))
                {
                    try
                    {
                        Rpc.SetActivity(wanted);
                        if (!sentAny || wanted == null) Log.Write(wanted == null ? "presence cleared" : "presence on: " + shown.Name);
                        lastKey = key; sentAny = true; lastSend = DateTime.UtcNow;
                    }
                    catch (Exception e) { Log.Write("send failed: " + e.Message); Rpc.Close(); }
                }
                snap.Conn = Rpc.Ready ? "connected" : Rpc.Open ? "connecting" : "down"; snap.User = Rpc.User;
                var u = Updated; if (u != null) u(snap);
            }
            catch (Exception e) { Log.Write("tick error: " + e.Message); }
            finally { Interlocked.Exchange(ref busy, 0); }
        }
    }
}
