# Milestone 1: Go 版功能对齐 + 多方案支持

## 目标
实现 Go 版 rime-wanxiang-updater 全部功能，并扩展支持雾凇和白霜。

## Checklist

### 核心功能 (Go 版已有的)
- [x] Schema 枚举 + 多方案支持 (万象10变体 + 雾凇 + 白霜)
- [x] GitHub Releases API (带 token 认证)
- [x] CNB 镜像 API
- [x] 方案下载 + 解压
- [x] 词库下载 + 解压
- [x] 模型下载 (wanxiang-lts-zh-hans.gram)
- [x] SHA256 校验
- [x] 代理支持 (SOCKS5 / HTTP)
- [x] 跨平台部署 (小狼毫/鼠须管/Fcitx5/IBus)
- [x] 多引擎支持
- [x] Pre/Post update hooks
- [x] Fcitx 兼容目录同步 (软链接/复制)
- [x] 排除文件管理
- [x] 配置管理 (JSON, 平台特定路径)
- [x] 首次初始化向导
- [x] TUI 主菜单 + 键盘导航
- [x] 更新进度显示
- [x] 模型 patch (grammar/language_model)
- [x] 皮肤/主题 patch (weasel/squirrel.custom.yaml)
- [x] 内置主题 (简纯/Win11/微信/Mac/灵梦)
- [x] i18n 中英双语
- [x] CLI 参数 (--init, --update, --scheme, --dict, --model, --mirror, --proxy, --lang)

### 待补全 (Go 版有，Rust 版还缺的)
- [ ] 下载断点续传 (Range header + .tmp 文件)
- [ ] 缓存复用 (SHA256 匹配时跳过下载)
- [ ] 关键文件检测 (wanxiang.lua 不存在强制更新)
- [ ] 版本切换检测 (local_record.name != config.scheme_file)
- [ ] CNB 最新 tag 查询 + 词库 tag 回退逻辑
- [ ] 自动更新倒计时 (auto_update + countdown)
- [ ] 排除文件模板 (按引擎预设排除规则)
- [ ] 多引擎同步 (更新后同步到所有已安装引擎目录)
- [ ] update 结果汇总 (已更新/已跳过/版本号)
- [ ] 部署失败时自动重启服务尝试恢复

### 测试
- [ ] unit test: types, config, fileutil/hash, fileutil/extract
- [ ] unit test: updater record load/save/needs_update
- [ ] unit test: model_patch read/write/patch/unpatch
- [ ] integration test: skin patch 写入 + 读取

### 打包
- [x] README.md
- [x] GitHub Actions CI (check/fmt/clippy/test + cross-compile)
- [ ] Cargo.toml metadata 完善
- [ ] CHANGELOG.md
- [ ] LICENSE 文件

## 实际进度
- 2026-04-15: Phase 1-3 完成 (核心 + 更新器 + TUI)
- 2026-04-15: deployer 重构 (hooks/fcitx/multi-engine), i18n, README, CI
