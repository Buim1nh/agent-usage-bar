# agent-usage-bar

Ứng dụng khay hệ thống (tray) nhỏ cho Windows, cho biết bạn đã dùng bao nhiêu
hạn mức của các AI coding agent: **Claude**, **Codex** và **Antigravity**, kèm
một mục **GitHub** chung hiển thị biểu đồ đóng góp (commit) giống như trên
trang hồ sơ GitHub.

[English](README.md)

- Biểu tượng trên khay vẽ hai thanh cho mỗi agent đang bật (cửa sổ 5 giờ và
  7 ngày): xanh lá, chuyển cam từ 70 %, đỏ từ 90 %. Rê chuột để xem số.
- Bấm vào biểu tượng để mở popover dạng kính mờ (acrylic): hạn mức chính của
  mỗi agent hiện bằng số % lớn kèm thời gian reset, các cửa sổ còn lại là
  thanh mỏng, biểu đồ đóng góp GitHub, các chip bật/tắt từng agent và GitHub
  (agent tắt thì không lấy dữ liệu), cùng nút làm mới và đổi chu kỳ (5–60
  phút, mặc định 10).
- Chuột phải để đăng nhập, đăng xuất, mở thư mục dữ liệu, bật/tắt tạm dừng
  khi có ứng dụng toàn màn hình và thoát.
- Chỉ làm mới theo bộ hẹn giờ, không polling liên tục. Khi một ứng dụng toàn
  màn hình (game, video) đang ở phía trước, các lần làm mới qua mạng được bỏ
  qua và tiếp tục khi ứng dụng đó thoát.
- Ghi số liệu hiện tại vào `%LOCALAPPDATA%\agent-usage-bar\status.json`
  ([schema](docs/status-json.md)) để công cụ khác, ví dụ thanh trên của
  [wdock](https://github.com/Buim1nh/wdock), hiển thị lại.

Hiện được trên mọi vùng thông báo: taskbar của explorer hoặc thanh trên của
wdock.

## Số liệu lấy từ đâu

| Agent | Nguồn | Cần |
| --- | --- | --- |
| Claude | `GET https://api.anthropic.com/api/oauth/usage`, endpoint mà lệnh `/usage` của Claude Code dùng | Claude Code đã đăng nhập, hoặc chọn "Sign in with Claude…" trong ứng dụng |
| Codex | sự kiện `token_count` mới nhất trong `~/.codex/sessions/**/*.jsonl` (tôn trọng `$CODEX_HOME`); Codex ghi hạn mức của máy chủ vào đó sau mỗi lượt | Đã dùng Codex CLI trên máy; số liệu mới bằng lượt Codex gần nhất |
| Antigravity | `RetrieveUserQuotaSummary` / `GetUserStatus` trên language server cục bộ của Antigravity IDE (127.0.0.1, tìm qua dòng lệnh của tiến trình) | Antigravity IDE đang chạy |
| GitHub | `gh api graphql` (lịch đóng góp) | [GitHub CLI](https://cli.github.com) đã đăng nhập (`gh auth login`) |

API của Antigravity là API nội bộ, không có tài liệu và có thể đổi; khi IDE
đóng, dòng đó ghi "Not running".

## Thông tin đăng nhập và quyền riêng tư

- **Claude:** ứng dụng chỉ đọc `~/.claude/.credentials.json` của Claude Code
  và không bao giờ làm mới token đó (làm mới sẽ xoay vòng token và đăng xuất
  Claude Code); khi token hết hạn, chỉ cần dùng Claude Code là nó tự gia hạn.
  Nếu muốn đăng nhập riêng, chọn "Sign in with Claude…": luồng OAuth (PKCE)
  bình thường trên trình duyệt; token được mã hoá bằng Windows DPAPI cho
  riêng tài khoản của bạn, lưu ở
  `%LOCALAPPDATA%\agent-usage-bar\claude-token.bin` và do ứng dụng tự làm
  mới. "Sign out of Claude" xoá file đó.
- **Codex** không cần thông tin đăng nhập (chỉ đọc file cục bộ).
- **Antigravity:** CSRF token của language server đọc từ dòng lệnh, chỉ giữ
  trong bộ nhớ và chỉ gửi tới 127.0.0.1.
- **GitHub:** ứng dụng không lưu gì; `gh` dùng phiên đăng nhập của chính nó.
- Không bao giờ ghi token vào log. Log (`agent-usage-bar.log`) chỉ có mã trạng
  thái và lý do ngắn. Ứng dụng không gửi telemetry.

## Cài đặt

Cần Rust (stable) để build. Không cần quyền admin.

```powershell
git clone https://github.com/Buim1nh/agent-usage-bar
cd agent-usage-bar
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

Script sẽ build bản release, chép vào `%LOCALAPPDATA%\Programs\agent-usage-bar`,
đăng ký scheduled task theo người dùng tên `agent-usage-bar` để chạy khi đăng
nhập, rồi khởi động luôn. Tuỳ chọn: `-NoBuild`, `-NoStart`, `-Exe <đường dẫn
exe đã build>`.

Gỡ cài đặt:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1 -RemoveFiles
```

`-RemoveData` xoá thêm `%LOCALAPPDATA%\agent-usage-bar` (config, status.json,
log, phiên đăng nhập Claude đã mã hoá).

Người dùng wdock có thể cài cả hai một lần bằng `scripts\install.ps1 -WithUsage`
trong repo wdock.

## Cấu hình

`%LOCALAPPDATA%\agent-usage-bar\config.json` (UTF-8, không BOM), do popover
ghi; bạn cũng có thể sửa tay khi ứng dụng đang tắt:

```json
{
  "refresh_minutes": 10,
  "shown": ["claude", "codex", "antigravity"],
  "pause_when_fullscreen": true,
  "show_github": true
}
```

`refresh_minutes` được giới hạn trong khoảng 5–240.

## Dòng lệnh

- `agent-usage-bar.exe` — ứng dụng khay (một tiến trình mỗi phiên).
- `agent-usage-bar.exe --show` — như trên, mở popover ngay khi khởi động.
- `agent-usage-bar.exe --once` — lấy dữ liệu mọi agent một lần, ghi
  status.json và in ra, không tạo biểu tượng khay.

## Build và kiểm thử

```powershell
cargo clippy --release --all-targets
cargo test
```

## Ghi công

Ý tưởng và chi tiết giao thức lấy từ các dự án sau; không sao chép mã nguồn.

- [Blimp-Labs/claude-usage-bar](https://github.com/Blimp-Labs/claude-usage-bar)
  (BSD 2-Clause): endpoint usage của Claude, các trường phản hồi và luồng
  đăng nhập PKCE.
- [steipete/CodexBar](https://github.com/steipete/CodexBar) (MIT): giao thức
  language server của Antigravity và ghi chú về nguồn dữ liệu Claude.
- [Henrik-3/AntigravityQuota](https://github.com/Henrik-3/AntigravityQuota)
  (MIT): cách tìm tiến trình Antigravity và các trường quota trên Windows.
- [xiangz19/codex-ratelimit](https://github.com/xiangz19/codex-ratelimit):
  nơi Codex ghi hạn mức trong log phiên.

Không liên kết với Anthropic, OpenAI, Google hay GitHub.

## Giấy phép

MIT, xem [LICENSE](LICENSE).
