# Windsurf Windows 切号重写（cockpit-tools 对齐修复）

## 背景

现有多账号功能已经完成账号存储、完整 SecretStorage 写入和 Provider current 提交，但 Windows 切号流程仍可能因错误 profile、Windsurf 进程未完全退出、DPAPI 环境异常或启动路径缺失而不可用。

本次不重新实现账号系统，而是针对 Windows 运行链路做修复，参考 `cockpit-tools/src-tauri/src/modules/windsurf_instance.rs` 的已用实现，保留 cc-switch 已有事务、备份、MCP 同步和 macOS 行为。

## 实施范围

1. `src-tauri/src/commands/windsurf.rs`
   - 切换改为：账号/Token/目录/DPAPI/启动路径预检 → 按 profile 关闭进程 → `ProviderService::switch` 注入并提交 current → 使用同一路径重启。
   - 启动路径缺失时在写登录态前返回 `APP_PATH_NOT_FOUND:windsurf`。
   - 注入失败时仅在原应用运行过的情况下尽力恢复启动；注入成功但重启失败继续返回 warning。

2. `src-tauri/src/windsurf/process.rs`
   - 移植 cockpit-tools 的 `--user-data-dir` 参数解析、路径规范化、目标 profile 匹配和 helper 排除。
   - 关闭流程增加优雅关闭、强制关闭、重新扫描和第二次重试。
   - `is_running`、关闭等待、重启统一使用同一 profile 和启动路径。
   - Windows 支持 `Windsurf.exe`、`Devin.exe`，以及对应安装目录内的 `Electron.exe`。

3. `src-tauri/src/windsurf/paths.rs`
   - 用户显式目录优先。
   - Devin/Windsurf 双目录按 `state.vscdb`、`codeium.installationId`、`windsurfAuthStatus` 确定性评分。
   - 状态展示、进程管理和注入统一使用同一解析结果。

4. `src-tauri/src/windsurf/auth_write.rs`
   - 将 Windows Local State、DPAPI 解密、AES-256-GCM v10 加密拆为与 cockpit-tools 对齐的辅助函数。
   - 增加写入前预检，明确区分 Local State、encrypted_key、DPAPI 和 AES key 长度错误。
   - 保留现有 Windsurf 键结构，不复制 Antigravity Credential Manager、机器 ID 或未经验证的 v11 方案。
   - macOS 分支保持不变。

5. `src-tauri/src/windsurf/inject.rs`
   - SQLite 增加有限 `busy_timeout`。
   - 保留写前备份和事务回滚。
   - 回读验证扩展到 auth status、两个 SecretStorage Buffer、selected auth 和 extension state。
   - 仅在提交和验证成功后更新 last-used；current provider 继续由 `ProviderService::switch` 在注入成功后提交。

6. `src-tauri/src/commands/workspace.rs`
   - cc-switch 写入受管 `AGENTS.md` 时，同步写入 `windsurf::paths::rules_path()`，默认即 `~/.codeium/windsurf/memories/global_rules.md`。
   - 继续尊重 `windsurf_rules_dir` 自定义目录；其它 Workspace 文件不镜像。

7. `src-tauri/src/mcp/windsurf.rs` / `src-tauri/src/services/mcp.rs`
   - Windsurf MCP 的默认目标固定为 `%APPDATA%/devin/mcp_config.json`。
   - 管理器保存、启停或定向同步 Windsurf MCP 时，全量投影并覆盖 `mcpServers`，移除已不再由 cc-switch 启用的陈旧项，同时保留文件中其它顶层字段。
   - 即使目标文件或父目录尚不存在也创建；`windsurf_mcp_dir` 自定义目录仍优先。

## 测试与验收

- 路径：双目录选择、installationId 优先、显式覆盖和稳定回退。
- 进程：引号/非引号 user-data-dir、helper 排除、目标 profile 匹配、Windows EXE 校验。
- DPAPI：Local State 缺失、JSON/base64/前缀错误、DPAPI 失败、32 字节 key、v10 加密格式。
- 注入：完整键写入、busy timeout、事务回滚、损坏/缺失行验证失败。
- 编排：启动路径缺失不得修改 DB/current；成功切换后才更新 current；启动失败仅 warning。
- 执行 `cargo fmt --check`、目标 Windsurf 测试、完整 `cargo test` 和可行的 Clippy。
- Windows 手工验证两个账号、运行/停止状态、Devin/Windsurf 双目录、手工 EXE 路径和启动失败场景。

## 明确不做

- 不使用 Antigravity 的 Windows Credential Manager。
- 不修改或伪造机器 ID / installationId。
- 不引入多实例 UI、自动切号或未经验证的新协议。
- 不改变 macOS SecretStorage 实现。
