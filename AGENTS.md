# AGENTS.md

GPUI 编写的图形化 `.deb` 安装器（Linux only，Debian 系）。UI 框架为 `gpui-kit 0.6.2`，Rust edition 2024。

## 常用命令

```bash
cargo run --release                 # 启动（可选参数：某个 .deb 路径）
cargo check                         # 快速类型检查
cargo deb                           # 打 .deb 包 → target/debian/*.deb
./install-icon.sh                   # 开发机桌面集成（右键菜单 + 图标）
```

**注意：本项目只能在 Linux 上编译运行**（依赖 dpkg/apt/pkexec 与 Vulkan/Wayland 相关系统库）。在 macOS 上不要尝试 `cargo build`，改动靠 `cargo check` 无法验证时直接说明未编译验证。

## 目录结构

- `src/main.rs` — 入口：解析第 1 个命令行参数作为 deb 路径、加载配置、开窗
- `src/app.rs` — `AppView`，持有三个 tab 视图并负责它们之间的联动（安装完成刷新列表、卸载委托等）
- `src/views/` — `install.rs`（安装/日志）、`packages.rs`（已安装列表）、`files_preview.rs`（包内文件预览）
- `src/utils/` — 所有系统交互：`dpkg.rs`（pkexec + apt 安装/卸载，流式日志）、`apt.rs`（已安装列表）、`deb_reader.rs`/`deb_files.rs`（ar+tar 解析）、`pkg_time.rs`、`app_launch.rs`
- `src/models/` — `config.rs`（`~/.config/gpui-deb-installer/config.json`，窗口尺寸 + 设置）、`package.rs`
- `locales/{en,zh-CN}.yml` — 文案，由 `rust-i18n` 在编译期嵌入

## 编码约定

- **一切 UI 文案走 i18n**：硬编码字符串用 `tr("a.b.c")`，同时在 `locales/en.yml` **和** `locales/zh-CN.yml` 添加同名 key（key 结构保持一致）。新增文件后无需改 `build.rs`，但改了 `locales/` 会触发重编译。
- **系统操作用 `pkexec`**，不要用 `sudo`；安装走 `apt-get install -y --allow-downgrades`（允许降级），日志通过 `async_channel` 流式回传 UI。
- **视图之间不要直接互相调用**，用 `app.rs` 里已有的回调钩子（`on_installed`、`on_tab_switch`、`install_view` 弱引用）来解耦。
- 只用 `gpui_kit` 导出的组件与 API；`cargo fmt` 风格、`use` 分组不强制，跟随所在文件现状。
- 注释与文档用英文，提交信息与界面文案用中文。
- 文档是双语的：`README.md`（英文，GitHub 默认展示）与 `README.zh-CN.md`，改动功能时**两份同步更新**，并保留各自顶部的语言切换链接。

## 提交与发布

- 提交信息遵循 Conventional Commits，描述用中文：`feat: ...` / `fix: ...` / `style: ...` / `chore: ...`。
- 版本号在 `Cargo.toml`，发版流程：改版本 → 提交 → 打 `v*` tag；CI（`.github/workflows/build.yml`）在 `ubuntu:20.04` 容器里构建 x86_64/arm64 并自动创建 Release。**不要在本地构建发布包**，容器是为了保证 glibc ≤ 2.31 的兼容性。
- 新增 i18n key、依赖或打包资产时，检查 `Cargo.toml` 的 `[package.metadata.deb]` 是否需要同步更新。
