# QuotaPlusPlus

QuotaPlusPlus 是一个面向 Codex 的极简桌面配置工具，运行命令为 `qpp`。

主窗口只有两个入口：

- 官方登录：启动 Codex 官方设备码登录；
- API 配置：填写 Responses API 的 API URL 和 API Key。

API 配置会写入独立的 `qpp` 提供方：

```toml
model_provider = "qpp"

[model_providers.qpp]
name = "QuotaPlusPlus"
base_url = "https://api.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
experimental_bearer_token = "API_KEY"
```

程序不会读取或修改 Codex 登录凭据。现有 `config.toml` 会在写入前备份到 `~/.codex/qpp-backups/`，其他配置和提供方保持不变。

## 一行安装

发布到 GitHub 后，将命令中的 `OWNER` 换成仓库所属账号：

```powershell
irm https://github.com/OWNER/QuotaPlusPlus/releases/latest/download/install.ps1 | iex
```

安装脚本下载最新的 Windows x64 成品到当前用户的 `%LOCALAPPDATA%\QuotaPlusPlus\bin`，把该目录加入用户 `PATH`，然后启动 QuotaPlusPlus。不需要管理员权限；以后重复执行同一条命令就是升级。

卸载命令：

```powershell
irm https://github.com/OWNER/QuotaPlusPlus/releases/latest/download/uninstall.ps1 | iex
```

卸载只删除程序和对应的 `PATH` 条目，保留 Codex 登录、配置、对话和备份。

不希望直接执行网络脚本时，可以先下载并检查：

```powershell
irm https://github.com/OWNER/QuotaPlusPlus/releases/latest/download/install.ps1 -OutFile install.ps1
notepad install.ps1
.\install.ps1
```

## 构建

```text
cargo build --release
```

产物名为 `qpp`，Windows 上是 `qpp.exe`。
