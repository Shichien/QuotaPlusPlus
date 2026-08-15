# QuotaPlusPlus

QuotaPlusPlus 是一个面向 Codex 的极简桌面配置工具，运行命令为 `qpp`。

主窗口只有两个入口：

- 官方登录：切换到已经保存的 ChatGPT 官方登录；登录缺失或失效时，由 QuotaPlusPlus 打开浏览器完成官方 OAuth；
- API 配置：填写 Responses API 的 API URL 和 API Key。

QuotaPlusPlus 不会运行 `codex login`。如果当前官方登录有效，程序会通过刷新令牌测活并保存最新令牌；如果刷新令牌已失效，点击官方登录后才会打开浏览器。OAuth 使用 PKCE、本地一次性 `state` 校验以及 `localhost:1455` 或 `localhost:1457` 回调。登录等待期间，同一个入口会显示取消登录；浏览器被关闭或不再继续登录时，点击它即可立即返回，不会改动当前配置和对话数据。

API 配置会写入独立的 `custom` 提供方：

```toml
model_provider = "custom"
cli_auth_credentials_store = "file"

[model_providers.custom]
name = "QuotaPlusPlus"
base_url = "https://api.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
```

API Key 不写入 `config.toml`。第三方模式使用 Codex 标准的 `auth.json`：

```json
{
  "OPENAI_API_KEY": "API_KEY"
}
```

第三方模式会把 `cli_auth_credentials_store` 设为 `file`，确保 macOS 也把活动 API Key 保存在 `~/.codex/auth.json`，而不是转存到系统钥匙串。QPP 能读取 Codex 的文件、系统钥匙串和加密认证存储；接管官方会话后也会把官方凭据存储统一为 `file`，保证刷新令牌轮换后恢复的是同一份最新认证。模型、桌面、插件、MCP、通知和其他用户设置保持不变。

第一次从官方切到 API 配置时，程序会把有效的官方认证和当时的 `config.toml` 配对保存到 `~/.codex/qpp-profiles/official/`。第三方配置以当前 `config.toml` 为底稿，只改顶层提供方和 `model_providers.custom`。API Key 填过一次后，同一个 API URL 可以留空继续使用；API URL 变化时必须重新填写对应的 API Key，避免把旧地址的密钥发送给新地址。

保存第三方配置前，QPP 会向对应的 `/responses` 发送不含模型和输入的空请求，检查网络、认证和端点是否可用。这个请求不会触发模型推理；完整的模型、流式响应和工具调用兼容性仍由实际请求决定。

进入第三方模式时，活动目录中的 `auth.json` 会切换为 API Key 格式，普通会话、归档会话和 Codex SQLite 状态数据库中的所有任务提供方会统一为 `custom`。切回官方时，程序会恢复配对的官方 `auth.json` 和 `config.toml`，并把所有任务提供方统一为 `openai`。对话正文、标题、时间和模型字段不会改动。

每次切换前的活动配置、登录文件、rollout 元数据和 SQLite 会备份到 `~/.codex/qpp-backups/`，只保留最新十份完整备份。配置、登录文件、rollout 和 SQLite 属于同一次持久化事务；普通错误会立即恢复，进程被强制结束或设备断电时会在下次启动继续恢复。执行切换前需要退出 Codex，以免 SQLite 正在被占用。

## 下载和安装

发布页会生成以下平台成品：

| 平台 | 成品 |
| --- | --- |
| Windows x64 | `qpp-windows-x64.exe` |
| Apple 芯片 Mac | `qpp-macos-arm64.dmg` |
| Intel Mac | `qpp-macos-x64.dmg` |
| Linux x64 | `qpp-linux-x64.AppImage`、`qpp-linux-x64.deb` |

### Windows

```powershell
irm https://github.com/Shichien/QuotaPlusPlus/releases/latest/download/install.ps1 | iex
```

安装脚本下载最新的 Windows x64 成品到当前用户的 `%LOCALAPPDATA%\QuotaPlusPlus\bin`，把该目录加入用户 `PATH`，然后启动 QuotaPlusPlus。不需要管理员权限；以后重复执行同一条命令就是升级。

卸载命令：

```powershell
irm https://github.com/Shichien/QuotaPlusPlus/releases/latest/download/uninstall.ps1 | iex
```

卸载只删除程序和对应的 `PATH` 条目，保留 Codex 登录、配置、对话和备份。

不希望直接执行网络脚本时，可以先下载并检查：

```powershell
irm https://github.com/Shichien/QuotaPlusPlus/releases/latest/download/install.ps1 -OutFile install.ps1
notepad install.ps1
.\install.ps1
```

### macOS

安装了 Homebrew 的 Mac 可以执行：

```bash
brew tap shichien/qpp https://github.com/Shichien/QuotaPlusPlus && brew install --cask shichien/qpp/qpp
```

Homebrew 会自动选择 Apple 芯片或 Intel 成品，把 QuotaPlusPlus 安装到应用程序目录，并提供 `qpp` 命令。升级和卸载分别使用：

```bash
brew update && brew upgrade --cask shichien/qpp/qpp
brew uninstall --cask shichien/qpp/qpp
```

也可以手动下载 `qpp-macos-arm64.dmg` 或 `qpp-macos-x64.dmg`，打开后把 QuotaPlusPlus 拖入应用程序目录。

当前发布包没有使用 Apple 开发者证书签名。首次启动时，需要在访达中右键点击 QuotaPlusPlus 并选择打开。

遇到 macOS 阻止启动时，可以执行：

```bash
xattr -dr com.apple.quarantine /Applications/QuotaPlusPlus.app
```

### Linux

Debian、Ubuntu 及其衍生系统可以安装 `qpp-linux-x64.deb`。其他常见 x86_64 桌面发行版可以下载 `qpp-linux-x64.AppImage`，赋予执行权限后运行：

```bash
chmod +x qpp-linux-x64.AppImage
./qpp-linux-x64.AppImage
```

## 构建

```bash
npx @tauri-apps/cli@2 build
```

GitHub Actions 会分别在 Windows、Intel Mac、Apple 芯片 Mac 和 Ubuntu 原生运行环境中构建。macOS 生成 DMG，Linux 生成 AppImage 和 DEB，Windows 继续生成现有的一键安装成品。
