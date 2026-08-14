# QuotaPlusPlus

QuotaPlusPlus 是一个面向 Codex 的极简桌面配置工具，运行命令为 `qpp`。

主窗口只有两个入口：

- 官方登录：启动 Codex 官方设备码登录；
- API 配置：填写 Responses API 的 API URL 和 API Key。

API 配置会写入独立的 `custom` 提供方：

```toml
model_provider = "custom"

[model_providers.custom]
name = "QuotaPlusPlus"
base_url = "https://api.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
experimental_bearer_token = "API_KEY"
```

程序不会读取或修改 Codex 登录凭据。保存 API 配置时，程序会扫描普通会话、归档会话和 Codex SQLite 状态数据库，把其中所有任务的提供方统一为 `custom`。修改前会把配置、rollout 元数据和 SQLite 备份到 `~/.codex/qpp-backups/`；其他配置和提供方定义保持不变，失败时自动恢复。

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

Apple 芯片 Mac 下载 `qpp-macos-arm64.dmg`，Intel Mac 下载 `qpp-macos-x64.dmg`，打开后把 QuotaPlusPlus 拖入应用程序目录。

当前发布包没有使用 Apple 开发者证书签名。首次启动时，需要在访达中右键点击 QuotaPlusPlus 并选择打开。

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
