<p align="center">
  <a href="https://github.com/openrect/dsh-community-installer"><img src="assets/dsh-modern-mark.svg" width="96" alt="DSH Community Installer" /></a>
</p>

<h1 align="center">DSH Community Installer - Unofficial</h1>

<p align="center">在 Windows 上干净、轻量地安装并持续更新上游 DSH。</p>

<p align="center">
  <a href="https://github.com/openrect/dsh-community-installer/releases"><img src="https://img.shields.io/github/v/release/openrect/dsh-community-installer?style=flat-square&label=release&color=4D6BFE" alt="Release" /></a>
  <a href="https://github.com/openrect/dsh-community-installer/releases"><img src="https://img.shields.io/github/downloads/openrect/dsh-community-installer/total?style=flat-square&label=downloads&color=4D6BFE" alt="Downloads" /></a>
  <a href="https://github.com/openrect/dsh-community-installer/stargazers"><img src="https://img.shields.io/github/stars/openrect/dsh-community-installer?style=flat-square&label=stars&color=4D6BFE" alt="Stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-4D6BFE?style=flat-square" alt="MIT License" /></a>
  <img src="https://img.shields.io/badge/Windows%20%7C%20macOS%20coming%20soon-black?style=flat-square" alt="Windows | macOS coming soon" />
</p>

<p align="center"><samp><a href="README.md">English</a> · <strong>中文</strong></samp></p>

<p align="center"><img src="assets/readme-hero.svg" width="1100" alt="DSH Community Installer Windows 安装器" /></p>

<p align="center"><strong>联网安装包约 3.8 MB · 独立 Node.js 环境 · 校验 DSH 组件 · 内置更新</strong></p>

## 为什么使用它

| | |
| --- | --- |
| **默认干净** | 只安装上游 DSH，不附带 API Key 或第三方插件，也不修改系统 Node.js 和 `%USERPROFILE%\.dsh`。 |
| **联网包轻量** | Windows 联网安装包约 3.8 MB，复用系统 WebView2，首次安装时只下载固定版本的运行组件。 |
| **运行环境独立** | DSH 使用位于 `%LOCALAPPDATA%\DSHCommunityInstaller` 的独立 Node.js `24.19.0` 环境。 |
| **更新可控** | 托盘分别检查安装器和 DSH；安装器经确认后才下载，DSH 更新会先暂存并验证再切换。 |

## 下载

| 平台 | 版本 | 约占体积 | 下载 |
| --- | --- | ---: | --- |
| Windows 10/11 x64 | 联网版（推荐） | 3.8 MB | [DSH-Community-Setup-0.4.3-Windows-x64.exe](https://github.com/openrect/dsh-community-installer/releases/download/v0.4.3/DSH-Community-Setup-0.4.3-Windows-x64.exe) |
| Windows 10/11 x64 | 离线版 | 95 MB | [DSH-Community-Offline-Setup-0.4.3-Windows-x64.exe](https://github.com/openrect/dsh-community-installer/releases/download/v0.4.3/DSH-Community-Offline-Setup-0.4.3-Windows-x64.exe) |
| macOS | 联网版 | — | 开发中 |
| macOS | 离线版 | — | 开发中 |

日常使用请选择联网版。离线版包含相同的独立运行环境，适合无法访问 Node.js 或 npm 的电脑。两个版本都需要 Microsoft Edge WebView2；Windows 10/11 通常已经安装。表中体积经过取整，不同版本可能略有变化。

## 工作方式

1. 安装程序校验下载内容后，安装固定版本的 Node.js 运行环境和 `@deepseek-ai/dsh@0.1.0-rc.7`。
2. 轻量 Tauri 托盘只在 `127.0.0.1:3080` 启动 Harness、收集日志，并在默认浏览器中打开上游界面。
3. 更新检查会区分安装器与 DSH。DSH 更新会先禁用安装脚本完成下载，再通过固定脚本策略与运行验证后切换。

托盘还提供中英文切换、日志、手动检查更新和一个明确的退出入口。卸载会删除独立运行环境与日志，保留上游 DSH 设置和会话。

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

`pnpm test` 会运行前端测试、视觉规范检查和 Rust 测试。`pnpm release` 会准备两个安装器、SHA-256 文件及带 Tauri 更新签名的产物。

## 许可证

[MIT](LICENSE)。上游组件保留各自许可证，详见[第三方声明](THIRD_PARTY_NOTICES.txt)和[安全说明](SECURITY.md)。
