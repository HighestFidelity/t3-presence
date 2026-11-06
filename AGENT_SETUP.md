# Set up T3 Presence with an AI agent

Paste the prompt below into T3 Code (or any coding agent that can run PowerShell on your Windows PC). It installs the app, wires in what it can find, and checks that the numbers are real.

---

```text
Set up T3 Presence (Discord Rich Presence for T3 Code) for me on this Windows PC.
Repo: https://github.com/HighestFidelity/t3-presence. Read its README first.

1. Check prerequisites: Windows 10/11, T3 Code installed and used at least once
   (%USERPROFILE%\.t3\userdata\statev2.sqlite exists), and the Discord desktop app installed.
   If one is missing, tell me which and stop.
2. Install: download the latest release asset T3Presence.exe and T3Presence.exe.sha256 from
   https://github.com/HighestFidelity/t3-presence/releases/latest into
   %LOCALAPPDATA%\T3Presence\ (create it). Verify the SHA256 matches before running anything.
   Close a running copy first with: & "$env:LOCALAPPDATA\T3Presence\T3Presence.exe" --exit
3. Verify data: run & "$env:LOCALAPPDATA\T3Presence\T3Presence.exe" --dump "$env:TEMP\t3p.txt",
   wait 3 seconds, and read the first line (JSON). "error" must be null, and "threads" should
   be above 0. "today" counts from local midnight, so small numbers early in the day are normal.
4. CLIProxyAPI hub (only if I use one): look in %USERPROFILE%\.t3\userdata\settings.json under
   "usageLimitSources" for an entry with "kind": "cliproxy". If there is one, T3 masks its
   management key, so ask me for that key (or where I keep it). Never print it in chat or logs.
   Write it with no trailing newline to %LOCALAPPDATA%\T3Presence\hub.key. If the hub URL is not
   in T3's settings, set "hubUrl" in config.json next to the exe. Re-run the dump: "planSource"
   must be "hub" and "planAccounts" should list one entry per Claude account in the hub.
   If "hubError" is set, report it.
   No hub: skip this. Plan limits then come from T3's own logs.
5. Other machines (only if I run T3 on another machine too, e.g. a server or a Mac): ask me for
   its SSH target, port and key, and how this PC reaches its ~/.t3/userdata and
   ~/.claude/projects (normally a mapped network share). Add it to "remotes" in config.json
   (format in config.example.json). It needs python3 there. Skip if I only use this PC.
6. Start it: & "$env:LOCALAPPDATA\T3Presence\T3Presence.exe". Ask me whether it should start
   with Windows; if yes, set HKCU\Software\Microsoft\Windows\CurrentVersion\Run value
   "T3Presence" to "\"<full path to exe>\" --minimized" (same as the Settings toggle).
7. Report: install path, which pages show data, hub on or off, remotes added, and anything
   that needs me. Tell me my Discord profile should now show "Watching T3 Code".
   Discord activity sharing must be on: Discord Settings > Activity Privacy.
```

---

Notes for the agent:
- The app only reads T3's files and talks to Discord's local pipe. It never calls a model and needs no Discord token.
- `config.json` is created next to the exe on first run. Edit it while the app is closed (`--exit`), then start it again.
- SmartScreen may warn on first run because the exe is unsigned. It is built from this repo by GitHub Actions; the checksum is on the release.
