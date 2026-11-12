# T3 Presence
Discord Rich Presence for [T3 Code](https://github.com/pingdotgg/t3code) as one portable Windows app. Unofficial, not affiliated with T3 or Discord.

## Install
1. Download `T3Presence.exe` from the [latest release](https://github.com/HighestFidelity/t3-presence/releases/latest) (built from this repo by GitHub Actions, SHA256 alongside).
2. Put it in its own folder, e.g. `%LOCALAPPDATA%\T3Presence\`, and run it. Needs Windows 10/11, T3 Code and the Discord desktop app. Nothing else to install.

Or let your agent do it: paste the prompt in [AGENT_SETUP.md](AGENT_SETUP.md) into T3 Code. It installs, finds your hub and other machines, and checks the numbers.

Open it and your presence is on. X hides it to the tray, tray Exit clears the presence and quits, tray Restart starts a fresh copy. Click a page chip under Rotation to show that page now. It shows as Watching by default, so it sits next to a game instead of replacing it.

It costs zero model tokens: no model or API calls. It reads T3's files and talks to the Discord app over its local pipe. No Discord token is involved, the local pipe only needs an Application ID.

## Pages
They rotate every 15 s. Toggle each in Settings. Activity and Agents list every project with a running agent.

| Page | Line 1 | Line 2 | Image hover |
|---|---|---|---|
| Activity | Working in my-project | Editing a file | Model and effort |
| Agents | 3 agents running + 2 sub in my-project, other-project | 2x Opus 5.5 max, Opus 5.5 high | Context 157k of 1M |
| Tokens | Today: 213k in, 723k out | Cache: 74.9M read, 2.8M write | Last 7 days: 136.2M tokens |
| Value | Token value today: $40.08 | Last 30 days: $220.31 | API list prices, not a bill. |
| Plan | 5h limit: 55% used, resets in 4h37m | Week limit: 62% used, resets in 1d11h | Model and effort |
| Plan (hub, 2 accounts) | 5h limits: 16% / 100% used, resets 4h56m / 1h46m | Week limits: 24% / 51% used, resets 3d17h / 6d0h | 2 accounts pooled through the hub |

Friends can see all of this. The live line uses fixed wording per item kind, and unknown tools read "Using a tool". Thread titles, messages, file paths and commands are never read for display. `showProject: false` hides project names too.

## All environments
By default it counts this PC. To add a linked machine, put it in `remotes` in `config.json` (see `config.example.json`):

- `ssh`, `port`, `key`: one SSH session using the Windows ssh client. `src/remote-state.py` is sent inline and prints T3's state every 15 s. It needs `python3` on that machine, installs nothing and writes nothing.
- `userData`, `transcripts`: that machine's `~/.t3/userdata` and `~/.claude/projects` as this PC can read them, normally a network share. Never open a T3 database over a share: SQLite's WAL mode isn't safe there, which is why live state goes over SSH.

Counts and token totals add up across machines. Project, model and context come from whichever ran last. If a machine is away, the window says so and the app keeps retrying quietly. The Settings toggle turns this off.

## Where the numbers come from
- Tokens and value: the same arithmetic as T3's Usage page. `usage-scan-cache-v*.json` rows plus what the Claude transcripts grew since, deduped on message id + request id, priced from `usage-model-rates.json`. Refreshed at most every 2 min, and only new bytes get read. The 30 days match that page's default window; `valueDays` changes it.
- Plan limits: with a CLIProxy hub, every enabled Claude account's real 5h and week limits, read like T3's own hub source (management `auth-files`, then `api-call` to Anthropic's OAuth usage endpoint), every 3 min. Hub URL comes from T3's `usageLimitSources` (or `hubUrl` in config.json); T3 masks the management key, so put it in `hub.key` next to the exe. The red limit dot only shows when every account is past `limitPercent`. No hub or hub down: the newest `rate_limit_event` in T3's provider logs, which only shows whichever account answered last.
- Live state: T3's `statev2.sqlite`, opened read-only. As with any WAL reader, SQLite itself touches the `-shm` read marks. Nothing else writes to T3 files.

These are T3's private formats, so a T3 update can break this.

## Images
- Large image: `projectImages`. Lookup order is `"<project>|<state>"`, `"<project>"`, `"*|<state>"`, then `largeImageKey`. States are `working`, `idle`, `limit`. The Settings tab edits the entry for the project on screen.
- Small overlay: `stateImages`, defaulting to public Twemoji dots on jsDelivr. Discord fetches those, the app never does. `limit` applies from `limitPercent` (80).
- Discord only accepts a public https URL or an art asset uploaded in the Developer Portal, so generated per-project covers need one of those.

## Build
```
powershell -ExecutionPolicy Bypass -File build.ps1
```
Makes the multi-size icon from `t3-logo-1024.png`, runs `cargo build --release` and copies the exe next to the script. Needs Rust (rustup, MSVC toolchain). Written in Rust: egui window, a Win32 tray, bundled SQLite opened read-only. `cargo test` checks the text formats.

`config.json` sits next to the exe and is created on first run. `clientId` defaults to a shared Discord application named T3 Code; to use your own, create one in the Developer Portal and put its Application ID there.

## Flags
`--dump [file]` writes every page's JSON and exits. `--minimized` starts in the tray (Start with Windows uses it). `--settings` opens the Settings tab. `--exit` closes the running copy. `--restart` restarts it. `--test-drop=N` drops the Discord pipe once after N seconds.
