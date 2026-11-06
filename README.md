# myterm

一个 Windows 终端：NSH（Native Shell，Rust）+ [libghostty-rs](https://github.com/Uzaaft/libghostty-rs)（`libghostty-vt`，VT 解析与终端状态）。

状态：**M0 技术验证完成**（[报告](docs/m0-report.md)），**M1 代码完成并已人工验证**（见下）。设计文档：[docs/design.md](docs/design.md)。

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
nsh package --target windows
```

首次构建会克隆固定 commit 的 Ghostty 并编译，约 1–2 分钟，Zig 缓存 ~340 MB。离线/可复现构建：

```sh
GHOSTTY_SOURCE_DIR=<ghostty checkout> GHOSTTY_ZIG_SYSTEM_DIR=<zig package dir> cargo build
```

依赖按 git commit 锁定（`libghostty-vt` 仍是 pre-1.0）。注意：crates.io 的 `libghostty-vt 0.2.2` 固定的 Ghostty 要求 **Zig 0.15.2**；当前 git commit 要求 **Zig 0.17.x**。

## 功能（M1）

- **标签**：新建、关闭、切换；每个标签独立会话与回滚；标题跟随 shell 的 OSC 标题。
- **配置文件**：启动时发现 PowerShell 7 / Windows PowerShell / cmd / WSL / Git Bash；`+` 用默认配置，`⌄` 按配置新建。
- **选择与剪贴板**：鼠标拖选（蓝底高亮）、全选、复制、粘贴；粘贴走 `Ctrl+Shift+V`。
- **退出状态**：进程退出后窗格顶部出现退出条（退出码、重新启动、关闭窗格），标签标题标记「已退出」。
- **启动失败**：窗格内就地错误面板（命令、原因、关闭标签）。
- **空状态**：关闭最后一个标签后显示新建入口。

### 快捷键

| 命令 | 快捷键 |
| --- | --- |
| 新建标签（默认配置） | `Ctrl+Shift+T` |
| 关闭标签 / 窗格 | `Ctrl+Shift+W` |
| 下一个 / 上一个标签 | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| 跳到标签 1–9 | `Ctrl+1`…`Ctrl+9` |
| 复制 / 粘贴 | `Ctrl+Shift+C` / `Ctrl+Shift+V` |
| 全选 | `Ctrl+Shift+U`（`Ctrl+Shift+A` 被输入法/输入框占用） |
| 向上 / 向下回滚 | `Shift+PageUp` / `Shift+PageDown` |
| `Ctrl+C` 等终端按键 | 原样发给 shell（由 libghostty 编码） |

## 结构

```text
src/lib.rs        AppState（标签集合、活动标签）、标签栏、窗格、快捷键
src/grid.rs       单元格度量与 Canvas 绘制（背景段、文本、选区、光标）
src/term.rs       VT 线程：Terminal + RenderState → 帧；选择文本；视口滚动
src/session.rs    ConPTY：读/写线程、resize、退出轮询、选择与滚动命令
src/profiles.rs   配置文件发现
src/keys.rs       NSH keystroke → libghostty key::Key/Mods
tests/pty_vt.rs   ConPTY → 网格 的端到端测试
tests/profiles.rs 配置文件发现的测试
```

线程模型：PTY 读线程 → VT 线程（独占 `Terminal`，`!Send`）→ 最新帧 → UI 线程 Canvas。帧按 ~60 Hz 节流，未变化时不重绘。

输入：隐藏 `input()` 只负责键盘焦点与 IME；普通按键走 `on_key_press`（每次一个字符），组合键/特殊键走 `on_key` → `key::Encoder`；输入框设为 `paste_behavior(Report)`，粘贴由应用自己写 PTY。

## 已知 NSH 限制（M1 遇到）

- **右对齐布局不可用**：`justify_between` 的最后一个子项、`absolute().right(..)` 都不绘制。因此尺寸指示和退出条按钮改为**行内左对齐**。
- **窗口下半部的同级视图不绘制**（M0 记录）：状态条放顶部，覆盖层只能放在画布上方。
- 通用视图没有滚轮事件：回滚暂用键盘。

## 下一步（M1 剩余）

- 查找（`Ctrl+Shift+F`）、右键菜单、滚轮回滚、标签溢出菜单、设置页。
- 中文 IME 实测（微软拼音）。
