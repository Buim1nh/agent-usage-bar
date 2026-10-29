# agent-usage-bar

A small Windows tray app that shows how much of your AI coding-agent limits
you have used: **Claude**, **Codex** and **Antigravity**, plus a shared
**GitHub** section with your contribution graph, drawn like the one on your
GitHub profile.

[Tiếng Việt](README.vi.md)

- The tray icon draws two bars per shown agent (5-hour and weekly window),
  green, amber from 70 %, red from 90 %. Hover for the numbers.
- Click the icon for the popover: a toggle per agent (hidden agents are not
  fetched), every window with its reset time, the GitHub contribution graph,
  and buttons to refresh, change the interval (5–60 min, default 10) or quit.
- Right-click for sign-in, sign-out, the data folder and the full-screen
  pause option.
- Refreshes on a timer only; no busy polling. Network refreshes are skipped
  while a full-screen app (game, video) is in front and resume once it is gone.
- Writes the current numbers to `%LOCALAPPDATA%\agent-usage-bar\status.json`
  ([schema](docs/status-json.md)) so other tools, such as
  [wdock](https://github.com/Buim1nh/wdock)'s top bar, can show them.

It shows in any notification area: explorer's taskbar or wdock's top bar.

## Where the numbers come from

| Agent | Source | Needs |
| --- | --- | --- |
| Claude | `GET https://api.anthropic.com/api/oauth/usage`, the endpoint behind Claude Code's `/usage` | Claude Code signed in, or "Sign in with Claude…" in the app |
| Codex | the newest `token_count` event in `~/.codex/sessions/**/*.jsonl` (`$CODEX_HOME` respected); Codex writes the server's rate limits there every turn | Codex CLI used on this PC; numbers are as fresh as your last Codex turn |
| Antigravity | `RetrieveUserQuotaSummary` / `GetUserStatus` on the Antigravity IDE's local language server (127.0.0.1, found from the process's command line) | Antigravity IDE running |
| GitHub | `gh api graphql` (contribution calendar) | [GitHub CLI](https://cli.github.com) signed in (`gh auth login`) |

Antigravity's API is internal and undocumented and may change; when the
IDE is closed the row says "Not running".

## Credentials and privacy

- **Claude:** the app reads Claude Code's `~/.claude/.credentials.json`
  read-only and never refreshes that token (refreshing rotates it and would
  sign Claude Code out); when it has expired, using Claude Code renews it. If
  you prefer an independent sign-in, choose "Sign in with Claude…": a normal
  browser OAuth (PKCE) flow; the token is stored encrypted with Windows DPAPI
  for your user in `%LOCALAPPDATA%\agent-usage-bar\claude-token.bin` and
  refreshed by the app. "Sign out of Claude" deletes it.
- **Codex** needs no credentials (local files only).
- **Antigravity:** the language server's CSRF token is read from its command
  line, kept in memory and only sent to 127.0.0.1.
- **GitHub:** the app stores nothing; `gh` uses its own sign-in.
- Tokens are never logged. The log (`agent-usage-bar.log`) holds status codes
  and short reasons only. The app sends no telemetry.

## Install

Needs Rust (stable) to build. No admin rights.

```powershell
git clone https://github.com/Buim1nh/agent-usage-bar
cd agent-usage-bar
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

This builds the release binary, copies it to
`%LOCALAPPDATA%\Programs\agent-usage-bar`, registers a per-user scheduled task
`agent-usage-bar` that starts it at sign-in, and starts it now. Options:
`-NoBuild`, `-NoStart`, `-Exe <path to a built exe>`.

Uninstall:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1 -RemoveFiles
```

`-RemoveData` also deletes `%LOCALAPPDATA%\agent-usage-bar` (config,
status.json, log, the encrypted Claude sign-in).

wdock users can install both at once with `scripts\install.ps1 -WithUsage`
in the wdock repo.

## Configuration

`%LOCALAPPDATA%\agent-usage-bar\config.json` (UTF-8, no BOM), written by the
popover; you can also edit it while the app is closed:

```json
{
  "refresh_minutes": 10,
  "shown": ["claude", "codex", "antigravity"],
  "pause_when_fullscreen": true,
  "show_github": true
}
```

`refresh_minutes` is clamped to 5–240.

## Command line

- `agent-usage-bar.exe` — the tray app (one instance per session).
- `agent-usage-bar.exe --show` — the same, opening the popover at start.
- `agent-usage-bar.exe --once` — fetch every agent once, write status.json and
  print it, without a tray icon.

## Build and test

```powershell
cargo clippy --release --all-targets
cargo test
```

## Credits

Ideas and protocol details come from these projects; no code was copied.

- [Blimp-Labs/claude-usage-bar](https://github.com/Blimp-Labs/claude-usage-bar)
  (BSD 2-Clause): the Claude usage endpoint, its response fields and the
  PKCE sign-in flow.
- [steipete/CodexBar](https://github.com/steipete/CodexBar) (MIT): the
  Antigravity language-server protocol and Claude source notes.
- [Henrik-3/AntigravityQuota](https://github.com/Henrik-3/AntigravityQuota)
  (MIT): Antigravity process discovery and quota fields on Windows.
- [xiangz19/codex-ratelimit](https://github.com/xiangz19/codex-ratelimit):
  where Codex records rate limits in its session logs.

Not affiliated with Anthropic, OpenAI, Google or GitHub.

## License

MIT, see [LICENSE](LICENSE).
