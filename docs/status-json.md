# status.json

agent-usage-bar rewrites `%LOCALAPPDATA%\agent-usage-bar\status.json` after
every refresh (atomically: a temporary file and a rename). It is UTF-8 JSON
without a BOM, meant for other programs such as wdock's top bar.

`schema` is bumped only on breaking changes; new fields may appear in any
version, so ignore fields you do not know.

```json
{
  "schema": 1,
  "updated_at": "2026-10-29T07:24:34.31Z",
  "refresh_minutes": 10,
  "paused": false,
  "agents": [
    {
      "id": "claude",
      "name": "Claude",
      "shown": true,
      "status": "ok",
      "message": null,
      "plan": "pro",
      "source": "claude-code",
      "fetched_at": "2026-10-29T07:24:34.30Z",
      "windows": [
        { "id": "five_hour", "label": "5h", "used_percent": 60.0, "resets_at": "2026-10-29T11:40:00Z" },
        { "id": "seven_day", "label": "7d", "used_percent": 8.0, "resets_at": "2026-11-04T11:00:00Z" }
      ]
    }
  ],
  "github": {
    "shown": true,
    "status": "ok",
    "message": null,
    "fetched_at": "2026-10-29T07:24:35Z",
    "login": "octocat",
    "total": 130,
    "days": [ { "date": "2026-10-29", "count": 1, "level": 1 } ]
  }
}
```

| Field | Meaning |
| --- | --- |
| `updated_at` | When the file was written (RFC 3339, UTC). |
| `refresh_minutes` | Configured refresh interval. |
| `paused` | The last refresh skipped network sources because a full-screen app was in front. |
| `agents[]` | Always `claude`, `codex`, `antigravity`, in that order. |
| `agents[].shown` | The user turned this agent on; hidden agents are not refreshed. |
| `agents[].status` | `pending` (not fetched yet), `ok`, `sign_in_required`, `not_found` (no local data), `not_running` (Antigravity closed) or `error`. |
| `agents[].message` | Human-readable reason when `status` is not `ok`. With `error`, `windows` still holds the last good numbers. |
| `agents[].plan` | Plan name when the source reports one (`pro`, `plus`, `Pro`...). |
| `agents[].source` | `sign-in` / `claude-code` (Claude), `session-log` (Codex), `language-server` (Antigravity). |
| `agents[].fetched_at` | When `windows` were last fetched successfully. |
| `agents[].windows[]` | Rate-limit windows, most important first; the tray icon draws the first two. |
| `windows[].id` | Source id: `five_hour`, `seven_day`, `seven_day_opus`, `primary`, `secondary`, `gemini-weekly`... |
| `windows[].label` | Short label: `5h`, `7d`, `Opus 7d`, `Gemini 7d`... |
| `windows[].used_percent` | 0–100, share of the window used. |
| `windows[].resets_at` | When the window resets (RFC 3339), or `null` if unknown. |
| `github.days[]` | The last year of the GitHub contribution calendar, oldest first, starting on a Sunday. `level` is GitHub's shade, 0–4. |
