# 2026-08-27 Windsurf OAuth 等待回调与静默切号

## 背景 / 根因

Phase A 已支持本机导入、Token/Auth1、邮箱密码登录与切号。仍缺两项：

1. 浏览器 OAuth（wait-callback）登录，以及手动粘贴回调 URL 兜底。
2. Windows 切号时 `taskkill` 弹出黑色控制台窗口；切号后应沿用同一 `user-data-dir` 自然恢复上次页面。

## 修改文件与核心符号

### 后端

- `src-tauri/src/windsurf/browser_oauth.rs`（新）
  - `start_login` / `complete_login` / `cancel_login` / `submit_callback_url`
  - 本地 axum 监听 `http://127.0.0.1:{port}/windsurf-auth-callback`
  - Firebase `access_token` → `RegisterUser` → `apiKey` / `apiServerUrl`（可选 `GetUserStatus`）
- `src-tauri/src/windsurf/account.rs` — `new_account_from_oauth`
- `src-tauri/src/windsurf/mod.rs` — 注册 `browser_oauth`
- `src-tauri/src/commands/windsurf.rs` — OAuth 四个 Tauri 命令，完成登录后 `upsert_account` + provider pointer
- `src-tauri/src/lib.rs` — 命令注册
- `src-tauri/src/windsurf/process.rs` — graceful/force `taskkill` 均加 `CREATE_NO_WINDOW`（`0x08000000`）；启动仍为 `--user-data-dir` + `--reuse-window`

### 前端

- `src/lib/api/windsurf.ts` — OAuth API + `WindsurfOAuthStartResponse`
- `src/lib/api/index.ts` — 导出 OAuth 类型
- `src/hooks/useWindsurf.ts` — OAuth mutations
- `src/components/windsurf/WindsurfAccountsPanel.tsx` — OAuth 对话框（打开浏览器、等待、手动粘贴回调、取消/重试）
- i18n：`zh` / `zh-TW` / `en` / `ja`

## 行为差异

### 之前

- 仅支持本机导入 / Token / Auth1 邮箱密码。
- Windows 关闭 Windsurf 时 `taskkill` 可能闪黑框。
- 无浏览器 OAuth wait-callback / 手动粘贴回调。

### 之后

- 新增「OAuth 授权」：打开 Windsurf 登录页 → 本地回调或粘贴回调 URL → 写入账号。
- 切号关闭进程时静默 `taskkill`；重启使用同一 user-data-dir + `--reuse-window`，由 Windsurf 自然恢复上次工作区/页面。
- 密码登录与 Token 添加路径保持不变。

## 测试

- 本机无 `cargo`，未执行 `cargo check` / `cargo test` / `cargo clippy`。
- `node_modules/.bin/tsc --noEmit`：通过。
- Prettier（Windsurf 前端/API 与四份 i18n）：通过。
- `git diff --check`：通过。
- 全量 Vitest 运行超过 10 分钟，因扫描到 `.claude/worktrees/*` 中的重复测试副本而未完成，已停止；未获得完整单测结论。
- `pnpm typecheck` / `pnpm exec prettier` 被本机 `ERR_PNPM_IGNORED_BUILDS` 策略拦截，改用已安装的同一工具二进制完成验证，未更改依赖审批。
- 已静态核对：模块注册、命令导出、account upsert、CREATE_NO_WINDOW、前端 API/面板/i18n。
- 额外修复：OAuth 回调端口改为持有 listener 再启动，避免 bind-release-rebind 竞态；前端 OAuth 重试用 flowId 忽略过期请求结果。
- 剩余风险：需 CI 编译；真机需验证 OAuth 回调、手动粘贴、取消/超时、切号无黑窗与上次页面恢复。

## Git

- 分支：`release/windsurf-v3.19.2`
- 功能提交：`c344cb82`（Windsurf OAuth、静默切号、前端与本归档同一提交）
- 分支推送：已成功推送到公开 fork `https://github.com/chunzhimoe/cc-switch` 的 `release/windsurf-v3.19.2`
- Release 标签：`v3.19.2-windsurf.4` 已成功推送，并触发 Release workflow
- 排除：`pnpm-workspace.yaml`、`cockpit-tools/`、`WindsurfAPI/`、`@plan/`、`.waylog/`
