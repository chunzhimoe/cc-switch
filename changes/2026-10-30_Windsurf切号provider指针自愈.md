# 2026-10-30 Windsurf 切号 provider 指针自愈

## 修改背景

macOS 上 Windsurf 切换账号报错 `无效输入: Windsurf provider no longer exists`。此前版本可正常切换。

## 问题根因

`prepare_switch`（`src-tauri/src/windsurf/switch.rs`，`75b03c06` 引入的准备闸门）第一步用 `get_provider_by_id(provider_id, "windsurf")` 查 `providers` 表，查不到即报错。

账号列表读的是 `~/.cc-switch/windsurf_accounts*.json` 文件（`windsurf/account.rs`），而 provider 指针行在 `cc-switch.db` 的 `providers` 表里——两份存储天然会发散：

- WebDAV/S3 同步下载或手动 SQL 导入会用旧快照覆盖整个 `providers` 表（账号文件不参与同步，因此保留）。
- 通用供应商管理界面允许删除非当前的 windsurf provider 行（`ProviderService::delete`）。
- 账号文件被手动拷贝进 `~/.cc-switch/`（迁移、备份恢复）。
- 账号创建时 `upsert_account` 成功但随后的 `save_provider_pointer` 失败（命令返回错误，文件已落盘）。

对照参考实现 `cockpit-tools/src-tauri/src/modules/windsurf_instance.rs` + `commands/windsurf.rs::inject_windsurf_to_vscode`：其切号只依赖账号文件，无 DB provider 间接层，索引还能从明细文件自愈。cc-switch 的 provider 指针是它自己叠加的簿记层，不应成为单点硬前置。

## 修改文件和核心符号

- `src-tauri/src/windsurf/account.rs`：新增 `provider_pointer(account) -> Provider`（抽出原 `save_provider_pointer` 的构造逻辑，`pub(crate)`）。
- `src-tauri/src/windsurf/switch.rs`：新增 `heal_provider_pointer`——provider 行缺失时从账号文件重建指针（校验文件内 `id == provider_id`）；`prepare_switch` 改用它兜底。新增回归测试 `missing_provider_pointer_is_rebuilt_from_the_account_file`。
- `src-tauri/src/commands/windsurf.rs`：`save_provider_pointer` 复用 `account::provider_pointer`；移除不再需要的 `serde_json::json` / `Provider` import。

## 修改前后行为差异

- 修改前：provider 行缺失 → `InvalidInput("Windsurf provider no longer exists")`，账号永久卡死只能删除重加。
- 修改后：provider 行缺失但账号文件存在 → 自动重建指针行（含 `accountId`/`tokenType`/`email`/`maskedToken`、category=`windsurf-account`、icon=`windsurf`）并继续切号；账号文件也缺失 → 报 `Windsurf account no longer exists`。
- `validate_target` 在自愈后能正常读到指针行，`commit` 的 `set_current_provider` 路径不变。

## 执行的测试及结果

- 新增 `switch.rs` 测试：缺行自愈重建 + 账号文件缺失时报错。本机 Windows 无 cargo/rustfmt，编译与测试未能在本地执行，依赖 GitHub Actions CI（macOS/Ubuntu/Windows fmt、clippy、cargo test）验证。

## 已知风险和后续事项

- 自愈会写入一行 provider 簿记数据（`save_provider` 是 UPSERT，幂等）；若账号文件内部 `id` 与查询 id 不一致，直接报错不落地。
- WebDAV 同步不覆盖 `windsurf_accounts*.json` 的设计保持不变；跨设备场景下账号文件缺失仍无法切号（属既有边界，非本次引入）。

## Git 状态

- 分支：`release/windsurf-v3.19.4`
