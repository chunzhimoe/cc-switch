# 修改归档

- [2026-09-05 Windsurf Windows 切号重写](./2026-09-05_Windsurf-Windows切号重写.md) — 对齐 cockpit-tools 重写 Windows 预检、进程关闭、DPAPI/SQLite 注入与回读验证，并同步 Windsurf global_rules.md 和 Devin mcp_config.json。

- [2026-08-28 Windsurf v3.19.5 发布候选](./2026-08-28_Windsurf-v3.19.5发布.md) — 统一升级到 3.19.5，补齐 Windsurf Skills/目录覆盖、macOS SecretStorage 和 schema v18，并准备 v3.19.5-windsurf.1 构建。
- [2026-08-28 Windsurf macOS SecretStorage 写入](./2026-08-28_Windsurf-macOS-SecretStorage写入.md) — 在 `auth_write.rs` 新增 macOS Keychain + PBKDF2/AES-128-CBC 分支，使 macOS 可写入 `state.vscdb` 的 SecretStorage 密文。
- [2026-08-27 供应商级跳过 Auto 分类器](./2026-08-27_供应商跳过Auto分类器.md) — 在每个 Claude 供应商设置中增加高风险开关，按供应商投影 sandbox 与 bypassPermissions，并对称恢复 live 配置。
- [2026-08-27 Windsurf v3.19.4 发布](./2026-08-27_Windsurf-v3.19.4发布.md) — 统一版本元数据并准备 Windsurf prerelease，包含 OAuth、多账号、静默切号和分类器直连修复。
- [2026-08-27 Auto 分类器外部直连启用修复](./2026-08-27_Auto分类器外部直连修复.md) — 通过 Claude Code 的 `CLAUDE_CODE_AUTO_MODE_MODEL` 支持外部 API 直连，并修复本地代理 billing header 漏判与诊断。
- [2026-08-27 Windsurf OAuth 与静默切号](./2026-08-27_Windsurf-OAuth与静默切号.md) — 浏览器 OAuth wait-callback（含手动粘贴回调）与 Windows 静默 taskkill / 同目录自然恢复上次页面。
- [2026-08-26 供应商 Auto 模式分类器分流](./2026-08-26_供应商Auto分类器分流.md) — 在 Claude 供应商高级选项中增加 provider-scoped Auto 安全分类器模型分流，并接入 Claudish marker 检测。
- [2026-08-26 Windsurf 多账号切换](./2026-08-26_Windsurf多账号切换.md) — 接入 Windsurf 本机导入、Token/Auth1、邮箱密码登录与切号重启。
- [2026-08-25 Claude 供应商上下文与自动压缩配置](./2026-08-25_Claude供应商上下文与自动压缩配置.md) — 在编辑供应商中加入三个独立的 400K 上下文/压缩配置，并保证类型与公共配置隔离。
