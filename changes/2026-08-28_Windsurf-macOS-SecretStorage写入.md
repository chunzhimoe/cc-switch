# 2026-08-28 Windsurf macOS SecretStorage 写入

## 背景 / 根因

- `src-tauri/src/windsurf/auth_write.rs` 原先只有 Windows SecretStorage 写入实现；macOS 会进入不支持分支，因此切换 Windsurf/Devin 账号时无法写入 `state.vscdb` 中的两个加密 secret。
- Electron/Chromium 在 macOS 上使用 Keychain Safe Storage 密钥，经 PBKDF2-HMAC-SHA1 派生 AES-128-CBC key；其格式与 Windows 的 DPAPI + AES-256-GCM 不同，必须使用平台专属实现。
- 最终代码已按互操作格式独立重写，不复用 `cockpit-tools` 的函数结构或控制流；早期草稿中“直接复刻”的表述已作废，未将该目录或其许可证内容纳入发布提交。

## 修改文件与核心符号

### `src-tauri/Cargo.toml` / `src-tauri/Cargo.lock`

macOS target 新增以下直接依赖，并同步锁文件：

- `aes = "0.8"`
- `cbc = "0.1"`（锁定 `0.1.2`）
- `pbkdf2 = "0.12"`
- `sha1 = "0.10"`
- `zeroize = "1"`

这些依赖只在 `target_os = "macos"` 下参与编译，不改变 Windows/Linux 依赖路径。

### `src-tauri/src/windsurf/auth_write.rs`

新增 macOS 专用实现：

- `MacosKeychainQuery`、`DEVIN_KEYCHAIN_QUERIES`、`WINDSURF_KEYCHAIN_QUERIES`
  - 兼容 `Devin Safe Storage` 与 `Windsurf Safe Storage` 的历史 service/account 组合。
  - `macos_profile_prefers_windsurf` 根据当前 profile 目录名确定优先品牌，随后仍遍历另一组候选。
- `read_macos_keychain_secret`
  - 通过 `/usr/bin/security find-generic-password -w -s ... [-a ...]` 在子进程内捕获 Keychain 输出。
  - 不在日志、错误或测试输出中打印 Safe Storage 密钥。
- `strip_macos_command_line_ending`
  - 只移除 `security` 命令追加的一层 `\r\n`、`\r` 或 `\n`。
  - 不再使用 `.trim()`，避免合法的首尾空格被删除后派生出错误密钥。
- `encrypt_macos_secret`
  - Salt：`saltysalt`
  - 迭代次数：`1003`
  - PBKDF2：HMAC-SHA1，输出 16-byte key
  - 加密：AES-128-CBC + PKCS#7
  - IV：16 个 ASCII 空格
  - 输出前缀：`v10`
  - key、明文缓冲区与读取到的 Keychain 字符串使用 `zeroize::Zeroizing` 清理内存。
- `encrypt_secret_payload`
  - Windows 分支保持原 DPAPI + AES-256-GCM 行为。
  - macOS 分支读取 Keychain 后生成 Chromium/Electron 兼容 `v10` 密文。
  - 已存在 `v11` secret 时明确失败，避免以未知格式静默覆盖。
  - Linux/其他平台继续返回 `windsurf.secret_storage_platform_pending`。

### 既有事务边界

`src-tauri/src/windsurf/inject.rs` 已在调用 `write_windsurf_auth_data` 前执行 `BEGIN IMMEDIATE`，写入失败时 `ROLLBACK`，成功时 `COMMIT`。因此 sessions、API server、auth status、extension state、onboarding 与 login/usage 条目仍作为一个原子事务提交，本次不再嵌套创建第二个事务。

## 行为差异

### 修改前

- Windows 可以写入 Windsurf SecretStorage。
- macOS 直接返回平台不支持，账号切换无法完成。

### 修改后

- Windows 行为不变。
- macOS 可读取当前 Devin/Windsurf Safe Storage 密钥，生成 `v10` AES-128-CBC 密文，并写入：
  - `windsurf_auth.sessions`
  - `windsurf_auth.apiServerUrl`
- 双品牌安装按 profile 品牌优先查询，仍保留另一品牌作为兼容回退。
- Keychain 密钥中的首尾空格会被保留；只剥离命令行结束符。
- Linux 仍不支持 SecretStorage 写入。

## 测试与验证

### 已完成

- 新增 macOS 单元测试：
  - `strips_only_security_command_line_ending`
  - `orders_keychain_queries_by_profile_brand`
  - `encrypts_with_chromium_macos_secret_storage_format`
- 固定向量：`plaintext = "hello"`、`password = "test-password"`，预期输出为 `v10` 加 16-byte CBC 密文；向量同时使用 Node.js `crypto` 独立计算核对。
- 前端完整测试：`vitest run --dir tests`，81 个测试文件、509 个用例通过。
- `pnpm typecheck`、`pnpm format:check`、renderer production build、`pnpm install --frozen-lockfile` 均通过。
- `git diff --check` 通过。

### 尚未完成

- 本机未安装 Cargo/rustfmt，Rust 编译、Clippy、格式检查与新增 Rust 单元测试需由 GitHub Actions CI 验证。
- 尚未在 macOS 真机读取实际 Keychain 或执行切号；验证过程中不得把真实 Safe Storage 密钥打印到终端、日志或 CI 输出。

## 已知风险与后续事项

- 首次读取 Keychain 可能触发 macOS 授权对话框，用户必须允许访问对应 Safe Storage 条目。
- 自定义 user-data 目录名若不体现品牌，会默认优先 Devin 候选，但仍会继续尝试 Windsurf 候选。
- macOS `v11` 写入格式尚未实现；遇到现有 `v11` 数据会安全失败。
- 需要在 macOS 真机验证 Devin、Windsurf、双装及自定义目录四类场景，并确认切换、重启和回切均能正常解密。

## Git 状态

- 分支：`release/windsurf-v3.19.4`
- 目标应用版本：`3.19.5`
- 目标标签：`v3.19.5-windsurf.1`
- 归档生成时提交、推送与标签均待执行；最终结果以 Git 历史和 GitHub Actions 为准。
