# 2026-09-06 Windsurf macOS 切号与重启修复

## 背景与根因

macOS 上 Windsurf 切号后无法可靠关闭/重启：旧 macOS 进程路径沿用 Windows 风格的
可执行名匹配，认不出 Electron bundle（`Windsurf.app` 内实际二进制），且切号命令在
写入前未保证关闭、前后端对 `restarted: false` 的语义不一致，导致“已切换”误报。

## 范围（生产文件）

- `src-tauri/src/windsurf/process.rs` — macOS 分发到新增 `process/macos.rs`
- `src-tauri/src/windsurf/process/macos.rs` — 新增，macOS 专用实现
- `src-tauri/src/commands/windsurf.rs` — 切号闭包：先关闭再写，预检缺路径返回 Err，
  恢复启动错误透出
- `src-tauri/src/windsurf/auth_write.rs` — macOS 使用真实 Keychain 预检/自定义 profile
  品牌选择应用，crypto 不变
- `src-tauri/src/windsurf/inject.rs` — macOS 最终 `ensure_stopped_for` 守卫
- `src/components/windsurf/WindsurfAccountsPanel.tsx` — `restarted: false` 永不 toast 成功
- `src/i18n/locales/{zh,en,ja,zh-TW}.json` — 四语言对应文案
- `.github/workflows/ci.yml` — 增加精确分支触发
- `tests/components/WindsurfAccountsPanel.test.tsx` — 新增面板测试（5 用例）

## 核心符号

- `macos::{is_valid_launch_path, validate_launch_profile, detect_and_save_launch_path,
  is_running, is_running_for, ensure_stopped_for, close_for, start_with}`
- 内部：`bundle_root / bundle_brand / profile_brand / resolve_bundle / is_main_executable /
  parse_process_line / profile_argument / collect_main_processes / matches_profile /
  matching_processes / wait_for_exit / signal_processes / launch_command`

## 行为差异（修改前 → 修改后）

- 进程识别：可执行名匹配 → `/bin/ps` 取 pid/stat/comm 再取 args，profile + brand
  默认匹配，区分同品牌多 profile
- 启动路径校验：简单存在性检查 → `.app`/bundle 内路径解析 +
  `plutil CFBundleExecutable` 校验
- 重启：直接拉起 → `open -n -a --args --user-data-dir --new-window`，清除污染环境变量，
  有界 launcher 等待 + tempfile 有界 stderr，返回稳定 actualPID 而非 openPID
- 切号顺序：初始检测漏掉 Electron 时跳过关闭 → 每次写入前重新扫描并确认已退出；
  预检缺路径 warning → Err；恢复启动失败吞掉 → 透出错误。
  旧实现发现关闭失败时原本也会停止写入，本次修的是漏检导致跳过关闭，不是写后关闭。
- 前端：`restarted: false` 仍 toast 成功 → 永不报成功，走四语言警告文案

## 测试结果（实测）

- `pnpm typecheck` — PASS
- `pnpm format:check` — PASS
- `pnpm build:renderer` — PASS（既有 outdated browserslist、chunk size/dynamic import 警告）
- 新增面板测试 — 5 PASS
- `pnpm exec vitest run --dir src` — 8 文件 87 PASS
- `pnpm exec vitest run --dir tests --maxWorkers=2 --minWorkers=1` — 82 文件 514 PASS
- 合计 601 PASS
- 插曲：`pnpm test:unit` 误扫入既有 `.claude/worktrees` 与未跟踪参考目录，已停止；
  初次并行 scoped 测试出现一例既有 SettingsDialog 默认语言 waitFor 失败，隔离 `--dir tests`
  5 PASS 后 2 workers 全量 514 PASS，未动无关 settings 生产代码

## 未执行验证与剩余风险

- Rust 新增测试本地未执行：Windows 宿主无 Rust/cargo；临时下载的官方 rustfmt 已校验
  checksum，但执行被拒，用户明确选择 CI-only（不运行该二进制）
- 用户 macOS 真机关闭/重拉/账号测试未做
- 保留既有事务及登录态结构回读校验；本次尚未执行 Rust 测试或取得 live 登录证明。
  新增 PID 稳定存活检查不等于窗口可交互或客户端已接受账号。

## Git 状态

- 分支：`release/windsurf-v3.19.4`
- 归档创建时：尚未提交/推送，Rust 格式、Clippy、测试等待本提交触发的 CI；最终状态以 Git 历史及对应 Actions run 为准。
- 用户已授权推送：`https://github.com/chunzhimoe/cc-switch.git`
  `HEAD:refs/heads/release/windsurf-v3.19.4`（非 main、不改 remotes、不用 force）

## 备注

- 参照实现为 cockpit-tools GUI，非 legacy core；归档不含任何凭据值。
- `comm` 的语义按 macOS 而非 Linux 核验：Apple 的
  [keyword.c](https://github.com/apple-oss-distributions/adv_cmds/blob/main/ps/keyword.c)
  将 `comm` 映射到 `just_command`；
  [print.c](https://github.com/apple-oss-distributions/adv_cmds/blob/main/ps/print.c)
  使用无参数的 argv[0]，正常保留调用路径及内嵌空格，`-ww` 防止显示宽度截断。
  这不替代用户机器上实际进程输出与切号效果的验收。
