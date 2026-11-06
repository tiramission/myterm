# myterm

一个 Windows 终端：NSH（Native Shell，Rust）+ [libghostty-rs](https://github.com/Uzaaft/libghostty-rs)（`libghostty-vt`，VT 解析与终端状态）。

当前状态：**M0 技术验证完成**（见 [`docs/m0-report.md`](docs/m0-report.md)）。设计文档：[`docs/design.md`](docs/design.md)。

## 构建

前置：

- Windows 10 1809+ / Windows 11，MSVC 目标（`x86_64-pc-windows-msvc`）
- Rust 1.90+（`libghostty-vt` 的 MSRV）
- **Zig 0.17.x** 在 `PATH`（或设置 `ZIG`）——`libghostty-vt` 在构建期用 Zig 编译 Ghostty 的静态库
- Visual Studio Build Tools（C++ 工作负载）与 Windows SDK

```sh
nsh check          # cargo check + TS 类型检查
cargo test         # ConPTY → libghostty-vt → 帧数据的端到端测试
nsh run            # 打开窗口（= cargo run）
nsh package --target windows   # 打包
```

首次构建会克隆固定 commit 的 Ghostty 并编译，约 1–2 分钟，Zig 缓存 ~340 MB。离线/可复现构建：

```sh
GHOSTTY_SOURCE_DIR=<ghostty checkout> \
GHOSTTY_ZIG_SYSTEM_DIR=<zig package dir> \
cargo build
```

依赖在 `Cargo.toml` 里**按 git commit 锁定**（`libghostty-vt` 仍是 pre-1.0，API 会变）。注意：crates.io 上的 `libghostty-vt 0.2.2` 固定的 Ghostty 版本要求 **Zig 0.15.2**；当前 git commit 要求 **Zig 0.17.x**。

## 结构

```text
src/lib.rs        NSH 根组件：状态条 + Canvas 终端表面 + 隐藏输入 + 按键绑定
src/grid.rs       单元格度量与 Canvas 绘制（背景段合并、文本、光标）
src/term.rs       VT 线程：Terminal + RenderState，把 PTY 字节折叠成纯数据 Frame
src/session.rs    ConPTY（portable-pty）：读线程、写线程、resize、shell 选择
src/keys.rs       NSH keystroke → libghostty key::Key/Mods
tests/pty_vt.rs   ConPTY → 网格的端到端测试（不需要窗口）
```

线程模型：PTY 读线程 → VT 线程（独占 `Terminal`，`!Send`）→ 最新帧 → UI 线程 Canvas。帧按 ~60 Hz 节流，未变化时不重绘。
