# myterm

一个 Windows 终端：NSH（Native Shell，Rust）+ [libghostty-rs](https://github.com/Uzaaft/libghostty-rs)（`libghostty-vt`，VT 解析与终端状态）。

状态：**M0 技术验证完成**（[报告](docs/m0-report.md)）；**M1 可用**、**M2 的命令面板/右键菜单**、**M4 打包** 已完成并人工验证。设计文档：[docs/design.md](docs/design.md)。

![myterm](icon.png)

## 构建与运行

前置：

- Windows 10 1809+ / Windows 11，MSVC 目标（`x86_64-pc-windows-msvc`）
- Rust 1.90+（`libghostty-vt` 的 MSRV）
- **Zig 0.17.x** 在 `PATH`（或设置 `ZIG`）——`libghostty-vt` 在构建期用 Zig 编译 Ghostty
- Visual Studio Build Tools（C++ 工作负载）与 Windows SDK

```sh
nsh check          # cargo check + TS 类型检查
cargo test         # ConPTY → libghostty-vt → 帧 的端到端测试
nsh run            # 打开窗口（= cargo run）
nsh package --target windows   # 生成 .nsh/out/myterm/ 与 myterm.zip
```

首次构建会克隆固定 commit 的 Ghostty 并编译，约 1–2 分钟，Zig 缓存 ~340 MB。离线/可复现构建：

```sh
GHOSTTY_SOURCE_DIR=<ghostty checkout> GHOSTTY_ZIG_SYSTEM_DIR=<zig package dir> cargo build
```

依赖按 git commit 锁定（`libghostty-vt` 仍是 pre-1.0）。注意：crates.io 的 `libghostty-vt 0.2.2` 固定的 Ghostty 要求 **Zig 0.15.2**；当前 git commit 要求 **Zig 0.17.x**。

## 功能

- **标签**：新建、关闭、切换；每个标签独立会话与回滚；标题跟随 shell 的 OSC 标题。
- **配置文件**：发现 PowerShell 7 / Windows PowerShell / cmd / WSL / Git Bash；`+` 默认配置，`⌄` 选择。
- **选择与剪贴板**：鼠标拖选、全选、复制、粘贴；右键菜单含复制/粘贴/全选/查找/关闭窗格。
- **查找**（`Ctrl+Shift+F`）：搜索回滚缓冲，显示 `当前/总数`，`Enter`/`↑↓` 跳转，当前命中高亮，`Esc` 关闭。
- **命令面板**（`Ctrl+Shift+P`）：按标题或英文关键字过滤，`Enter` 执行，含新建标签（按配置）、查找、复制、粘贴、全选、关闭、滚动。
- **退出状态**：进程退出后顶部出现退出条（退出码、重新启动、关闭窗格），标签标记「已退出」。
- **启动失败 / 空状态**：就地错误面板；关闭最后一个标签后显示新建入口。
- **打包**：`nsh package --target windows` 生成带图标的单目录与 zip。

### 快捷键

| 命令 | 快捷键 |
| --- | --- |
| 新建标签（默认配置） | `Ctrl+Shift+T` |
| 关闭标签 / 窗格 | `Ctrl+Shift+W` |
| 下一个 / 上一个标签 | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| 跳到标签 1–9 | `Ctrl+1`…`Ctrl+9` |
| 查找 / 下一个 / 上一个 | `Ctrl+Shift+F` / `Enter` / `Shift+Enter` |
| 命令面板 | `Ctrl+Shift+P` |
| 复制 / 粘贴 | `Ctrl+Shift+C` / `Ctrl+Shift+V` |
| 全选 | `Ctrl+Shift+U`（`Ctrl+Shift+A` 被输入法/输入框占用） |
| 向上 / 向下回滚 | `Shift+PageUp` / `Shift+PageDown` |
| `Ctrl+C` 等终端按键 | 原样发给 shell（由 libghostty 编码） |

## 结构

```text
src/lib.rs        AppState、标签栏、窗格、查找条、命令面板、右键菜单、快捷键
src/grid.rs       单元格度量与 Canvas 绘制（背景段、文本、选区、光标）
src/term.rs       VT 线程：Terminal + RenderState → 帧；选择/查找/滚动
src/session.rs    ConPTY：读/写线程、resize、退出轮询、命令通道
src/profiles.rs   配置文件发现
src/keys.rs       NSH keystroke → libghostty key::Key/Mods
tests/pty_vt.rs   ConPTY → 网格 的端到端测试
tests/profiles.rs 配置文件发现的测试
```

线程模型：PTY 读线程 → VT 线程（独占 `Terminal`，`!Send`）→ 最新帧 → UI 线程 Canvas。帧按 ~60 Hz 节流，未变化时不重绘。

输入：隐藏 `input()` 只负责键盘焦点与 IME；普通按键走 `on_key_press`（每次一个字符），组合键/特殊键走 `on_key` → `key::Encoder`；输入框设为 `paste_behavior(Report)`，粘贴由应用自己写 PTY。

## 已知 NSH 限制

- **右对齐布局不可用**：`justify_between` 的末项与 `absolute().right(..)` 不绘制。尺寸指示和退出条按钮因此行内左对齐。
- **窗口下半部的同级视图不绘制**（M0 记录）：chrome 放顶部；浮层（命令面板、右键菜单）放在画布**之后**才能盖在终端上。
- 通用视图没有滚轮事件：回滚用键盘。

## 未完成

- 分屏（M2）、设置页（M2）、右键「清空回滚缓冲」。
- 滚轮回滚 / 平滑滚动（M3）、kitty graphics（M3）、系统通知（M3）、会话恢复（M3）。
- 中文 IME 实机验证。
