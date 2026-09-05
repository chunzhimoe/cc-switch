# 2026-09-05 Windsurf Windows 切号重写

## 背景与根因

Windows 上的 Windsurf 多账号数据与 SecretStorage 写入已存在，但切换编排和运行时管理不够可靠：

- 启动路径缺失时仍可能先写入登录态，造成“显示已切换但应用未启动”的半完成状态。
- 进程关闭只依赖进程名，没有按 `--user-data-dir` 识别目标 profile，也缺少关闭后重扫与重试，`state.vscdb` 容易继续被占用。
- `%APPDATA%\Devin` 与 `%APPDATA%\Windsurf` 同时存在时，目录选择未优先识别稳定的 `codeium.installationId`。
- Windows DPAPI、Local State 和 AES key 错误只能在关闭应用后的实际写入阶段暴露。
- 写入验证只检查 `windsurfAuthStatus.apiKey`，不能确认两个 SecretStorage Buffer 和相关状态键都已完成。
- cc-switch 管理 Workspace `AGENTS.md` 时没有同步 Windsurf `global_rules.md`；Windsurf MCP 同步也不是对 `%APPDATA%\devin\mcp_config.json` 的完整投影。

本次参考 `cockpit-tools/src-tauri/src/modules/windsurf_instance.rs` 的 Windsurf Windows 实现重写关键链路，不采用与 Windsurf 无关的 Antigravity Windows Credential Manager 或机器 ID 修改。

## 修改文件与核心行为

### `src-tauri/src/commands/windsurf.rs`

- `switch_windsurf_account` 改为：账号/Token/profile/DPAPI/启动路径预检 → 关闭目标进程 → `ProviderService::switch` 注入并提交 current → 使用同一路径重启。
- `APP_PATH_NOT_FOUND:windsurf` 在登录态写入前返回。
- 注入失败时仅在切换前应用正在运行的情况下尽力恢复启动；注入成功但启动失败继续作为 warning 返回。
- 手动设置 EXE 路径统一复用 `process::is_valid_launch_path`。

### `src-tauri/src/windsurf/process.rs`

- 新增 `WindsurfProcess`，解析进程命令行中的 `--user-data-dir`。
- 排除 renderer/GPU/utility/crashpad/sandbox 等 helper 进程。
- `close_for` 使用非强制关闭、等待、强制关闭、重新扫描和第二次强制关闭；最终错误包含残留 PID。
- 没有显式 `--user-data-dir` 的正常单 profile 进程也视为受管，防止未关闭就写 SQLite。
- `start_with` 复用预检得到的 EXE/profile，并继续以隐藏窗口方式传入 `--user-data-dir` 和 `--reuse-window`。
- Windows 识别 `Windsurf.exe`、`Devin.exe` 和品牌目录中的 `Electron.exe`。

### `src-tauri/src/windsurf/paths.rs`

- 明确保持用户目录覆盖最高优先级。
- 默认候选按可读 `state.vscdb`、`codeium.installationId`、登录态和固定候选顺序进行确定性选择。
- 避免同分时在 Devin/Windsurf 目录之间不稳定跳转。

### `src-tauri/src/windsurf/auth_write.rs`

- 将 Windows SecretStorage 拆分为 Local State 定位、DPAPI 解密、32 字节 key 校验和 AES-256-GCM v10 加密函数。
- 新增 `validate_profile_encryption`，在关闭 Windsurf 前检查 DPAPI 环境。
- DPAPI 空输出安全失败并释放系统内存。
- 已存在 v11 secret 时明确拒绝 Windows 写入，避免静默降级覆盖。
- macOS Keychain/AES-CBC 分支保持不变。

### `src-tauri/src/windsurf/inject.rs`

- SQLite 增加 3 秒 `busy_timeout`。
- 继续在写前生成 `state.vscdb.cc-switch.bak.*`。
- 在同一事务内验证 auth status、sessions/API server SecretStorage Buffer、selected auth 和 extension state；验证失败时先 `ROLLBACK`，不再提交后才发现损坏。
- 仅在验证和提交成功后更新账号 `last_used`；Provider current 继续由 `ProviderService::switch` 在注入成功后提交。

### `src-tauri/src/commands/workspace.rs`

- cc-switch 写入受管 `AGENTS.md` 时，同步写入 `windsurf::paths::rules_path()`。
- 默认目标为 `~/.codeium/windsurf/memories/global_rules.md`，并继续尊重 `windsurf_rules_dir` 覆盖。
- Windsurf 镜像写入失败时回滚本次 Workspace `AGENTS.md` 写入，避免两份规则静默分叉。

### `src-tauri/src/mcp/windsurf.rs`、`src-tauri/src/services/mcp.rs`、`src-tauri/src/mcp/mod.rs`

- 默认 MCP 目标保持 `%APPDATA%\devin\mcp_config.json`；自定义 `windsurf_mcp_dir` 仍优先。
- 移除父目录不存在时的静默跳过，允许管理操作创建目标目录与文件。
- Windsurf MCP 保存、启停和定向同步改为一次性重建 `mcpServers`，确保文件与 cc-switch 中启用的 Windsurf MCP 完全一致。
- 保留 `mcp_config.json` 中 `mcpServers` 以外的顶层字段。

## 行为差异

### 修改前

- 可能在找不到 EXE 时仍完成数据库切换。
- Windsurf 主进程未完全退出时直接写 `state.vscdb`，容易得到 database locked。
- 双品牌目录可能选错 profile。
- DPAPI 和 SecretStorage 错误出现得晚，验证范围不足。
- Workspace `AGENTS.md`、Windsurf `global_rules.md` 与 Devin `mcp_config.json` 可能不同步。

### 修改后

- Windows 切号只有在预检通过后才关闭应用并写入登录态。
- 进程关闭和 SQLite 写入有明确的等待、重试、超时与事务边界。
- 使用同一个 profile 完成状态展示、进程管理、注入和重启。
- 必需登录态键在提交前完成回读验证。
- cc-switch 管理的 `AGENTS.md` 同步到 Windsurf 全局规则；Windsurf MCP 对 Devin 配置执行全量投影。

## 测试与验证

### 已新增测试

- profile 选择优先 installationId，且同分保持固定顺序。
- `--user-data-dir` 两种参数形式、helper 排除、profile 匹配与 Windows EXE 校验。
- Windows Local State 缺失、错误 DPAPI 前缀、v11 拒绝和 AES-GCM v10 往返。
- 完整登录态回读成功及 SecretStorage 损坏失败。
- `AGENTS.md` 仅镜像到 `global_rules.md`。
- Windsurf MCP 全量替换 `mcpServers` 且保留其它顶层字段。

### 已执行

- `git diff --check`：通过。
- GitHub Actions 首次运行 `cargo fmt --check --manifest-path src-tauri/Cargo.toml` 时仅报告 rustfmt 排版差异；已严格按 CI 输出修正。
- 后续 Windows 编译暴露 5 个确定错误：`concat!` 格式串无法捕获变量、未使用的 Windsurf MCP re-export、以及 profile 评分中临时 JSON 值借用逃逸；均已修复，等待下一次 CI 复验。
- 代码差异人工复核，并针对审查发现修正：事务提交前验证、无参数 Windsurf 主进程管理、PID 重用等待逻辑、Windows v11 安全拒绝、AGENTS 镜像失败回滚。

### 未执行

- 本机没有 `cargo` / `rustfmt`，未运行 Rust 格式检查、编译、单元测试或 Clippy。
- 尚未在 Windows 真机使用两个真实 Windsurf 账号完成端到端切换。
- 推送后需以 GitHub Actions 和真机切换结果为最终验证依据。

## 已知风险与后续事项

- 现有 Windows v11 SecretStorage profile 会安全拒绝写入，尚未实现 Windows v11 加密。
- Windsurf MCP 现在以 cc-switch 数据库为权威，全量覆盖 `mcpServers`；直接手工写入但未导入 cc-switch 的服务器会在下一次同步时被移除，这是本次明确要求的覆盖语义。
- 进程与 DPAPI 行为仍需在当前 Windows 用户、Devin/Windsurf 双目录及自定义路径场景实测。
- Rust 格式与编译错误只能由具备工具链的环境发现。

## Git 状态

- 开发分支：`release/windsurf-v3.19.4`
- 功能提交：`8944103d`（`fix(windsurf): rewrite Windows account switching`）
- 推送 `origin/main`（`farion1231/cc-switch`）失败：当前账号无该上游仓库写权限，GitHub 返回 HTTP 403。
- 经用户明确指定并授权，已将 `8944103d` 非强制推送到 `https://github.com/chunzhimoe/cc-switch.git` 的 `main`。
- Rust 编译与测试仍需以目标仓库后续 CI 结果为准。
