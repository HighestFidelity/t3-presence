// Local data only: T3's state DB (read-only), T3's usage scan cache + rate table, Claude transcripts' usage fields, T3 provider logs.
// No model calls, no network. Thread titles, messages, paths and commands are never selected or kept.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Web.Script.Serialization;

namespace T3Presence
{
    public static class Json
    {
        static readonly JavaScriptSerializer S = new JavaScriptSerializer { MaxJsonLength = int.MaxValue, RecursionLimit = 200 };
        public static object Parse(string s) { return S.DeserializeObject(s); }
        public static string Write(object o) { return S.Serialize(o); }
        public static object Get(object o, params string[] path)
        {
            foreach (var k in path)
            {
                var d = o as IDictionary<string, object>;
                if (d == null || !d.TryGetValue(k, out o)) return null;
            }
            return o;
        }
        public static string Str(object o) { return o as string; }
        public static double? Num(object o)
        {
            if (o == null || o is string || o is bool || o is IDictionary || o is IEnumerable) return null;
            try { return Convert.ToDouble(o); } catch { return null; }
        }
        public static long Count(object o)
        {
            var n = Num(o);
            return n.HasValue && n.Value > 0 ? (long)n.Value : 0;
        }
    }

    public sealed class Db : IDisposable
    {
        const string L = "winsqlite3.dll";
        [DllImport(L)] static extern int sqlite3_open_v2(byte[] f, out IntPtr db, int flags, IntPtr vfs);
        [DllImport(L)] static extern int sqlite3_prepare_v2(IntPtr db, byte[] sql, int n, out IntPtr st, IntPtr tail);
        [DllImport(L)] static extern int sqlite3_step(IntPtr st);
        [DllImport(L)] static extern IntPtr sqlite3_column_text(IntPtr st, int c);
        [DllImport(L)] static extern int sqlite3_column_bytes(IntPtr st, int c);
        [DllImport(L)] static extern int sqlite3_finalize(IntPtr st);
        [DllImport(L)] static extern int sqlite3_close_v2(IntPtr db);
        [DllImport(L)] static extern int sqlite3_busy_timeout(IntPtr db, int ms);
        const int READONLY = 1, ROW = 100;
        IntPtr db;

        public Db(string path)
        {
            if (sqlite3_open_v2(Encoding.UTF8.GetBytes(path + "\0"), out db, READONLY, IntPtr.Zero) != 0) throw new IOException("cannot open T3 DB");
            sqlite3_busy_timeout(db, 1500);
        }
        public List<string[]> Query(string sql, int cols)
        {
            IntPtr st; var rows = new List<string[]>();
            if (sqlite3_prepare_v2(db, Encoding.UTF8.GetBytes(sql + "\0"), -1, out st, IntPtr.Zero) != 0) throw new IOException("bad query");
            try
            {
                while (sqlite3_step(st) == ROW)
                {
                    var r = new string[cols];
                    for (int c = 0; c < cols; c++)
                    {
                        var p = sqlite3_column_text(st, c);
                        if (p == IntPtr.Zero) continue;
                        var b = new byte[sqlite3_column_bytes(st, c)];
                        Marshal.Copy(p, b, 0, b.Length);
                        r[c] = Encoding.UTF8.GetString(b);
                    }
                    rows.Add(r);
                }
            }
            finally { sqlite3_finalize(st); }
            return rows;
        }
        public long One(string sql) { var r = Query(sql, 1); long v; return r.Count > 0 && long.TryParse(r[0][0], out v) ? v : 0; }
        public void Dispose() { if (db != IntPtr.Zero) { sqlite3_close_v2(db); db = IntPtr.Zero; } }
    }

    public class T3State
    {
        public string Error, Project, Model = "?", LiveType, LiveTool, LiveStatus, LiveAt;
        public bool Waiting;
        public long Running, Subagents, Sessions, Threads, CtxUsed, CtxMax;
        public DateTime LastAt = DateTime.MinValue;
        public List<string> RunningModels = new List<string>();
        public string Live
        {
            get
            {
                if (Running <= 0) return null;
                if (Waiting) return "Waiting on you";
                return LiveStatus == "running" ? T3.LiveWord(LiveType, LiveTool) : "Thinking";
            }
        }
    }

    public static class T3
    {
        public static readonly string Home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        public static readonly string UserData = Path.Combine(Home, ".t3", "userdata");
        const string P = "orchestration_v2_projection_";

        public static string ModelName(string id)
        {
            if (string.IsNullOrEmpty(id)) return "?";
            var m = Regex.Match(id, @"claude-(\w+?)-(\d+)-(\d+)");
            return m.Success ? char.ToUpper(m.Groups[1].Value[0]) + m.Groups[1].Value.Substring(1) + " " + m.Groups[2].Value + "." + m.Groups[3].Value : id;
        }
        static string EffortOf(object sel)
        {
            var opts = Json.Get(sel, "options") as IEnumerable;
            if (opts == null || opts is string) return null;
            foreach (var o in opts) if (Json.Str(Json.Get(o, "id")) == "effort") return Json.Str(Json.Get(o, "value"));
            return null;
        }
        static string Label(string selectionJson)
        {
            if (string.IsNullOrEmpty(selectionJson)) return "?";
            object sel; try { sel = Json.Parse(selectionJson); } catch { return "?"; }
            var e = EffortOf(sel);
            return ModelName(Json.Str(Json.Get(sel, "model"))) + (string.IsNullOrEmpty(e) ? "" : " " + e);
        }

        // Fixed wording per item KIND. Raw tool names, titles, paths and commands never get through here.
        static readonly Dictionary<string, string> Kinds = new Dictionary<string, string> {
            {"command_execution","Running a command"},{"file_change","Editing a file"},{"web_search","Searching the web"},
            {"reasoning","Thinking"},{"assistant_message","Writing a reply"},{"subagent","Running a sub-agent"},
            {"user_input_request","Waiting on you"},{"compaction","Compacting context"},{"error","Hit an error"},
            {"checkpoint","Saving a checkpoint"},{"user_message","Thinking"} };
        static readonly Dictionary<string, string> Tools = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase) {
            {"Read","Reading a file"},{"NotebookRead","Reading a file"},{"Grep","Searching the code"},{"Glob","Searching the code"},{"LS","Searching the code"},
            {"ToolSearch","Loading tools"},{"Edit","Editing a file"},{"MultiEdit","Editing a file"},{"Write","Editing a file"},{"NotebookEdit","Editing a file"},
            {"Bash","Running a command"},{"PowerShell","Running a command"},{"WebFetch","Reading the web"},{"WebSearch","Searching the web"},
            {"Agent","Running a sub-agent"},{"Task","Running a sub-agent"},{"Skill","Using a skill"},{"AskUserQuestion","Waiting on you"} };
        public static string LiveWord(string type, string tool)
        {
            string w;
            if (type == "dynamic_tool") return tool != null && Tools.TryGetValue(tool, out w) ? w : "Using a tool";
            return Kinds.TryGetValue(type ?? "", out w) ? w : "Working";
        }

        static DateTime ParseUtc(string v)
        {
            DateTime at;
            return DateTime.TryParse(v, null, System.Globalization.DateTimeStyles.AdjustToUniversal | System.Globalization.DateTimeStyles.AssumeUniversal, out at) ? at : DateTime.MinValue;
        }
        public static T3State FromJson(object o)
        {
            var s = new T3State { Error = Json.Str(Json.Get(o, "error")) };
            if (s.Error != null) return s;
            s.Running = Json.Count(Json.Get(o, "running")); s.Subagents = Json.Count(Json.Get(o, "subagents"));
            s.Sessions = Json.Count(Json.Get(o, "sessions")); s.Threads = Json.Count(Json.Get(o, "threads"));
            s.CtxUsed = Json.Count(Json.Get(o, "ctxUsed")); s.CtxMax = Json.Count(Json.Get(o, "ctxMax"));
            s.Project = Json.Str(Json.Get(o, "project")); s.Model = Label(Json.Str(Json.Get(o, "lastSelection")));
            s.LastAt = ParseUtc(Json.Str(Json.Get(o, "lastAt")));
            var sel = Json.Get(o, "selections") as object[];
            if (sel != null) foreach (var x in sel) s.RunningModels.Add(Label(x as string));
            var w = Json.Get(o, "waiting"); s.Waiting = w is bool && (bool)w;
            s.LiveType = Json.Str(Json.Get(o, "liveType")); s.LiveTool = Json.Str(Json.Get(o, "liveTool"));
            s.LiveStatus = Json.Str(Json.Get(o, "liveStatus")); s.LiveAt = Json.Str(Json.Get(o, "liveAt"));
            return s;
        }
        // All environments as one: counts add up, project, model and context come from whichever ran last.
        public static T3State Merge(IEnumerable<T3State> all)
        {
            var ok = all.Where(x => x != null && x.Error == null).ToList();
            if (ok.Count == 0) return all.FirstOrDefault() ?? new T3State { Error = "no data" };
            var m = new T3State();
            var newest = ok.OrderByDescending(x => x.LastAt).First();
            m.Project = newest.Project; m.Model = newest.Model; m.LastAt = newest.LastAt; m.CtxUsed = newest.CtxUsed; m.CtxMax = newest.CtxMax;
            foreach (var x in ok)
            {
                m.Running += x.Running; m.Subagents += x.Subagents; m.Sessions += x.Sessions; m.Threads += x.Threads;
                m.RunningModels.AddRange(x.RunningModels); m.Waiting |= x.Waiting && x.Running > 0;
            }
            var live = ok.Where(x => x.Running > 0).OrderByDescending(x => x.LiveAt ?? "", StringComparer.Ordinal).FirstOrDefault();
            if (live != null) { m.LiveType = live.LiveType; m.LiveTool = live.LiveTool; m.LiveStatus = live.LiveStatus; m.LiveAt = live.LiveAt; }
            return m;
        }

        public static T3State Read()
        {
            var s = new T3State();
            try
            {
                using (var db = new Db(Path.Combine(UserData, "statev2.sqlite")))
                {
                    s.Running = db.One("select count(*) from " + P + "runs where status='running'");
                    s.Subagents = db.One("select count(*) from " + P + "subagents where status='running'");
                    s.Sessions = db.One("select count(*) from " + P + "provider_sessions where status='ready'");
                    s.Threads = db.One("select count(*) from " + P + "threads where archived_at is null and deleted_at is null");
                    var last = db.Query("select json_extract(r.payload_json,'$.modelSelection'), coalesce(r.completed_at,r.requested_at), coalesce(p.title,'No project') from " + P + "runs r join " + P + "threads t on t.thread_id=r.thread_id left join projection_projects p on p.project_id=t.project_id order by r.requested_at desc limit 1", 3);
                    if (last.Count > 0)
                    {
                        s.Model = Label(last[0][0]); s.Project = last[0][2];
                        s.LastAt = ParseUtc(last[0][1]);
                    }
                    foreach (var r in db.Query("select json_extract(payload_json,'$.modelSelection') from " + P + "runs where status='running'", 1)) s.RunningModels.Add(Label(r[0]));
                    var ctx = db.Query("select json_extract(payload_json,'$.tokenUsage.usedTokens'), json_extract(payload_json,'$.tokenUsage.maxTokens') from " + P + "provider_turns where provider_turn_id in (select provider_turn_id from " + P + "provider_turns order by started_at desc limit 40) and json_extract(payload_json,'$.tokenUsage.usedTokens') is not null order by started_at desc limit 1", 2);
                    if (ctx.Count > 0) { long.TryParse(ctx[0][0], out s.CtxUsed); long.TryParse(ctx[0][1], out s.CtxMax); }
                    if (s.Running > 0)
                    {
                        const string live = "from " + P + "turn_items i where i.run_id in (select run_id from " + P + "runs where status='running')";
                        var waiting = db.One("select count(*) " + live + " and i.status='running' and (i.type='user_input_request' or (i.type='dynamic_tool' and json_extract(i.payload_json,'$.toolName')='AskUserQuestion'))") > 0
                            || db.One("select count(*) from " + P + "runtime_requests where status not in ('resolved','cancelled','expired','failed') and thread_id in (select thread_id from " + P + "runs where status='running')") > 0;
                        s.Waiting = waiting;
                        // Only the type column and, for tool items, the tool's name are read. Mapped to fixed words above.
                        var it = db.Query("select i.type, case when i.type='dynamic_tool' then json_extract(i.payload_json,'$.toolName') end, i.status, i.updated_at " + live + " and i.type not in ('notification','system_notice','run_interrupt_request','run_interrupt_result','user_input_request') order by i.updated_at desc limit 1", 4);
                        if (it.Count > 0) { s.LiveType = it[0][0]; s.LiveTool = it[0][1]; s.LiveStatus = it[0][2]; s.LiveAt = it[0][3]; }
                    }
                }
            }
            catch (Exception e) { s.Error = e.Message; }
            return s;
        }
    }

    // Plan usage: newest rate_limit_event in the T3 provider logs.
    public class PlanWindow { public int Used; public int? InMin; }
    public class PlanUsage
    {
        public PlanWindow FiveHour, Week;
        public readonly List<string> UserDataDirs = new List<string> { T3.UserData };
        DateTime at = DateTime.MinValue;
        public void Refresh()
        {
            if ((DateTime.UtcNow - at).TotalSeconds < 120) return;
            at = DateTime.UtcNow;
            try
            {
                var all = new List<FileInfo>();
                foreach (var ud in UserDataDirs.ToArray())
                {
                    try { all.AddRange(new DirectoryInfo(Path.Combine(ud, "logs", "provider")).GetFiles("events.*.log")); }
                    catch (IOException) { }
                }
                DateTime bestTs = DateTime.MinValue; object best = null;
                foreach (var f in all.OrderByDescending(x => x.LastWriteTimeUtc).Take(4))
                {
                    string txt;
                    using (var fs = new FileStream(f.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
                    {
                        long len = Math.Min(fs.Length, 1 << 20);
                        fs.Seek(-len, SeekOrigin.End);
                        var b = new byte[len]; int got = 0, n;
                        while (got < len && (n = fs.Read(b, got, (int)len - got)) > 0) got += n;
                        txt = Encoding.UTF8.GetString(b, 0, got);
                    }
                    var lines = txt.Split('\n');
                    for (int i = lines.Length - 1; i >= 0; i--)
                    {
                        var L = lines[i];
                        if (L.IndexOf("\"rate_limit_event\"", StringComparison.Ordinal) < 0 || L.IndexOf("\"incoming\"", StringComparison.Ordinal) < 0) continue;
                        var m = Regex.Match(L, @"^\[([^\]]+)\]");
                        DateTime ts; if (!m.Success || !DateTime.TryParse(m.Groups[1].Value, out ts)) continue;
                        ts = ts.ToUniversalTime();
                        int k = L.IndexOf("NTIVE: ", StringComparison.Ordinal);
                        object o; try { o = Json.Parse(L.Substring(k >= 0 ? k + 7 : L.IndexOf('{'))); } catch { continue; }
                        var info = Json.Get(o, "event", "payload", "rate_limit_info");
                        if (info != null && ts > bestTs) { bestTs = ts; best = info; }
                        break;
                    }
                }
                if (best == null) return;
                FiveHour = Window(best, "five_hour"); Week = Window(best, "seven_day");
            }
            catch { /* keep last good value */ }
        }
        static PlanWindow Window(object info, string key)
        {
            var x = Json.Get(info, "unifiedWindows", key);
            if (x == null) return null;
            var resets = Json.Num(Json.Get(x, "resetsAt")) ?? 0;
            var inMs = resets * 1000 - (DateTime.UtcNow - new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc)).TotalMilliseconds;
            bool stale = resets > 0 && inMs <= 0;
            return new PlanWindow { Used = stale ? 0 : (int)Math.Round((Json.Num(Json.Get(x, "utilization")) ?? 0) * 100), InMin = resets > 0 ? (int?)Math.Max(0, (int)Math.Round(inMs / 60000)) : null };
        }
    }

    public class DayTotals { public long Input, Output, CacheRead, CacheWrite; public double Usd; public long Total { get { return Input + Output + CacheRead + CacheWrite; } } }

    // Same arithmetic as T3's Usage page: its scan-cache rows plus whatever the transcripts have grown since, deduped on
    // "message id:request id" (first copy wins), priced from T3's cached LiteLLM rates unless a row carries a reported cost.
    public class UsageScan
    {
        public readonly List<UsageSource> Sources = new List<UsageSource> { new UsageSource(T3.UserData, null) };
        public bool IncludeRemotes = true;
        IEnumerable<UsageSource> Active { get { return Sources.Where(s => !s.Remote || IncludeRemotes).ToList(); } }
        public DayTotals Range(int days)
        {
            var t = new DayTotals();
            foreach (var s in Active) s.AddRange(days, t);
            return t;
        }
        public bool AllLoaded { get { return Active.All(s => s.Loaded); } }
        public string Error { get { return string.Join("; ", Active.Where(s => s.Error != null).Select(s => s.Error)); } }
        public void RefreshNow() { foreach (var s in Active) s.Refresh(); }
        // Off the presence loop: a slow network share must not stall page rotation.
        public void RefreshInBackground() { foreach (var s in Active) s.RefreshInBackground(); }
    }

    public class UsageSource
    {
        readonly string userData, transcriptRoot; string remoteDir;
        public readonly bool Remote;
        public volatile bool Loaded;
        int running;
        public UsageSource(string userDataDir, string remoteTranscriptRoot) { userData = userDataDir; transcriptRoot = remoteTranscriptRoot; Remote = remoteTranscriptRoot != null; }
        public void RefreshInBackground()
        {
            if ((DateTime.UtcNow - stamp).TotalSeconds < 120 || Interlocked.Exchange(ref running, 1) == 1) return;
            new Thread(() => { try { Refresh(); } finally { Interlocked.Exchange(ref running, 0); } }) { IsBackground = true, Priority = ThreadPriority.BelowNormal, Name = "usage-scan" }.Start();
        }
        class Rate { public double In, Out, Read, Write, Fast = 1; }
        readonly Dictionary<string, Rate> rates = new Dictionary<string, Rate>();
        readonly Dictionary<string, long> offsets = new Dictionary<string, long>(StringComparer.OrdinalIgnoreCase);
        readonly HashSet<string> seen = new HashSet<string>();
        Dictionary<string, DayTotals> days = new Dictionary<string, DayTotals>();
        readonly List<string> transcriptDirs = new List<string>();
        DateTime stamp, cacheMtime, ratesMtime;
        readonly object gate = new object();
        public string Error;
        static readonly DateTime Epoch = new DateTime(1970, 1, 1, 0, 0, 0, DateTimeKind.Utc);

        static string DayKey(DateTime local) { return local.ToString("yyyy-MM-dd"); }
        public void AddRange(int daysBack, DayTotals t)
        {
            lock (gate)
            {
                var d = DateTime.Now.Date;
                for (int i = 0; i < daysBack; i++)
                {
                    DayTotals x; if (!days.TryGetValue(DayKey(d.AddDays(-i)), out x)) continue;
                    t.Input += x.Input; t.Output += x.Output; t.CacheRead += x.CacheRead; t.CacheWrite += x.CacheWrite; t.Usd += x.Usd;
                }
            }
        }
        public void Refresh()
        {
            if ((DateTime.UtcNow - stamp).TotalSeconds < 120) return;
            stamp = DateTime.UtcNow;
            try
            {
                var ratesFile = new FileInfo(Path.Combine(userData, "usage-model-rates.json"));
                var cacheFile = NewestCache();
                bool reload = cacheFile != null && (cacheFile.LastWriteTimeUtc != cacheMtime || (ratesFile.Exists && ratesFile.LastWriteTimeUtc != ratesMtime));
                if (reload)
                {
                    if (ratesFile.Exists && ratesFile.LastWriteTimeUtc != ratesMtime) LoadRates(ratesFile);
                    LoadCache(cacheFile);
                }
                TailTranscripts();
                Error = null; Loaded = cacheMtime != default(DateTime);
                if (reload)
                {   // the two big JSON parses are done, give that memory back
                    System.Runtime.GCSettings.LargeObjectHeapCompactionMode = System.Runtime.GCLargeObjectHeapCompactionMode.CompactOnce;
                    GC.Collect();
                }
            }
            catch (Exception e) { Error = e.Message; }
        }
        FileInfo NewestCache()
        {
            var di = new DirectoryInfo(userData); FileInfo best = null; int v = -1;
            foreach (var f in di.GetFiles("usage-scan-cache-v*.json"))
            {
                var m = Regex.Match(f.Name, @"-v(\d+)\.json$"); int n;
                if (m.Success && int.TryParse(m.Groups[1].Value, out n) && n > v) { v = n; best = f; }
            }
            return best;
        }
        void LoadRates(FileInfo f)
        {
            string raw;
            using (var r = new StreamReader(new FileStream(f.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))) raw = r.ReadToEnd();
            var doc = Json.Get(Json.Parse(raw), "document") as IDictionary<string, object>;
            if (doc == null) return;
            rates.Clear();
            foreach (var kv in doc)
            {
                var i = Json.Num(Json.Get(kv.Value, "input_cost_per_token")); var o = Json.Num(Json.Get(kv.Value, "output_cost_per_token"));
                if (!i.HasValue || !o.HasValue) continue;
                var rate = new Rate { In = i.Value, Out = o.Value, Read = Json.Num(Json.Get(kv.Value, "cache_read_input_token_cost")) ?? i.Value, Write = Json.Num(Json.Get(kv.Value, "cache_creation_input_token_cost")) ?? i.Value };
                var fast = Json.Num(Json.Get(kv.Value, "provider_specific_entry", "fast"));
                if (fast.HasValue && fast.Value > 0) rate.Fast = fast.Value;
                rates[kv.Key.Trim().ToLowerInvariant()] = rate;
            }
            ratesMtime = f.LastWriteTimeUtc;
        }
        double Price(string model, long uncached, long cached, long write, long output, double? reported, bool fast)
        {
            if (reported.HasValue) return reported.Value;
            var key = (model ?? "").Trim().ToLowerInvariant();
            int br = key.IndexOf('['); if (br >= 0) key = key.Substring(0, br);
            Rate r; if (!rates.TryGetValue(key, out r)) return 0;
            return (uncached * r.In + cached * r.Read + write * r.Write + output * r.Out) * (fast ? r.Fast : 1);
        }
        void Add(long tsMs, string model, long unc, long cached, long write, long output, string dedupe, double? reported, bool fast)
        {
            if (dedupe != null && !seen.Add(dedupe)) return;
            var key = DayKey(Epoch.AddMilliseconds(tsMs).ToLocalTime());
            DayTotals d; if (!days.TryGetValue(key, out d)) days[key] = d = new DayTotals();
            d.Input += unc; d.CacheRead += cached; d.CacheWrite += write; d.Output += output;
            d.Usd += Price(model, unc, cached, write, output, reported, fast);
        }
        void LoadCache(FileInfo f)
        {
            string raw;
            using (var r = new StreamReader(new FileStream(f.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))) raw = r.ReadToEnd();
            var doc = Json.Parse(raw);
            var models = (Json.Get(doc, "models") as object[] ?? new object[0]).Select(x => x as string).ToArray();
            var files = Json.Get(doc, "files") as IDictionary<string, object>;
            if (files == null) return;
            lock (gate)
            {
                days = new Dictionary<string, DayTotals>(); seen.Clear(); offsets.Clear(); transcriptDirs.Clear();
                var sources = Json.Get(doc, "sources") as IDictionary<string, object>;
                if (sources != null) foreach (var kv in sources) { var dir = Json.Str(Json.Get(kv.Value, "dir")); if (dir != null && kv.Key.StartsWith("claude", StringComparison.Ordinal)) { remoteDir = dir; transcriptDirs.Add(Remote ? transcriptRoot : dir); } }
                foreach (var kv in files)
                {
                    // A linked machine's cache holds its own paths: translate them to where its transcripts are reachable from here.
                    var local = Remote && remoteDir != null && kv.Key.StartsWith(remoteDir, StringComparison.Ordinal) ? transcriptRoot + kv.Key.Substring(remoteDir.Length).Replace('/', '\\') : kv.Key;
                    offsets[local] = Json.Count(Json.Get(kv.Value, "o"));
                    foreach (var part in new[] { "r", "t" })
                    {
                        var rows = Json.Get(kv.Value, part) as object[];
                        if (rows == null) continue;
                        foreach (var ro in rows)
                        {
                            var r = ro as object[];
                            if (r == null || r.Length < 11) continue;
                            int mi = (int)Json.Count(r[1]); var model = mi < models.Length ? models[mi] : null;
                            if (model == "<synthetic>") continue;
                            string rate = model;
                            if (r.Length > 11 && r[11] != null) { int ri = (int)Json.Count(r[11]); if (ri < models.Length) rate = models[ri]; }
                            Add((long)(Json.Num(r[0]) ?? 0), rate, Json.Count(r[3]), Json.Count(r[4]), Json.Count(r[5]), Json.Count(r[6]), r[8] as string, Json.Num(r[9]), Json.Count(r[10]) == 1);
                        }
                    }
                }
            }
            cacheMtime = f.LastWriteTimeUtc;
        }
        void TailTranscripts()
        {
            if (transcriptDirs.Count == 0) transcriptDirs.Add(Remote ? transcriptRoot : Path.Combine(T3.Home, ".claude", "projects"));
            var cutoff = DateTime.UtcNow.AddDays(-100);
            foreach (var dir in transcriptDirs)
            {
                if (!Directory.Exists(dir)) continue;
                foreach (var fi in new DirectoryInfo(dir).EnumerateFiles("*.jsonl", SearchOption.AllDirectories))
                {
                    if (fi.LastWriteTimeUtc < cutoff) continue;
                    long off; offsets.TryGetValue(fi.FullName, out off);
                    if (fi.Length == off) continue;
                    if (fi.Length < off) off = 0;
                    try { offsets[fi.FullName] = TailOne(fi.FullName, off); } catch (IOException) { }
                }
            }
        }
        long TailOne(string path, long off)
        {
            using (var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete, 1 << 16, FileOptions.SequentialScan))
            {
                fs.Seek(off, SeekOrigin.Begin);
                var line = new MemoryStream(); var buf = new byte[1 << 16]; long pos = off, lineStart = off; int n;
                while ((n = fs.Read(buf, 0, buf.Length)) > 0)
                {
                    int start = 0;
                    for (int i = 0; i < n; i++)
                    {
                        if (buf[i] != 10) continue;
                        line.Write(buf, start, i - start);
                        ParseLine(line);
                        line.SetLength(0); start = i + 1; lineStart = pos + i + 1;
                    }
                    line.Write(buf, start, n - start);
                    pos += n;
                }
                return lineStart;   // an unfinished last line gets re-read next time
            }
        }
        static readonly byte[] MarkA = Encoding.ASCII.GetBytes("\"usage\""), MarkB = Encoding.ASCII.GetBytes("\"assistant\"");
        static bool Has(byte[] h, int len, byte[] needle)
        {
            for (int i = 0; i <= len - needle.Length; i++)
            {
                int j = 0;
                while (j < needle.Length && h[i + j] == needle[j]) j++;
                if (j == needle.Length) return true;
            }
            return false;
        }
        void ParseLine(MemoryStream line)
        {
            int len = (int)line.Length; var b = line.GetBuffer();
            if (len == 0 || !Has(b, len, MarkA) || !Has(b, len, MarkB)) return;
            object o; try { o = Json.Parse(Encoding.UTF8.GetString(b, 0, len)); } catch { return; }
            if (Json.Str(Json.Get(o, "type")) != "assistant") return;
            var msg = Json.Get(o, "message"); var u = Json.Get(msg, "usage");
            var model = Json.Str(Json.Get(msg, "model"));
            DateTime ts;
            if (u == null || string.IsNullOrEmpty(model) || model == "<synthetic>") return;
            if (!DateTime.TryParse(Json.Str(Json.Get(o, "timestamp")), null, System.Globalization.DateTimeStyles.AdjustToUniversal, out ts)) return;
            long ms = (long)(ts - Epoch).TotalMilliseconds;
            var id = Json.Str(Json.Get(msg, "id")); var req = Json.Str(Json.Get(o, "requestId"));
            var key = id == null && req == null ? null : (id ?? "") + ":" + (req ?? "");
            bool fast = Json.Str(Json.Get(u, "speed")) == "fast";
            lock (gate)
            {
                Add(ms, model, Json.Count(Json.Get(u, "input_tokens")), Json.Count(Json.Get(u, "cache_read_input_tokens")), Json.Count(Json.Get(u, "cache_creation_input_tokens")), Json.Count(Json.Get(u, "output_tokens")), key, Json.Num(Json.Get(o, "costUSD")), fast);
                var its = Json.Get(u, "iterations") as object[];
                if (its == null) return;
                for (int i = 0; i < its.Length; i++)
                {
                    if (Json.Str(Json.Get(its[i], "type")) != "advisor_message") continue;
                    var am = Json.Str(Json.Get(its[i], "model"));
                    if (string.IsNullOrEmpty(am) || am == "<synthetic>") continue;
                    Add(ms, am, Json.Count(Json.Get(its[i], "input_tokens")), Json.Count(Json.Get(its[i], "cache_read_input_tokens")), Json.Count(Json.Get(its[i], "cache_creation_input_tokens")), Json.Count(Json.Get(its[i], "output_tokens")), key == null ? null : key + ":advisor:" + i, null, fast);
                }
            }
        }
    }
}
