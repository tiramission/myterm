# myterm

[![CI](https://github.com/tiramission/myterm/actions/workflows/ci.yml/badge.svg)](https://github.com/tiramission/myterm/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/tiramission/myterm?label=release)](https://github.com/tiramission/myterm/releases/latest)
![Windows](https://img.shields.io/badge/platform-Windows-0078d4)
![GPUI Kit](https://img.shields.io/badge/GPUI_Kit-0.9-4f7d14)

一个 Windows 终端：GPUI Kit 做界面，[libghostty-vt](https://lib.ghostty.org/vt/index.html) 1.0.0 的 C API 做 VT 解析。

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshot-dark.png">
  <img src="docs/screenshot.png" alt="myterm 的窗口：标题栏、窗格列表与两个终端窗格" width="880">
</picture>

状态：**可运行**。终端渲染、键盘输入、中文输入法、多标签、分屏与拖动调整比例、选区与复制、光标闪烁、滚轮回滚、配色与退出提示均已人工验证。设计见 [docs/design.md](docs/design.md)。

## 安装

从 [Releases](https://github.com/tiramission/myterm/releases) 下载 `myterm.exe`，双击运行。

装到系统里（开始菜单、桌面快捷方式、「设置 → 应用」里的卸载项）不必重新下载：打开标题栏右侧的信息按钮，点「安装到本机」。全部按用户级进行，不需要管理员权限。

<img src="docs/icon-small.png" alt="" width="64" align="right">

## 构建与运行

前置：

- Windows 10 1809+ / Windows 11，MSVC 目标（`x86_64-pc-windows-msvc`）
- Rust 1.90+
- Visual Studio Build Tools（C++ 工作负载）与 Windows SDK

不需要 CMake、Zig 或任何系统级依赖：`libghostty-vt` 的静态库已随仓库提供。

```sh
cargo build -p myterm  # 构建
cargo run -p myterm    # 打开窗口
cargo test             # 布局、安装、版本比较与 VT 绑定的单元测试
cargo clippy --workspace --all-targets  # CI 会把警告当错误
```

## 结构

```
crates/
  vt-sys/   libghostty-vt 1.0.0 的裸 FFI 声明 + 预编译静态库
  vt/       安全封装：所有权、错误、颜色、渲染状态、按键编码
  pty/      ConPTY 进程与读写线程
  myterm/   GPUI Kit 界面：标签、窗格、渲染、快捷键、更新与安装
```

依赖只向下：`myterm → pty/vt → vt-sys`。`vt-sys` 之外没有 `unsafe`。

## 功能

### 终端

- **渲染**：字体为 Maple Mono NF CN（等宽编程字体，自带中文字形与 Nerd Font 图标），自定义 `Element` 按行迭代绘制，支持粗体、斜体、下划线、反显、隐藏、删除线与上划线；宽字符占两格。
- **输入**：普通字符、功能键、方向键、`Ctrl` 组合键；按键编码交给解析器，因此 `application cursor keys` 等模式自动正确。
- **中文输入**：窗格把自己注册为平台文本输入目标，输入法组字提交后写入 PTY；候选窗跟随光标位置。
- **光标**：按终端请求显示竖线、下划线、空心块或实心块；实心块会把下面的字符反色重绘，所以光标压住的那一格仍然可读。默认闪烁，光标移动后立即恢复实心。
- **选区**：按住左键拖动高亮选中的文字，`Ctrl+Shift+C` 复制。没有选区时复制不做任何事——终端的屏幕大多是已经处理过的输出，把整屏塞进剪贴板很少是想要的。
- **剪贴板**：`Ctrl+Shift+C` 复制选区，`Ctrl+Shift+V` 粘贴。用带 Shift 的组合是因为终端必须把 `Ctrl+C`（中断前台程序）和 `Ctrl+V`（readline 的引号插入）原样交给 shell。
- **滚轮**：回滚视口，超出半屏自动回到底部；有回滚历史时右侧显示轨道与滑块，滑块按可见行数占比收缩。
- **退出提示**：进程退出后在窗格底部显示退出码，保留最后一帧输出。

### 窗口

- **标题栏**：自绘。左边是应用标记（点一下展开/收起侧栏），中间空白处可拖动窗口，右侧按功能分成两组——分屏操作，以及信息、主题、更新徽章——中间用间距隔开，最后是窗口按钮。标签不在这里，在侧栏。窗口框架跟随主题深浅。
- **主题**：跟随系统深浅色启动，`Ctrl+Shift+L` 或标题栏的太阳/月亮按钮切换；终端前景、背景、光标与 ANSI 16 色会随主题一起换，不只是外壳。
- **配色**：暖调近黑/近白的中性色配一个青柠强调色。四个表面层次、三档文字、一条描边，深浅两套结构一致；ANSI 16 色也按同一色系调过。ANSI 色与默认前景/背景注入解析器，因此查询颜色的程序得到与渲染一致的值。
- **标签**：在左侧一栏里，新建（`Ctrl+Shift+T` 或栏头的加号）、关闭（`Ctrl+Shift+Q` 或当前标签行上的叉）；标题跟随 shell 的 OSC 标题，路径会被截成文件名。

### 分屏

- **分屏**：左右（`Ctrl+Shift+D`）、上下（`Ctrl+Shift+E`）、关闭窗格（`Ctrl+Shift+W`）、`Alt+方向键` 切换；窗格是浮在外壳上的卡片，彼此留出间距，拖动间隙即可调整比例。
- **窗格头**：分成多个窗格后，每个窗格顶部有一行名字和状态点（绿=键盘所在，红=进程已退出）。鼠标移上去会出现分屏、缩放、关闭按钮，不必回到标题栏。缩放（`Ctrl+Shift+Z`）让一个窗格临时占满整个卡片，其余窗格保持原样。
- **窗格动作的位置**：单窗格时标题栏有左右/上下分屏两个按钮；一旦分成多窗格，按钮移到每个窗格头里，标题栏的那两个收起来——同一个动作不该在两条命令路径上指向不同的窗格。
- **焦点**：没有键盘的窗格会整体变淡——底色抬起、文字蒙一层薄雾。这比画一圈边框更安静，也更容易一眼看出打字会进哪里。
- **侧栏**：左侧一栏就是标签栏，默认是窄条；点标题栏左上角的应用标记、按 `Ctrl+Shift+B`，或者拖右边缘，都可以在窄条和列表之间切换，宽度只由用户决定（分开窗格不会自动展开）。列表里一个标签占一行——单窗格的标签这一行就是它自己；多窗格的标签变成一组，组头是标签、下面是缩进的窗格行。每行都有状态点、名字和状态（输出中 / 空闲 / 已退出），点标签行切过去，点窗格行切过去并聚焦它。指针进入右边缘时出现和窗格间隙一样的强调色抓手，往下拖过 118px 就回到窄条。窄条里一个标签是一个胶囊：未激活的是普通描边加一层淡底，当前标签换成强调色描边；胶囊里多窗格的标签是上面一个小标签字、下面窗格块，块里是名字首字（重名时用序号），状态不画点而用块的底色表示（输出中 / 已退出各有淡色，选中是白底），新建标签的加号是同样大小的空胶囊。标签全关掉时，空列表里有一行「新建终端」。

### 维护

- **自动更新**：启动后查一次最新 release，有新版本时标题栏出现一个徽章，点一下下载并在退出后替换、重启。下载过程中徽章显示百分比（服务器没给长度时显示已下载的兆数），所以慢和卡住能区分开。没有发布版本时什么都不显示；下载失败会留在徽章上，点一下重试。
- **版本信息**：标题栏右侧的信息按钮点开是一张锚定在它下面的浮层，列出终端解析库、界面框架、平台和安装状态，以及项目地址。它是浮层而不是对话框：这里没有需要决定的事，为一句查询把终端压暗会打断正在做的事。
- **安装与卸载**：还没安装的副本启动时，标题栏下面会出现一条可关闭的提示带（「安装」/「稍后」），安装失败的原因也留在这条带里。浮层里的按钮把程序装到 `%LOCALAPPDATA%\Programs\myterm`，建开始菜单和桌面快捷方式，写入「设置 → 应用」能看到的卸载项，然后启动装好的副本。卸载清掉快捷方式和注册表项，再延迟删除文件——运行中的 exe 删不掉，这一步交给一个等进程退出的脚本。注册表的卸载入口跑的是同一个 `--uninstall`。

## 快捷键

| 按键 | 作用 |
| --- | --- |
| `Ctrl+Shift+T` | 新建标签 |
| `Ctrl+Shift+Q` | 关闭标签 |
| `Ctrl+Shift+W` | 关闭窗格 |
| `Ctrl+Shift+D` | 左右分屏 |
| `Ctrl+Shift+E` | 上下分屏 |
| `Ctrl+Shift+Z` | 缩放窗格 |
| `Ctrl+Shift+B` | 折叠 / 展开侧栏 |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | 复制选区 / 粘贴 |
| `Ctrl+Shift+L` | 切换深浅主题 |
| `Alt+方向键` | 切换窗格 |

## 已知限制

- 系统磨砂材质（Mica / Acrylic）无法透出：Windows 后端通过 DirectComposition 呈现不透明表面，DWM 的材质被它盖住。标题栏改用一层比网格更浅的外壳色来区分层次。
- 查找尚未接入（`vt` 层已提供搜索接口）。
- 仅支持 Windows。
- 自动更新只替换可执行文件，不处理同目录的其他文件；目前也只有这一个文件。
- 字体依赖本机安装 Maple Mono NF CN；未安装时会回退到系统默认等宽字体。
