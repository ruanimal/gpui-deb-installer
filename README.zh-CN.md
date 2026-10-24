# GPUI DEB Installer 📦

**中文** | [English](README.md)

一个使用 [GPUI](https://www.gpui.rs/) 框架（`gpui-kit 0.7.3`）编写的高性能、轻量级图形化 `.deb` 软件包安装器。

![Screenshot](screenshot.png)
![Screenshot2](screenshot2.png)

## 特性 ✨

- **全新界面与交互**：跟随主题的卡片布局与单行紧凑导航，包状态与摘要同行显示；文件页直接展示目录与预览，省去重复标题与说明。详情区域独立滚动，安装操作始终可见。支持在欢迎页拖入单个 `.deb`、一键复制操作日志与预览文件路径，并提供明确的加载、空状态、错误与重试反馈。 桌面图标与应用内标识统一为靛蓝色安装包图形；工具栏支持切换深浅主题，详情页卸载前增加确认。
- **快速流畅**：基于 GPU 加速的 GPUI 渲染引擎，响应极其迅速。
- **全面信息展示**：支持查看包名、版本、描述、维护者、安装大小、依赖等元数据。
- **分类信息区分**：分别显示软件包分区（`Section`）和应用分类（`Categories`）。应用分类读取所选 `.deb` 内可见应用的 `.desktop` 文件，多入口时合并去重，缺失时显示“未知”。
- **已安装管理**：列出 `dpkg` 已知的全部软件包，支持搜索、表头排序，并显示安装/更新时间。
- **文件预览**：预览 `.deb` 包内文件。
- **系统集成**：支持通过命令行参数启动，可直接在文件管理器中右键打开 `.deb` 文件。
- **权限安全**：使用 `pkexec` 调用系统软件包管理器，确保操作安全受控。
- **可选清除配置**：安装页与已安装列表均可在卸载时选择是否清除软件包配置文件（purge）。每次选择软件包时默认不勾选，不影响用户主目录中的配置。

## 已安装列表 📋

「已安装」标签页直接读取 `apt list`，不再自己维护安装记录，因此始终与系统实际状态一致。

- 默认只列出本地安装（或已从所有软件源中移除）的包，对应 `apt` 的 `?installed ?obsolete` 匹配模式；打开 **显示软件源中的包** 后列出全部已安装包。
- 支持按包名或版本搜索、点击表头排序、拖拽列边界调整列宽（多余宽度全部分给包名），长列表虚拟滚动。
- 安装时间与更新时间从 `dpkg.log`（含 `.gz` 轮转日志）解析；日志已被清理的包回退到 `/var/lib/dpkg/info/<pkg>.list` 的修改时间，因此时间可能为空。

## 系统要求 🛠️

- **操作系统**：Linux (Ubuntu, Debian, Deepin, UOS 等基于 Debian 的发行版)
- **核心依赖**：`dpkg`, `apt`, `policykit-1` (pkexec)
- **编译依赖**：Rust 工具链, `libvulkan-dev`, `libssl-dev`, `pkg-config`, `libwayland-dev` (如果您在 Wayland 下)

## 快速开始 🚀

### 1. 编译与运行

首先确保您已安装 Rust。

```bash
# 运行 (不加载文件)
cargo run --release

# 运行 (加载指定 deb 文件)
cargo run --release /path/to/your/package.deb
```

### 2. 桌面环境集成 (开发模式)

本工作区提供了一个 `install-icon.sh` 脚本，可将应用右键菜单和图标集成到您的本地桌面系统中，而无需安装到 `/usr/bin`。

```bash
chmod +x install-icon.sh
./install-icon.sh
```

集成后，您可以直接在文件管理器（如 Nautilus, Thunar 或 Dolphin）中**右键点击 `.deb` 文件**并选择 **"DEB Installer"**。

### 3. 构建发布 DEB 包

```bash
# 安装 cargo-deb
cargo install cargo-deb

# 构建 .deb 包
cargo deb
```

生成的安装包将位于 `target/debian/gpui-deb-installer_*.deb`。

