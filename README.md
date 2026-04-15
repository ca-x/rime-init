# rime-init

Rime 输入法初始化与更新工具，Rust 重写的 [rime-wanxiang-updater](https://github.com/ca-x/rime-wanxiang-updater)，支持 **万象**、**雾凇**、**白霜** 三大方案。

## 特性

- 🔄 **一键更新**: 方案、词库、模型一键检查并更新
- 🎨 **TUI 界面**: ratatui 终端界面，键盘操作
- 🌐 **多方案**: 万象 (10 变体) + 雾凇 + 白霜
- 🧠 **模型 Patch**: 自动下载并启用万象语法模型
- 🎭 **皮肤 Patch**: 内置 6 个主题一键切换
- 🌍 **中英双语**: `--lang en` / `--lang zh`
- 🪞 **CNB 镜像**: 国内加速下载
- 🔐 **SHA256 校验**: 确保文件完整性
- 💾 **断点续传**: 节省流量
- 🔌 **代理支持**: SOCKS5 / HTTP
- ⚡ **跨平台**: Windows / macOS / Linux

## 安装

### 从源码编译

```bash
git clone https://github.com/ca-x/rime-init.git
cd rime-init
cargo build --release
# 二进制在 target/release/rime-init
```

### Arch Linux (AUR)

```bash
# 待发布
yay -S rime-init
```

## 使用

### TUI 模式 (默认)

```bash
rime-init
```

启动交互式终端界面，使用 `↑↓/jk` 导航，`Enter` 确认，`q/Esc` 退出。

### 首次初始化

```bash
rime-init --init
```

引导选择方案、词库，自动下载并部署。

### 命令行模式

```bash
# 一键更新所有
rime-init --update

# 仅更新方案
rime-init --scheme

# 仅更新词库
rime-init --dict

# 仅更新模型
rime-init --model

# 更新模型并启用 patch
rime-init --model --patch-model
```

### 其他选项

```bash
# 使用 CNB 镜像 (国内加速)
rime-init --update --mirror

# 设置代理
rime-init --update --proxy socks5://127.0.0.1:1080

# 英文界面
rime-init --lang en --update
```

## 支持的方案

| 方案 | 仓库 | 说明 |
|------|------|------|
| 万象拼音 (标准版) | [amzxyz/rime_wanxiang](https://github.com/amzxyz/rime_wanxiang) | 全拼、双拼 |
| 万象拼音 Pro (墨奇/小鹤/自然码/虎码/五笔/汉心/首右) | 同上 | 双拼 + 辅助码 |
| 雾凇拼音 | [iDvel/rime-ice](https://github.com/iDvel/rime-ice) | 16.6k ⭐ |
| 白霜拼音 | [gaboolic/rime-frost](https://github.com/gaboolic/rime-frost) | 3.1k ⭐ |

### 语法模型 (仅万象)

从 [amzxyz/RIME-LMDG](https://github.com/amzxyz/RIME-LMDG) 下载 `wanxiang-lts-zh-hans.gram`，自动 patch 到方案配置。

## TUI 菜单

```
╔══════════════════════════════════════╗
║  rime-init v0.1.0  万象拼音 (标准版)  ║
╚══════════════════════════════════════╝

  1. 一键更新
  2. 更新方案
  3. 更新词库
  4. 更新模型
  5. 模型 Patch
  6. 皮肤 Patch
  7. 切换方案
  8. 配置
  Q. 退出
```

## 内置皮肤

- 简纯 (amzxyz)
- Win11 浅色 / 暗色
- 微信
- Mac 白
- 灵梦

皮肤写入 `weasel.custom.yaml` (Windows) 或 `squirrel.custom.yaml` (macOS)。

## 配置

配置文件位置:

- **Linux**: `~/.config/rime-init/config.json`
- **macOS**: `~/Library/Application Support/rime-init/config.json`
- **Windows**: `%APPDATA%\rime-init\config.json`

```json
{
  "schema": "WanxiangBase",
  "use_mirror": false,
  "github_token": "",
  "proxy_enabled": false,
  "proxy_type": "socks5",
  "proxy_address": "127.0.0.1:1080",
  "exclude_files": [".DS_Store", ".git"],
  "auto_update": false,
  "language": "zh",
  "fcitx_compat": false,
  "model_patch_enabled": false,
  "skin_patch_key": ""
}
```

## 架构

```
src/
├── main.rs           CLI 入口
├── types.rs          核心类型 (Schema, Config, UpdateInfo)
├── config.rs         配置管理 + 平台路径检测
├── i18n.rs           国际化 (中/英)
├── api/mod.rs        GitHub / CNB API 客户端
├── fileutil/         SHA256, ZIP 解压
├── updater/          方案/词库/模型更新器 + model patch
├── deployer/         跨平台部署 + Fcitx 同步 + hooks
├── skin/             内置主题 + YAML patch
└── ui/               ratatui TUI + 初始化向导
```

## 开发

```bash
# 开发构建
cargo build

# Release 构建
cargo build --release

# 运行
cargo run
cargo run -- --init
cargo run -- --update --mirror
```

## 贡献

欢迎提交 Issue 和 PR！

## 许可证

MIT

## 致谢

- [rime-wanxiang-updater](https://github.com/ca-x/rime-wanxiang-updater) - Go 原版
- [rime_wanxiang](https://github.com/amzxyz/rime_wanxiang) - 万象拼音方案
- [rime-ice](https://github.com/iDvel/rime-ice) - 雾凇拼音方案
- [rime-frost](https://github.com/gaboolic/rime-frost) - 白霜拼音方案
- [RIME-LMDG](https://github.com/amzxyz/RIME-LMDG) - 语法模型
- [ratatui](https://github.com/ratatui/ratatui) - TUI 框架
- [reqwest](https://github.com/seanmonstar/reqwest) - HTTP 客户端
