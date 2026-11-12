// WPF window, tray icon, entry point.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Markup;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Microsoft.Win32;
using Forms = System.Windows.Forms;
using Drawing = System.Drawing;

namespace T3Presence
{
    static class Autostart
    {
        const string Key = @"Software\Microsoft\Windows\CurrentVersion\Run", Name = "T3Presence";
        static string Command { get { return "\"" + Assembly.GetExecutingAssembly().Location + "\" --minimized"; } }
        public static bool Enabled { get { using (var k = Registry.CurrentUser.OpenSubKey(Key)) return k != null && k.GetValue(Name) != null; } }
        public static void Set(bool on)
        {
            using (var k = Registry.CurrentUser.CreateSubKey(Key))
            {
                if (on) k.SetValue(Name, Command); else if (k.GetValue(Name) != null) k.DeleteValue(Name);
            }
        }
        public static void Refresh() { try { if (Enabled) Set(true); } catch { } }   // follows the exe if it moved
    }

    class Dark : Forms.ProfessionalColorTable
    {
        static readonly Drawing.Color Bg = Drawing.Color.FromArgb(30, 31, 34), Hi = Drawing.Color.FromArgb(88, 101, 242), Sep = Drawing.Color.FromArgb(58, 60, 66);
        public override Drawing.Color ToolStripDropDownBackground { get { return Bg; } }
        public override Drawing.Color ImageMarginGradientBegin { get { return Bg; } }
        public override Drawing.Color ImageMarginGradientMiddle { get { return Bg; } }
        public override Drawing.Color ImageMarginGradientEnd { get { return Bg; } }
        public override Drawing.Color MenuBorder { get { return Sep; } }
        public override Drawing.Color MenuItemBorder { get { return Hi; } }
        public override Drawing.Color MenuItemSelected { get { return Hi; } }
        public override Drawing.Color SeparatorDark { get { return Sep; } }
        public override Drawing.Color SeparatorLight { get { return Sep; } }
        public override Drawing.Color CheckBackground { get { return Bg; } }
        public override Drawing.Color CheckSelectedBackground { get { return Hi; } }
        public override Drawing.Color CheckPressedBackground { get { return Hi; } }
    }

    class Ui
    {
        [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr h, int attr, ref int val, int size);
        [StructLayout(LayoutKind.Sequential)] struct Margins { public int L, R, T, B; }
        [DllImport("dwmapi.dll")] static extern int DwmExtendFrameIntoClientArea(IntPtr h, ref Margins m);

        readonly Engine eng; readonly Window w; Forms.NotifyIcon tray; bool quitting;
        Forms.ToolStripMenuItem miPause, miAuto;
        readonly DispatcherTimer clock = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) };
        DateTime start = DateTime.MinValue; string shownPage, shownImage = "\0", coverProject = "\0"; bool loading;
        readonly BitmapImage logo; readonly Dictionary<string, CheckBox> pageToggles = new Dictionary<string, CheckBox>();

        T F<T>(string n) where T : class { return w.FindName(n) as T; }
        static Stream Res(string n) { return Assembly.GetExecutingAssembly().GetManifestResourceStream(n); }

        public Ui(Engine e, bool minimized)
        {
            eng = e;
            using (var s = Res("Window.xaml")) w = (Window)XamlReader.Load(s);
            using (var s = Res("logo.png"))
            {
                logo = new BitmapImage(); logo.BeginInit(); logo.CacheOption = BitmapCacheOption.OnLoad; logo.StreamSource = s; logo.EndInit(); logo.Freeze();
            }
            using (var s = Res("T3Presence.ico")) w.Icon = BitmapFrame.Create(s, BitmapCreateOptions.None, BitmapCacheOption.OnLoad);
            w.SourceInitialized += (a, b) => Backdrop();
            w.Closing += (a, b) => { if (!quitting) { b.Cancel = true; w.Hide(); } };
            BuildTray();
            WireSettings();
            F<Border>("Cover").Background = new ImageBrush(logo) { Stretch = Stretch.UniformToFill };
            clock.Tick += (a, b) => TickClock(); clock.Start();
            eng.Updated += s => w.Dispatcher.BeginInvoke(new Action(() => Apply(s)));
            if (!minimized) w.Show();
        }

        void Backdrop()
        {
            var h = new WindowInteropHelper(w).Handle; int on = 1;
            DwmSetWindowAttribute(h, 20, ref on, 4);        // dark title bar
            int caption = 0x001E1B1A, border = 0x00302D2B;   // COLORREF, BGR. Overrides "accent color on title bars".
            DwmSetWindowAttribute(h, 35, ref caption, 4);
            DwmSetWindowAttribute(h, 34, ref border, 4);
            var build = Environment.OSVersion.Version.Build;
            int mica = 2;
            if (build >= 22621 && DwmSetWindowAttribute(h, 38, ref mica, 4) == 0)
            {
                HwndSource.FromHwnd(h).CompositionTarget.BackgroundColor = Colors.Transparent;
                var m = new Margins { L = -1, R = -1, T = -1, B = -1 };
                DwmExtendFrameIntoClientArea(h, ref m);
            }
            else F<Grid>("Root").Background = new SolidColorBrush(Color.FromRgb(0x1E, 0x1F, 0x22));
        }

        void BuildTray()
        {
            Drawing.Icon ico;
            using (var s = Res("T3Presence.ico")) ico = new Drawing.Icon(s, Forms.SystemInformation.SmallIconSize);
            var menu = new Forms.ContextMenuStrip { Renderer = new Forms.ToolStripProfessionalRenderer(new Dark()), ShowImageMargin = false, ShowCheckMargin = true, ForeColor = Drawing.Color.FromArgb(242, 243, 245), BackColor = Drawing.Color.FromArgb(30, 31, 34) };
            menu.Items.Add("Show", null, (a, b) => ShowWindow());
            miPause = new Forms.ToolStripMenuItem("Pause presence") { CheckOnClick = true };
            miPause.Click += (a, b) => SetPaused(miPause.Checked);
            miAuto = new Forms.ToolStripMenuItem("Start with Windows") { CheckOnClick = true, Checked = Autostart.Enabled };
            miAuto.Click += (a, b) => SetAuto(miAuto.Checked);
            menu.Items.Add(miPause); menu.Items.Add(miAuto);
            menu.Items.Add(new Forms.ToolStripSeparator());
            menu.Items.Add("Restart", null, (a, b) => { Restart = true; Quit(); });
            menu.Items.Add("Exit", null, (a, b) => Quit());
            tray = new Forms.NotifyIcon { Icon = ico, Text = "T3 Presence", ContextMenuStrip = menu, Visible = true };
            tray.DoubleClick += (a, b) => ShowWindow();
        }
        public void ShowWindow() { w.Show(); if (w.WindowState == WindowState.Minimized) w.WindowState = WindowState.Normal; w.Activate(); }
        public void OpenSettingsTab() { F<System.Windows.Controls.Primitives.ToggleButton>("TabSet").IsChecked = true; }
        public static bool Restart;
        public void Quit()
        {
            quitting = true; tray.Visible = false; tray.Dispose();
            w.Hide();
            ThreadPool.QueueUserWorkItem(_ => { eng.Stop(); w.Dispatcher.BeginInvoke(new Action(() => Application.Current.Shutdown())); });
        }
        void SetPaused(bool p)
        {
            eng.Paused = p; miPause.Checked = p; F<Button>("PauseBtn").Content = p ? "Resume presence" : "Pause presence"; eng.Poke();
        }
        void SetAuto(bool on)
        {
            try { Autostart.Set(on); } catch (Exception e) { Log.Write("autostart: " + e.Message); }
            var en = Autostart.Enabled; miAuto.Checked = en;
            loading = true; F<CheckBox>("AutoT").IsChecked = en; loading = false;
        }

        void WireSettings()
        {
            var c = eng.Cfg; loading = true;
            F<System.Windows.Controls.Primitives.ToggleButton>("TabLive").Checked += (a, b) => Tab(true);
            F<System.Windows.Controls.Primitives.ToggleButton>("TabSet").Checked += (a, b) => Tab(false);
            F<Button>("PauseBtn").Click += (a, b) => SetPaused(!eng.Paused);
            F<Button>("OpenCfg").Click += (a, b) => { try { Process.Start(new ProcessStartInfo("notepad.exe", "\"" + Config.FilePath + "\"")); } catch { } };

            var box = F<ComboBox>("TypeBox"); int[] types = { 3, 0, 2, 5 };
            box.SelectedIndex = Math.Max(0, Array.IndexOf(types, c.activityType));
            box.SelectionChanged += (a, b) => { if (loading) return; c.activityType = types[box.SelectedIndex]; Saved(); };

            Bind("LiveT", c.liveLine, v => c.liveLine = v);
            Bind("ProjT", c.showProject, v => c.showProject = v);
            Bind("RemoteT", c.includeRemotes, v => { c.includeRemotes = v; eng.SyncRemotes(); });
            F<UIElement>("RemoteRow").Visibility = eng.HasRemotes ? Visibility.Visible : Visibility.Collapsed;
            var map = new Dictionary<string, string> { { "activity", "PgActivity" }, { "agents", "PgAgents" }, { "tokens", "PgTokens" }, { "value", "PgValue" }, { "usage", "PgUsage" } };
            foreach (var kv in map)
            {
                var name = kv.Key; pageToggles[name] = F<CheckBox>(kv.Value);
                Bind(kv.Value, c.pages.Contains(name), v =>
                {
                    if (v && !c.pages.Contains(name)) c.pages.Add(name);
                    if (!v) c.pages.Remove(name);
                    c.pages = Config.AllPages.Where(p => c.pages.Contains(p)).ToList();
                });
            }
            F<TextBlock>("SecVal").Text = c.pageSeconds.ToString(); F<TextBlock>("IdleVal").Text = c.idleMinutes.ToString();
            Stepper("SecDown", "SecUp", "SecVal", 5, 15, 300, () => c.pageSeconds, v => { c.pageSeconds = v; eng.Reschedule(); });
            Stepper("IdleDown", "IdleUp", "IdleVal", 5, 5, 240, () => c.idleMinutes, v => c.idleMinutes = v);
            var auto = F<CheckBox>("AutoT"); auto.IsChecked = Autostart.Enabled;
            auto.Click += (a, b) => { if (!loading) SetAuto(auto.IsChecked == true); };
            var cover = F<TextBox>("CoverBox");
            cover.LostKeyboardFocus += (a, b) => SaveCover();
            loading = false;
        }
        void Saved() { eng.Cfg.Save(); eng.Poke(); }
        void Bind(string name, bool value, Action<bool> set)
        {
            var t = F<CheckBox>(name); t.IsChecked = value;
            t.Click += (a, b) => { if (!loading) { set(t.IsChecked == true); Saved(); } };
        }
        void Stepper(string down, string up, string val, int step, int lo, int hi, Func<int> get, Action<int> set)
        {
            Action<int> go = d => { var v = Math.Max(lo, Math.Min(hi, get() + d)); set(v); F<TextBlock>(val).Text = v.ToString(); eng.Cfg.Save(); };
            F<Button>(down).Click += (a, b) => go(-step);
            F<Button>(up).Click += (a, b) => go(step);
        }
        void SaveCover()
        {
            var v = (F<TextBox>("CoverBox").Text ?? "").Trim();
            var c = eng.Cfg;
            if (coverProject == "\0" || string.IsNullOrEmpty(coverProject))
            {
                c.largeImageKey = v.Length == 0 ? Config.DefaultImage : v;
            }
            else if (v.Length == 0) c.projectImages.Remove(coverProject);
            else c.projectImages[coverProject] = v;
            Saved();
        }
        void Tab(bool live)
        {
            F<UIElement>("LivePanel").Visibility = live ? Visibility.Visible : Visibility.Collapsed;
            F<UIElement>("SetPanel").Visibility = live ? Visibility.Collapsed : Visibility.Visible;
        }
        static readonly Dictionary<string, string> Names = new Dictionary<string, string> { { "activity", "Activity" }, { "agents", "Agents" }, { "tokens", "Tokens" }, { "value", "Value" }, { "usage", "Plan" } };

        void Apply(Snapshot s)
        {
            // connection pill
            var dot = F<System.Windows.Shapes.Ellipse>("ConnDot"); var txt = F<TextBlock>("ConnText");
            if (s.Paused) { dot.Fill = B("#F0B232"); txt.Text = "Paused"; }
            else if (s.Conn == "connected") { dot.Fill = B("#23A55A"); txt.Text = string.IsNullOrEmpty(s.User) ? "Connected" : "Connected as " + s.User; }
            else if (s.Conn == "connecting") { dot.Fill = B("#F0B232"); txt.Text = "Connecting"; }
            else { dot.Fill = B("#80848E"); txt.Text = "Discord not running, retrying"; }

            F<TextBlock>("HeadWord").Text = s.TypeWord;
            var coverage = F<TextBlock>("Coverage"); coverage.Text = s.Coverage ?? "";
            var reason = F<TextBlock>("Reason");
            if (s.PageName == null)
            {
                F<TextBlock>("Details").Text = s.NoneReason ?? "Nothing to show"; F<TextBlock>("State").Text = "";
                F<TextBlock>("Hover").Text = ""; start = DateTime.MinValue;
                F<TextBlock>("Elapsed").Text = "";
                reason.Visibility = Visibility.Collapsed;
                shownPage = null;
            }
            else
            {
                reason.Visibility = Visibility.Collapsed;
                Action set = () =>
                {
                    Line("Details", s.Details); Line("State", s.State);
                    F<TextBlock>("Hover").Text = s.Hover ?? "";
                };
                start = s.Start; TickClock();
                if (shownPage != s.PageName) { Fade(set); shownPage = s.PageName; } else set();
            }
            // chips
            var chips = F<WrapPanel>("Chips"); chips.Children.Clear();
            foreach (var n in s.Pages.Count > 0 ? s.Pages : eng.Cfg.pages.ToList())
            {
                bool on = n == s.PageName;
                var chip = new Border { CornerRadius = new CornerRadius(12), Padding = new Thickness(11, 4, 11, 4), Margin = new Thickness(0, 0, 6, 6), Background = on ? B("#5865F2") : B("#26FFFFFF"), Cursor = System.Windows.Input.Cursors.Hand, ToolTip = "Show this page now", Child = new TextBlock { Text = Names[n], FontSize = 12, Padding = new Thickness(0, 0, 2, 0), Foreground = on ? Brushes.White : B("#B5BAC1") } };
                var name = n; chip.MouseLeftButtonUp += (a, b) => eng.JumpTo(name);
                chips.Children.Add(chip);
            }
            // overlay dot
            var ov = F<UIElement>("Overlay"); ov.Visibility = s.Overlay != null ? Visibility.Visible : Visibility.Collapsed;
            F<System.Windows.Shapes.Ellipse>("OverlayDot").Fill = B(s.Overlay == "limit" ? "#F23F43" : s.Overlay == "working" ? "#23A55A" : "#80848E");
            // cover
            if (s.Image != shownImage)
            {
                shownImage = s.Image;
                ImageSource src = logo;
                if (s.Image != null && s.Image != Config.DefaultImage && s.Image.StartsWith("https://", StringComparison.OrdinalIgnoreCase))
                {
                    try { var bi = new BitmapImage(); bi.BeginInit(); bi.UriSource = new Uri(s.Image); bi.DecodePixelWidth = 192; bi.EndInit(); bi.DownloadFailed += (a, b) => { F<Border>("Cover").Background = new ImageBrush(logo); }; src = bi; } catch { }
                }
                F<Border>("Cover").Background = new ImageBrush(src) { Stretch = Stretch.UniformToFill };
            }
            // cover settings target
            if (coverProject != (s.Project ?? ""))
            {
                coverProject = s.Project ?? "";
                var box = F<TextBox>("CoverBox");
                if (!box.IsKeyboardFocused)
                {
                    string v; F<TextBlock>("CoverLabel").Text = s.Project == null ? "Default cover" : "Cover for " + s.Project;
                    box.Text = s.Project != null && eng.Cfg.projectImages.TryGetValue(s.Project, out v) ? v : s.Project == null ? (eng.Cfg.largeImageKey == Config.DefaultImage ? "" : eng.Cfg.largeImageKey) : "";
                }
            }
        }
        void Line(string name, string text) { var t = F<TextBlock>(name); t.Text = text ?? ""; t.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible; }
        void Fade(Action swap)
        {
            var lines = F<StackPanel>("Lines");
            var outA = new DoubleAnimation(0, TimeSpan.FromMilliseconds(130));
            outA.Completed += (a, b) => { swap(); lines.BeginAnimation(UIElement.OpacityProperty, new DoubleAnimation(1, TimeSpan.FromMilliseconds(220))); };
            lines.BeginAnimation(UIElement.OpacityProperty, outA);
        }
        void TickClock()
        {
            var el = F<TextBlock>("Elapsed");
            if (start == DateTime.MinValue) { el.Text = ""; return; }
            var t = DateTime.Now - start;
            el.Text = (t.TotalHours >= 1 ? ((int)t.TotalHours) + ":" + t.Minutes.ToString("00") : t.Minutes.ToString("00")) + ":" + t.Seconds.ToString("00");
        }
        static readonly BrushConverter Conv = new BrushConverter();
        static Brush B(string hex) { return (Brush)Conv.ConvertFromString(hex); }
    }

    static class Program
    {
        [STAThread]
        static int Main(string[] args)
        {
            AppDomain.CurrentDomain.UnhandledException += (a, e) => Log.Write("crash: " + e.ExceptionObject);
            string dumpPath = null; bool minimized = false, settingsTab = false, exit = false, restart = false; int drop = 0;
            for (int i = 0; i < args.Length; i++)
            {
                if (args[i] == "--dump") dumpPath = i + 1 < args.Length ? args[i + 1] : Config.PathOf("T3Presence.dump.txt");
                else if (args[i] == "--minimized") minimized = true;
                else if (args[i] == "--settings") settingsTab = true;
                else if (args[i] == "--exit") exit = true;
                else if (args[i] == "--restart") restart = true;
                else if (args[i].StartsWith("--test-drop=")) int.TryParse(args[i].Substring(12), out drop);
            }
            var cfg = Config.Load();
            if (dumpPath != null)
            {
                File.WriteAllText(dumpPath, new Engine(cfg).Dump());
                return 0;
            }
            bool first;
            using (var mutex = new Mutex(true, @"Local\T3Presence.Singleton", out first))
            using (var show = new EventWaitHandle(false, EventResetMode.AutoReset, @"Local\T3Presence.Show"))
            using (var quit = new EventWaitHandle(false, EventResetMode.AutoReset, @"Local\T3Presence.Exit"))
            using (var again = new EventWaitHandle(false, EventResetMode.AutoReset, @"Local\T3Presence.Restart"))
            {
                if (!first)
                {
                    if (exit) quit.Set(); else if (restart) again.Set(); else show.Set();
                    Log.Write("second launch: asked the running copy to " + (exit ? "exit" : restart ? "restart" : "show itself"));
                    return 0;
                }
                if (exit || restart) return 0;
                Autostart.Refresh();
                var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
                app.DispatcherUnhandledException += (a, e) => { Log.Write("ui error: " + e.Exception); e.Handled = true; };
                var eng = new Engine(cfg) { TestDropAfter = drop };
                var ui = new Ui(eng, minimized);
                var waiter = new Thread(() => { while (show.WaitOne()) app.Dispatcher.BeginInvoke(new Action(ui.ShowWindow)); }) { IsBackground = true };
                waiter.Start();
                new Thread(() => { quit.WaitOne(); app.Dispatcher.BeginInvoke(new Action(ui.Quit)); }) { IsBackground = true }.Start();
                new Thread(() => { again.WaitOne(); app.Dispatcher.BeginInvoke(new Action(() => { Ui.Restart = true; ui.Quit(); })); }) { IsBackground = true }.Start();
                if (settingsTab) ui.OpenSettingsTab();
                eng.Start();
                app.Run();
                GC.KeepAlive(mutex);
            }
            // Tray Restart: the singleton mutex is released above, so the new copy starts as the first one.
            if (Ui.Restart)
            {
                Log.Write("restarting");
                try { Process.Start(new ProcessStartInfo(Assembly.GetExecutingAssembly().Location, "--minimized") { UseShellExecute = false }); } catch (Exception e) { Log.Write("restart failed: " + e.Message); }
            }
            return 0;
        }
    }
}
