<p align="center">
  <a href="https://github.com/openrect/dsh-community-installer"><img src="assets/dsh-modern-mark.svg" width="96" alt="DSH Community Installer" /></a>
</p>

<h1 align="center">DSH Community Installer - Unofficial</h1>

<p align="center">面向 DeepSeek Harness（DSH）的干净 Windows 安装器与更新助手。DSH 是开源、插件化的 Agent Harness。</p>

<p align="center">
  <a href="https://github.com/openrect/dsh-community-installer/releases"><img src="https://img.shields.io/github/v/release/openrect/dsh-community-installer?style=flat-square&label=release&color=4D6BFE" alt="Release" /></a>
  <a href="https://github.com/openrect/dsh-community-installer/releases"><img src="https://img.shields.io/github/downloads/openrect/dsh-community-installer/total?style=flat-square&label=downloads&color=4D6BFE" alt="Downloads" /></a>
  <a href="https://github.com/openrect/dsh-community-installer/stargazers"><img src="https://img.shields.io/github/stars/openrect/dsh-community-installer?style=flat-square&label=stars&color=4D6BFE" alt="Stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-4D6BFE?style=flat-square" alt="MIT License" /></a>
  <img src="https://img.shields.io/badge/Windows%20%7C%20macOS%20coming%20soon-black?style=flat-square" alt="Windows | macOS coming soon" />
</p>

<p align="center"><samp><a href="README.md">English</a> · <strong>中文</strong></samp></p>

<p align="center"><img src="assets/readme-hero.svg" width="1100" alt="DSH Community Installer Windows 安装器" /></p>

<p align="center"><strong>联网安装包约 3.8 MB · 独立 Node.js 环境 · 官方 DSH 软件包 · 自动检查更新</strong></p>

## 让 DeepSeek Harness 在 Windows 上开箱即用

[DeepSeek Harness](https://deepseek.com/harness/) (`dsh`) 是 DeepSeek AI 开源的插件化 Agent Harness，用于构建和运行 Agent。模型、工具、技能、会话、沙箱、存储、循环、调度和 UI 等能力都由插件组合而成。

官方快速启动命令是 `npx @deepseek-ai/dsh web`。本项目将同一个官方 npm 软件包安装到 Windows 独立运行环境，通过轻量托盘管理本地服务，并在默认浏览器中打开上游 Web UI。它不修改系统 Node.js，也不改动上游 DSH 用户数据。

<p align="center">
  <a href="https://deepseek.com/harness/"><img src="https://deepseek.com/harness/images/harness/feat-plugin.png" width="900" alt="DeepSeek Harness Web UI 的插件管理界面" /></a>
</p>
<p align="center"><sub>上游 DeepSeek Harness Web UI · 图片来自 DeepSeek Harness 官方网站</sub></p>

## 为什么使用它

| | |
| --- | --- |
| **默认干净** | 只安装上游 DSH，不附带 API Key 或第三方插件，也不修改系统 Node.js 和 `%USERPROFILE%\.dsh`。 |
| **联网包轻量** | Windows 联网安装包约 3.8 MB，复用系统 WebView2，首次安装时直接下载当前兼容的官方 DSH。 |
| **运行环境独立** | DSH 使用位于 `%LOCALAPPDATA%\DSHCommunityInstaller` 的独立 Node.js `24.19.0` 环境。 |
| **无需重装即可更新** | 默认自动检查更新，发现新版后先询问用户，确认后才下载；新版本会单独验证，不会先覆盖当前可用版本。 |

## 下载

| 平台 | 版本 | 约占体积 | 下载 |
| --- | --- | ---: | --- |
| Windows 10/11 x64 | 联网版（推荐） | 3.8 MB | [DSH-Community-Setup-0.4.6-Windows-x64.exe](https://github.com/openrect/dsh-community-installer/releases/download/v0.4.6/DSH-Community-Setup-0.4.6-Windows-x64.exe) |
| Windows 10/11 x64 | 离线版 | 95 MB | [DSH-Community-Offline-Setup-0.4.6-Windows-x64.exe](https://github.com/openrect/dsh-community-installer/releases/download/v0.4.6/DSH-Community-Offline-Setup-0.4.6-Windows-x64.exe) |
| macOS | 联网版 | — | 开发中 |
| macOS | 离线版 | — | 开发中 |

日常使用请选择联网版。它通过 Corepack 和固定版 pnpm，把 DSH 安装到独立 Node.js 运行环境中。离线版包含相同的预置运行环境，适合无法访问 Node.js 下载源或 npm 官方仓库的电脑。两个版本都需要 Microsoft Edge WebView2；Windows 10/11 通常已经安装。表中体积经过取整，不同版本可能略有变化。

## 工作方式

1. 联网版从 npm 安装当前兼容的最新官方 DSH；离线版导入构建时固定的运行环境种子，后续联网后也可更新。
2. 轻量 Tauri 托盘只在 `127.0.0.1:3080` 启动 Harness、收集日志，并在默认浏览器中打开上游界面。
3. 更新检查会区分安装器与 DSH。用户确认后，DSH 会安装到新目录，验证通过后原子切换，失败则恢复旧版。

托盘还提供中英文切换、日志、手动检查更新和一个明确的退出入口。卸载会删除独立运行环境与日志，保留上游 DSH 设置和会话。

## 自动检查，由你决定是否更新

- **自动检查：**默认开启，Harness 启动后自动检查；也可随时在托盘中手动检查。
- **持续跟进 DSH：**同时检查官方 npm `latest` 和 `next` 通道，选择与内置 Node.js 兼容的较新版本。
- **不静默安装：**发现新版后先显示提示，只有在用户确认后才开始下载。
- **安全切换：**新 DSH 使用独立 pnpm 安装，界面显示实时进度；验证通过后再原子切换，候选版本失败不会破坏当前版本。
- **安装器单独更新：**Windows 控制程序使用独立的签名更新源，与上游 DSH 更新分开处理。

因此，本项目既是 DeepSeek Harness 的 Windows 安装器，也是一个小型 DSH 更新管理器；上游日常发布新版本时，无需重新下载完整安装包。

## 项目状态

> [!IMPORTANT]
> 本项目是非官方社区项目，与 DeepSeek 不存在隶属、授权或背书关系。`DSH`、`DeepSeek Harness` 和 `@deepseek-ai/dsh` 仅用于说明兼容的上游软件。

> [!WARNING]
> 当前 Windows 构建没有代码签名，SmartScreen 可能显示“未知发布者”。请使用 GitHub Release 提供的 SHA-256 校验文件，不要关闭 SmartScreen 或安全软件。

## 构建

安装 Node.js、pnpm、Rust MSVC 工具链和 Windows 构建工具后运行：

```powershell
pnpm install --frozen-lockfile
pnpm build:online
pnpm build:offline
```

`pnpm test` 会运行前端、Rust 测试和发布一致性检查。`pnpm release` 会准备两个安装器、SHA-256 文件及带 Tauri 更新签名的产物。

## 许可证

[MIT](LICENSE)。上游组件保留各自许可证，详见[第三方声明](THIRD_PARTY_NOTICES.txt)和[安全说明](SECURITY.md)。
