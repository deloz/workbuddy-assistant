# WorkBuddy助手

WorkBuddy助手是一款面向 Windows 的本地桌面工具，用于集中查看多个 WorkBuddy 账号的额度，并在应用运行期间自动完成每日签到检查。

## 功能

- 内置浏览器登录：点击后在系统浏览器完成 WorkBuddy 登录，应用通过官方插件接口取得登录信息，无需打开 WorkBuddy 桌面端，也不再读取本机加密的认证文件。
- 支持保存多个账号，一次登录会自动列出该账号的个人与企业身份；在多个账号之间切换，按需查看完整账号 ID。
- 启动、每日本地时间 00:00 及休眠跨日后检查全部可用账号的签到状态。
- 关闭窗口后继续在系统托盘运行，并提供打开、立即签到和退出操作。
- 启动时后台检查 GitHub Releases，发现新版本后显示提示；点击可查看 GitHub Release 与 Scoop 更新指导，设置页也可手动检查。

## 安装

推荐使用 [Scoop](https://scoop.sh/)：

```powershell
scoop bucket add workbuddy-assistant https://github.com/deloz/scoop-workbuddy-assistant
scoop install workbuddy-assistant
```

更新或卸载：

```powershell
scoop update workbuddy-assistant
scoop uninstall workbuddy-assistant
```

也可以从 [GitHub Releases](https://github.com/deloz/workbuddy-assistant/releases) 下载便携版或 NSIS 安装器。当前发布未进行商业代码签名，Windows SmartScreen 可能在首次运行时显示提醒。

## 使用

1. 启动 WorkBuddy助手，点击“浏览器登录”。
2. 在打开的系统浏览器页面里正常登录 WorkBuddy，回到应用等待自动完成（最多 5 分钟）。
3. 登录成功后账号与额度会自动加载；需要更多账号时，在设置页点击“添加账号”，或使用顶部账号选择器底部的“＋ 添加账号”。
4. 使用顶部账号选择器切换身份，在“额度概览”和“签到状态”之间查看信息；登录过期时在设置页点击“重新登录”，不再需要的账号可以“移除”。
5. 需要完全退出时，使用系统托盘菜单中的“退出”。

## 隐私

浏览器登录得到的 Access Token、Refresh Token 以明文 JSON 保存在 `%LOCALAPPDATA%\com.workbuddy.assistant\workbuddy-signin.json`（仅当前 Windows 用户可读），用于免登录启动与自动续期；不再读取或修改 WorkBuddy 自己的认证文件。应用只会把这些凭据发给 WorkBuddy 官方接口，不会发送到前端、日志或 GitHub；切换账号时取得的临时会话不写入磁盘，检查更新只访问本项目的公开 Release 接口。

## 开发

需要 Node.js 24、Rust 1.95 或更高版本，以及 Windows MSVC Build Tools。

```powershell
npm.cmd install
npm.cmd run tauri -- dev
```

提交前运行：

```powershell
npm.cmd test
npm.cmd run build
npm.cmd run verify:version
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

构建 Windows 便携程序和 NSIS 安装器：

```powershell
npm.cmd run tauri -- build
```

推送符合 `vX.Y.Z` 格式且与项目版本一致的标签后，GitHub Actions 会根据 Conventional Commits 自动生成分类版本说明、构建附件、创建正式 Release，并通知 Scoop bucket。

## License

[MIT](LICENSE) © 2026 Deloz
