<p align="center">
  <a href="https://github.com/openrect/dsh-community-installer"><img src="assets/dsh-modern-mark.svg" width="96" alt="DSH Community Installer" /></a>
</p>

<h1 align="center">DSH Community Installer - Unofficial</h1>

<p align="center">A clean, lightweight way to install and keep upstream DSH current on Windows.</p>

<p align="center">
  <a href="https://github.com/openrect/dsh-community-installer/releases"><img src="https://img.shields.io/github/v/release/openrect/dsh-community-installer?style=flat-square&label=release&color=4D6BFE" alt="Release" /></a>
  <a href="https://github.com/openrect/dsh-community-installer/releases"><img src="https://img.shields.io/github/downloads/openrect/dsh-community-installer/total?style=flat-square&label=downloads&color=4D6BFE" alt="Downloads" /></a>
  <a href="https://github.com/openrect/dsh-community-installer/stargazers"><img src="https://img.shields.io/github/stars/openrect/dsh-community-installer?style=flat-square&label=stars&color=4D6BFE" alt="Stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-4D6BFE?style=flat-square" alt="MIT License" /></a>
  <img src="https://img.shields.io/badge/Windows%20%7C%20macOS%20coming%20soon-black?style=flat-square" alt="Windows | macOS coming soon" />
</p>

<p align="center"><samp><strong>English</strong> · <a href="README.zh-CN.md">中文</a></samp></p>

<p align="center"><img src="assets/readme-hero.svg" width="1100" alt="DSH Community Installer for Windows" /></p>

<p align="center"><strong>≈ 3.8 MB online setup · Private Node.js runtime · Verified DSH packages · Built-in updates</strong></p>

## Why this installer

| | |
| --- | --- |
| **Clean by default** | Installs upstream DSH without API keys or third-party plugins, and leaves the system Node.js installation and `%USERPROFILE%\.dsh` untouched. |
| **Small online setup** | The Windows installer is about 3.8 MB. It uses the system WebView2 runtime and downloads only the pinned runtime components during first setup. |
| **Self-contained runtime** | DSH runs with a private, verified Node.js `24.19.0` environment under `%LOCALAPPDATA%\DSHCommunityInstaller`. |
| **Controlled updates** | The tray checks both the installer and DSH. Updates download only after confirmation and switch versions only after validation. |

## Download

| Platform | Edition | Approx. size | Download |
| --- | --- | ---: | --- |
| Windows 10/11 x64 | Online — recommended | 3.8 MB | [DSH-Community-Setup-0.4.6-Windows-x64.exe](https://github.com/openrect/dsh-community-installer/releases/download/v0.4.6/DSH-Community-Setup-0.4.6-Windows-x64.exe) |
| Windows 10/11 x64 | Offline | 95 MB | [DSH-Community-Offline-Setup-0.4.6-Windows-x64.exe](https://github.com/openrect/dsh-community-installer/releases/download/v0.4.6/DSH-Community-Offline-Setup-0.4.6-Windows-x64.exe) |
| macOS | Online | — | Coming soon |
| macOS | Offline | — | Coming soon |

Choose the online edition for normal use. It installs DSH with a private Node.js runtime and pinned pnpm through Corepack. The offline edition carries the same prepared runtime for machines that cannot reach the Node.js distribution or npm registry. Both editions require Microsoft Edge WebView2, which is normally present on Windows 10/11. Sizes are rounded and may change slightly between releases.

## How it works

1. The online setup installs the newest compatible official DSH release once. The offline setup imports its pinned runtime seed.
2. The lightweight Tauri tray starts Harness only on `127.0.0.1:3080`, captures logs, and opens the upstream interface in the default browser.
3. Update checks distinguish installer releases from DSH releases. After confirmation, DSH is installed in a fresh directory, validated, and atomically activated with rollback on failure.

The tray also provides English/Chinese switching, logs, manual update checks, and a single clear exit action. Uninstall removes the private runtime and logs while preserving upstream DSH settings and sessions.

## Project status

> [!IMPORTANT]
> This is an unofficial community project. It is not affiliated with, authorized by, or endorsed by DeepSeek. `DSH`, `DeepSeek Harness`, and `@deepseek-ai/dsh` identify the compatible upstream software.

> [!WARNING]
> Windows builds are currently unsigned. SmartScreen may show an unknown-publisher warning. Verify the SHA-256 supplied with the GitHub Release; do not disable SmartScreen or security software.

## Build

Install Node.js, pnpm, the Rust MSVC toolchain, and the Windows build tools, then run:

```powershell
pnpm install --frozen-lockfile
pnpm build:online
pnpm build:offline
```

`pnpm test` runs the frontend and Rust tests plus release consistency checks. `pnpm release` prepares both installers, SHA-256 files, and signed Tauri updater artifacts.

## License

[MIT](LICENSE). Upstream components retain their own licenses; see [third-party notices](THIRD_PARTY_NOTICES.txt) and [security policy](SECURITY.md).
