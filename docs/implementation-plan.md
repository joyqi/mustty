# mustty 实现方案

> 状态：设计定稿 v1（2026-10-28）　适用范围：M0–M4
> 本文只描述"怎么做"，不含任何实现代码。所有版本号均于 2026-10-28 通过 `cargo search` / crates.io API 核实。

---

## 1. 目标与非目标

**一句话定位**：mustty 是一个跨平台的终端 Markdown 写作器——打开一个文件，在终端中央一栏固定宽度的纸面上安静地码字，除此之外什么都不做。

### 1.1 目标

| # | 目标 | 可度量的标准 |
|---|---|---|
| G1 | 沉浸式码字 | 界面只有标题 / 正文 / status bar 三块，居中、限宽，无边框、无行号、无菜单 |
| G2 | 中文一等公民 | CJK 宽字符、中文 IME 输入、中英混排软换行在 macOS / Windows / Linux 主流终端下正确 |
| G3 | 零学习成本 | 默认非 modal、M4 提供 --vim；默认键位与 GUI 编辑器一致（Ctrl+S 保存、Ctrl+Z 撤销……） |
| G4 | 不丢字 | 原子写入 + 默认自动保存；终端异常退出 / panic 时恢复终端 |
| G5 | 跨平台单二进制 | macOS (aarch64/x86_64)、Linux (x86_64 musl)、Windows (x86_64 msvc) |

### 1.2 非目标（明确不做）

- **不做所见即所得**：不隐藏 Markdown 标记，不渲染表格 / 图片 / 公式。
- **不做文件浏览器、多 buffer、tab、分屏**：一次只编辑一个文件。
- **默认非 modal、M4 提供 --vim**：`--vim` 只是一个最小的 vim-like 键位层（§7.4），只在 keymap 层实现，编辑内核不感知它。**不做**完整 vim 模拟（计数前缀 `3j`、`.` 重复、宏、具名寄存器、`:` 命令行、text object、`Ctrl+V` 块选择），不做 emacs 模式。
- **只提供 `~/.mustty.toml` 中的 10 个配置项**（§8.5）。不做自定义键位、主题配色、项目级配置、status line 开关、配置热重载；不做可编程配置、插件、LSP。
- **不做预览 / 导出**（HTML、PDF）。
- **不做语法高亮代码块内部**（fenced code 只整体着色）。
- **鼠标默认关闭**：只有配置 `mouse = true` 或使用 `--mouse` 时，才启用单击定位、拖拽选区和滚轮滚动（M4，§10.5）。双击选词、右键菜单、拖放文本都不做。
- **不做 GUI 级别的排版**（字体大小、行距）——终端做不到。
- **不支持非 UTF-8 编码**的读写（见 §8.3）。

---

## 2. 竞品 / 先例调研

### 2.1 GUI 写作器的专注模式

| 产品 | 关键做法 | mustty 的取舍 |
|---|---|---|
| **iA Writer** | 源码模式 + 标记变暗；固定行宽（约 64–72 字符）；Focus Mode 只高亮当前句/段；Typewriter Scrolling 让当前行固定在屏幕中线；文档标题即首个 H1 | **全部借鉴**：标记变暗（§6）、72 列行宽（§5）、标题 = 首行 H1（§5.5）、Typewriter 模式作为 M4 开关 |
| **Typora** | 真 WYSIWYG：标记默认隐藏，光标进入时才显示；标题用大字号 | **否决**：标记隐现会让同一行的列宽随光标进出而变化，终端无动画只会"跳字"；终端也无法用字号表达标题层级 |
| **Obsidian** Live Preview | 当前行显示源码、其他行渲染；有社区"专注模式"插件 | **否决**（同 Typora 理由，代价稍低但仍需要两套坐标映射）；仅借鉴"当前段落高亮"思路放入 M4 |

结论：**终端的物理约束（等宽、无字号、无动画）决定了 iA Writer 路线是唯一合理选择。**

### 2.2 TUI 编辑器

| 项目 | 值得学的 | 不学的 |
|---|---|---|
| **Helix** | 文档模型用 `ropey`；undo 以 *Transaction*（ChangeSet + 选区）为单位；soft wrap 在 `helix-core` 的 doc formatter 里按 grapheme 迭代、在词边界断行、超长词回退到强制断行；滚动用 `scrolloff` | modal；自绘光标而非终端硬件光标（IME 候选框定位相关的社区反馈很多）；历史树对写作者过重 |
| **Kakoune** | selection-first 的光标模型很优雅 | modal、client/server 架构，与"极简码字"无关 |
| **micro** | 非 modal、Ctrl 系键位（Ctrl+S/Q/Z/Y/F）对新手零门槛；使用终端真实光标，IME 体验较好；`softwrap` 选项 | 多 buffer / 插件 / 配置面太大 |

结论：**键位学 micro，内核学 Helix（rope + transaction undo + grapheme 软换行），光标必须用终端硬件光标。**

### 2.3 ratatui 生态

| crate（版本） | 现状 | 结论 |
|---|---|---|
| `tui-textarea` 0.7.0 | 依赖 `ratatui ^0.29`，与当前 `ratatui 0.30` 不兼容；无软换行 | **否决** |
| `ratatui-textarea` 0.9.3（ratatui 官方组织接手的后继） | 已有 `WrapMode::WordOrGlyph`、undo、搜索。但：① 内部是 `Vec<String>` + `(row, char_col)` 光标；② 光标以 `Span::styled(" ", cursor_style)` 自绘（`widget.rs:144`），不放置硬件光标；③ 除选区/搜索外**没有按区间着色的 API**，无法实现 Markdown 标记变暗 | **否决作为内核**。读过其 `wrap.rs` 的分段逻辑作为参考 |
| `edtui` 0.11.7 | vim 风格 modal，依赖 `syntect`（体积大）、`arboard` | **否决**：modal 与 G3 冲突，高亮粒度为代码语法 |
| `tui-markdown` 0.3.12 | 用 `pulldown-cmark` 把 Markdown 渲染成 `ratatui::Text`（只读、隐藏标记） | **否决**：只读渲染器，且是 WYSIWYG 方向；无源码位置映射 |

**结论：自研编辑内核（约 1.5k–2k 行）**。理由：三个关键需求——硬件光标（IME）、按区间着色（标记变暗）、标题/正文分离的文档模型——现成组件都不满足，而 fork 改造的成本高于自研。代价：需要自己写软换行、光标映射、undo，并为它们补齐单测（§13）。

---

## 3. 技术栈与依赖清单

工具链：Rust **edition 2024**，`rust-version = "1.88"`（由 `ratatui 0.30.2` 的 MSRV 决定）。

### 3.1 运行时依赖

| crate | 版本 | 用途 | 为什么是它 / 否决的备选 |
|---|---|---|---|
| `ratatui` | `0.30.2`，`default-features = false, features = ["crossterm"]` | 帧缓冲、diff 渲染、`Frame::set_cursor_position`、`ratatui::init()/restore()`（自带 panic hook 恢复终端） | 事实标准。否决 `cursive`（retained 组件模型、布局不可控）、直接手写 crossterm（要自己做 double buffer diff） |
| `crossterm` | `0.29.0`（默认 features：`bracketed-paste`, `events`, `windows`） | 终端后端 + 事件：raw mode、alternate screen、`Event::Paste`、`FocusGained/Lost`、`SetCursorStyle` | 唯一同时良好支持 Windows Console API 和 Unix 的后端；与 ratatui-crossterm 0.1 的 `crossterm_0_29` 对齐。否决 `termion`（不支持 Windows）、`termwiz`（依赖重、生态小） |
| `ropey` | `1.6.1` | 正文文本存储 | 见 §4.1。不用 `2.0.0-beta.1`：beta、API 仍在变 |
| `unicode-segmentation` | `1.13.3` | grapheme cluster 边界（光标移动、删除）、`unicode_words()` 字数统计 | 标准实现，ratatui 自身也依赖 |
| `unicode-width` | `0.2.2` | 显示宽度（CJK = 2） | **必须与 ratatui-core 使用同一版本**（其约束为 `>=0.2.0, <=0.2.2`），否则我们算的列宽与 ratatui 写入 Buffer 的宽度不一致，会出现光标错位 |
| `unicode-linebreak` | `0.1.5` | UAX #14 断行机会（中英混排软换行、避头尾） | 否决 `textwrap 0.16`：它返回 `Cow<str>` 行而非源偏移，我们需要的是 char 区间映射；否决自写"空格 + CJK 任意断"规则：不处理 `，。」` 等避头尾 |
| `anyhow` | `1.0.104` | `main` / `App` 层错误聚合 | 应用层惯例 |
| `tracing` | `0.1.44` | 调试日志宏 | 否决 `log`+`env_logger`：后者默认写 stderr，会污染 TUI |
| `tracing-subscriber` | `0.3.23`，features `["env-filter", "fmt"]` | 日志过滤与格式化 | — |
| `tracing-appender` | `0.2.5` | 写文件（非阻塞） | 日志**只在设置 `MUSTTY_LOG=<path>` 时启用**，永不写 stdout/stderr |
| `clap` | `4.6.7`，features `["derive"]` | CLI：`mustty [FILE] [--width N] [--[no-]autosave] [--[no-]vim] [--[no-]mouse]`，只有这四个覆盖项（M1 只有 `FILE`、`--width`、`--no-autosave`，其余在 M4 加入）。开关成对出现，是为了让"命令行 > 配置文件"能双向覆盖：配置里写了 `vim = true`，仍可用 `--no-vim` 关掉。成对开关用 clap 的 `overrides_with` 实现"后者生效" | 免费获得 `--help/--version` 和参数校验；否决 `lexopt`/`pico-args`：省下的编译时间不值得手写 help |
| `serde` | `1.0.229`，features `["derive"]` | 给 `CodeColor` 等配置值类型派生 `Deserialize`（M4） | 事实标准，`toml` 的反序列化也建立在它之上 |
| `toml` | `1.1.8`（默认 features） | 解析 `~/.mustty.toml`：先解析成 `toml::Table`，再逐个键 `Value::try_into::<T>()`，这样单个键类型错误不会拖垮整个文件（§8.5）（M4） | 否决 `serde_json` / `serde_yaml`：用户已拍板使用 TOML；否决 `toml_edit`：只读不写，不需要保留格式 |
| `dirs` | `7.0.0` | `dirs::home_dir()` 定位 `~/.mustty.toml`（Windows 为 `%USERPROFILE%\.mustty.toml`）（M4） | 否决读 `$HOME`：Windows 上没有这个变量；否决 `directories` / `etcetera`：它们面向 XDG / AppData，而用户要求的路径固定在 home 根目录 |
| `sys-locale` | `0.3.2` | 环境变量都没设时取系统语言（主要为 Windows），见 §11.4 | 体积约为 0、无传递依赖；否决 fluent / rust-i18n：几十条字符串用不上复数引擎和宏，见 §11.4 |
| `tempfile` | `3.27.0` | 原子保存：同目录临时文件 + `persist()`（Unix `rename`，Windows `MoveFileExW(REPLACE_EXISTING)`） | 否决 `atomic-write-file 0.3`：功能等价但用户少；`tempfile` 测试里也要用，复用一个依赖 |

### 3.2 开发依赖

| crate | 版本 | 用途 |
|---|---|---|
| `insta` | `1.49.0` | Markdown 扫描结果、`TestBackend` 渲染结果的快照测试 |
| `proptest` | `1.12.0` | 软换行 / 光标映射的性质测试 |
| `tempfile` | `3.27.0` | 文件读写测试 |

### 3.3 明确不引入

M4 的 `--vim` **不新增任何 crate**：Normal/Insert 状态机和按键序列（`gg`、`dd`）都在 `keymap.rs` 里手写（估计约 200 行），clap 只多一个 bool 参数。否决 `edtui` 的 vim 引擎：它和自己的 buffer 是绑在一起的（§2.3）。

此外明确不引入：`pulldown-cmark`（理由 §6.3）、`syntect`（体积 + 非目标）、`arboard`（系统剪贴板在 SSH / Linux 无 X 下不可靠，改用 OSC 52，§7.3）、`tokio`（同步事件循环足够，§11.3）、`undo`（§4.5）、`notify`（配置不做热重载，§8.5）。`dirs` 已从这里移出，因为配置文件需要它（§3.1）。
`thiserror` 已移除：`IoError` 的文案要按界面语言取，`#[error]` 只收字面量，改为手写 `Display`（§11.4）。

---

## 4. 编辑器内核设计

### 4.1 文档模型：`ropey::Rope`（结论）

- **结论**：正文存为 `ropey::Rope`；标题单独存为 `String`（§5.5）。
- **理由**：
  - `char_to_line / line_to_char / char_to_byte` 均为 O(log n)，光标用"全局 char 索引"表示时与行号互转零成本；
  - 编辑 O(log n)，长文（10 万字小说章节合并稿）不卡；
  - Helix 已验证其在编辑器场景的稳健性。
- **被否决：`Vec<String>`**。实现更简单，且散文单行通常 < 2k 字符，性能其实够用；否决原因是它把"行"作为一等公民，跨行删除 / 粘贴 / undo 的偏移计算都要手写 `(row, col)` 拆分合并逻辑，而 rope 把它们统一成区间替换。
- **代价**：`RopeSlice` 不能直接喂给 `unicode-segmentation`。对策：**以逻辑行为单位** `rope.line(i).to_string()` 成 `String` 再做 grapheme / wrap，结果放进 `WrapCache`（§4.3），仅在该行被编辑或宽度变化时重算。

**文件大小软上限 5 MB**（原 §14 风险 5，已定为规则）：超过这个大小的文件，打开时 status bar 提示一次，字数统计改为只在保存时刷新；不做异步计算。

文本规范化：内存中**换行一律为 `\n`**，不含 BOM；原始换行符与 BOM 记在 `FileMeta`，保存时还原（§8.3）。

### 4.2 光标模型

```text
struct Cursor {
    head: usize,              // 全局 char 索引（rope 坐标），永远落在 grapheme 边界
    anchor: Option<usize>,    // Some 时 [min(anchor,head), max) 为选区（M3）
    preferred_col: Option<u16>, // 上下移动时保持的"视觉列"，任何水平移动/编辑后清空
}
```

规则（全部可单测）：

1. **水平移动以 grapheme cluster 为单位**：`←/→`、Backspace、Delete 都通过 `grapheme::prev_boundary(line, idx)` / `next_boundary` 计算。`é`（e + 组合符）、`👨‍👩‍👧`、`🇨🇳` 都是一步。
2. **显示宽度**：`grapheme_width(g) = if g == "\t" { TAB_WIDTH - col % TAB_WIDTH } else { max(1, UnicodeWidthStr::width(g)) }`，`TAB_WIDTH` 取配置 `tab_width`（默认 4，见 §8.5）。对非控制字符最少计 1，避免零宽 grapheme 导致光标重叠。
3. **上下移动按视觉行**（软换行后的行）：取 `preferred_col.unwrap_or(当前视觉列)`，在目标视觉行上找"起始列 ≤ preferred_col 的最后一个 grapheme"。**CJK 宽字符**：若 preferred_col 落在宽字符的第二格，吸附到该字符左侧（与 VS Code / micro 一致）。
4. **行尾位置**：光标可位于逻辑行末（`\n` 之前）。软换行处的"行尾 / 下一行行首"是同一个 char 索引，**一律显示在下一视觉行行首**（`Affinity::Downstream`），`End` 键例外：`End` 把光标放在本视觉行最后一个 grapheme 之后，并设置 `affinity = Upstream` 使其显示在本行末尾。
5. Ambiguous-width 字符（`“”…—`）按 **narrow (1)** 计（unicode-width 默认、与 ratatui 一致），风险见 §14。

### 4.3 软换行与视觉行映射

**数据结构**

```text
struct VisualRow {
    start: usize,   // 行内 char 偏移（含）
    end: usize,     // 行内 char 偏移（不含）
    width: u16,     // 本行显示宽度（不含悬挂空白）
}
struct WrapCache {
    width: u16,                     // 生成时的换行宽度；变化 → 全部失效
    lines: Vec<Option<Vec<VisualRow>>>, // 下标 = 逻辑行；None = 脏
}
```

**算法 `wrap_line(line: &str, width: u16) -> Vec<VisualRow>`**

1. `opps = unicode_linebreak::linebreaks(line)` → 收集 `BreakOpportunity::Allowed` 的字节偏移，转为 char 偏移集合。
2. 依次遍历 grapheme，累加列宽 `col`；记录 `last_opp`（当前视觉行内最近的断行机会）。
3. 当加入 grapheme `g` 后 `col > width`：
   - 若 `g` 是空白：**悬挂**（hanging whitespace）——留在当前行，不触发换行，不计入 `width`；渲染时被裁剪，光标若在其后则显示在 `width - 1` 列。
   - 否则若 `last_opp > row_start`：在 `last_opp` 处断行，从 `last_opp` 重新开始累计。
   - 否则（单个词比整行还宽，如长 URL）：在 `g` 之前**强制断行**。
4. 空行产生一个 `VisualRow { start: 0, end: 0, width: 0 }`，保证每个逻辑行至少一行。

UAX #14 天然覆盖：英文在空格后断；CJK 字符间任意断；`，。」）` 不出现在行首、`「（` 不出现在行尾（避头尾）。

**失效策略**：编辑只把受影响的逻辑行（区间替换涉及的 `[first_line, last_line]`，插入换行则在 `lines` 中插入槽位）标脏；`width` 变化时 `lines.fill(None)`。只为**可见行 + 光标行**惰性计算，不对全文预先 wrap。

**坐标映射（两个纯函数，单测核心）**

```text
fn char_to_visual(doc, cache, idx: usize, aff: Affinity) -> VisualPos { line, row, col }
fn visual_to_char(doc, cache, pos: VisualPos) -> usize   // 列落在宽字符中间 → 吸附左侧
```

### 4.4 滚动策略

- 视口顶部用 **逻辑锚点** 表示：`struct ViewTop { line: usize, row: usize }`（第 line 个逻辑行的第 row 个视觉行），而不是"全文第 N 个视觉行"——这样无需 wrap 整篇文档。
- **默认：scrolloff = 3**。每次光标变化后：若光标视觉行距视口上 / 下边 < 3 行，最少量滚动使其恢复到 3 行边距（正文高度 < 8 时 scrolloff 取 `(body_h - 1) / 2`）。
- **Typewriter 模式（M4，`Ctrl+T` 切换，默认关）**：光标视觉行恒定在 `body_h / 2`（向下取整）处；文档开头允许上方留白，即 `ViewTop` 可为"负"——实现为 `top_padding: u16`。
- resize 后：先用旧 `ViewTop.line` 的首字符 char 索引在新宽度下重新定位 row，再执行一次 scrolloff 校正。

### 4.5 undo / redo：自研（结论）

```text
struct Edit { at: usize, deleted: String, inserted: String }   // char 坐标的区间替换
struct Transaction { edits: Vec<Edit>, before: Cursor, after: Cursor, title_before: Option<String> }
struct History { undo: Vec<Transaction>, redo: Vec<Transaction>, last_edit_at: Instant, max: usize /*1000*/ }
```

- **线性栈**（新编辑清空 redo），不做 Helix 那样的历史树——写作者不需要分支历史。
- **合并规则**（决定"一次 Ctrl+Z 撤回多少"）：连续的单 grapheme 插入，若 ① 与上一个 Edit 位置相接，② 间隔 < 1 s，③ 不跨越"空白 → 非空白"边界（即以词为单位），则并入同一 Transaction。以下操作总是开新 Transaction：换行、粘贴、删除方向改变、光标移动、保存、标题/正文焦点切换。
- 标题编辑也进同一个 History（`title_before` 字段），保证 Ctrl+Z 行为一致。
- **被否决：`undo` 0.52**。它提供的是通用 command pattern；合并规则、光标恢复依然要自己写，而 rope 区间替换的逆操作本身只有十几行。多一层泛型抽象不划算。
- 内存上限：`max = 1000` 个 Transaction，超出丢最旧。

---

## 5. 渲染与布局

### 5.1 常量

| 名称 | 值 | 说明 |
|---|---|---|
| `MEASURE` | **72** 列 | 正文最大宽度（= 72 个 ASCII 或 36 个汉字）。排版学推荐 60–75 字符/行；80 在中文下显得过宽。可用配置 `width` 或 `--width N` 覆盖（命令行优先），取值钳制在 `[40, 120]`（§8.5） |
| `MIN_SIDE_MARGIN` | 2 列 | 窄终端时左右最少留白 |
| `GAP` | 1 行 | 标题与正文、正文与 status bar 之间的空行 |
| `V_PAD_RATIO` | 1/8 | 上下留白占终端高度的比例 |

### 5.2 布局算法（纯函数 `compute_layout(area: Rect, measure: u16) -> Layout`）

设终端宽高 `W × H`（整数，单位为 cell），所有除法为**向下取整**。

**水平**

```text
if W >= measure + 2*MIN_SIDE_MARGIN:   w = measure
elif W >= 24:                          w = W - 2*MIN_SIDE_MARGIN
else:                                  w = W                       // 极窄：放弃留白
x = (W - w) / 2                                                    // 奇数余量给右侧
```

**垂直**（按高度分级退化）

```text
level = if H >= 12 { Full } elif H >= 5 { Compact } elif H >= 2 { BodyStatus } else { BodyOnly }

Full:       v_pad  = max(1, H / 8)
            block_h = H - 2*v_pad
            body_h  = block_h - 4          // 标题1 + GAP1 + GAP1 + status1
            y0      = (H - block_h) / 2    // 余量给底部
            title   = Rect(x, y0,              w, 1)
            body    = Rect(x, y0 + 2,          w, body_h)
            status  = Rect(x, y0 + 2 + body_h + 1, w, 1)
Compact:    无 v_pad、无 GAP：title(y=0) / body(y=1, h=H-2) / status(y=H-1)
BodyStatus: 无标题：body(y=0, h=H-1) / status(y=H-1)
BodyOnly:   body = 整个区域，无 title/status
```

示例：`120×40` → `w=72, x=24, v_pad=5, block_h=30, body_h=26, y0=5`；`80×24` → `w=72, x=4, v_pad=3, block_h=18, body_h=14, y0=3`；`60×10` → Compact，`w=56, x=2, body_h=8`。

`W < 10` 时不渲染内容，只在 (0,0) 起写 `"窗口太小"`（裁剪），并隐藏光标；恢复尺寸后状态完整保留。

**输出类型**

```text
pub struct Layout { pub title: Option<Rect>, pub body: Rect, pub status: Option<Rect>, pub too_small: bool }
```

### 5.3 resize 行为

收到 `Event::Resize(w, h)` → 重新 `compute_layout` → 若 `body.width` 变化则 `WrapCache` 全部失效 → 按 §4.4 重新定位视口 → 立即重绘（ratatui 的 `Terminal::draw` 会在尺寸变化时 `autoresize` 并全量清屏，避免残影）。同一批事件中多个 Resize 只处理最后一个。

### 5.4 渲染细节

- 三个区域都是自定义 `Widget`（`TitleWidget` / `BodyWidget` / `StatusWidget`），直接写 `Buffer`：按视觉行逐段 `buf.set_stringn(x, y, seg, max_w, style)`，宽字符的占位 cell 由 ratatui 处理。
- **不设置任何背景色**，不画边框——完全沿用终端主题背景，这是"沉浸"的前提。
- **硬件光标**：每帧调用 `frame.set_cursor_position(Position { x, y })`，并在启动时执行 `SetCursorStyle::SteadyBar`（退出时 `DefaultUserShape`）。**不自绘光标**——IME 预编辑文字与候选框都锚定在硬件光标处（§10.1）。
- 只在状态变化（事件、定时器到期）后重绘，空闲时 0 CPU。

### 5.5 标题区语义：绑定文档首行 H1（结论）

- **结论**：标题区显示并编辑**文档第一行的 ATX 一级标题** `# 标题`。加载时把它从正文中剥离，正文区从剩余内容开始显示；保存时拼回去。
- **识别规则**（加载时一次判定）：文件（去掉 BOM 后）第一行以 `"# "` 开头，且不以 `#` 结尾（排除 `# 标题 #` 闭合写法，避免回写变形）→ `TitleField::Bound { text: 行剩余部分原样, sep: 紧随其后的连续换行符原样 }`；否则 `Unbound`。文件以 `---` YAML front matter 开头时**一律 Unbound**（M4 再考虑）。
- **回写保真**：若标题与 sep 未改动，保存后的字节与原文件完全一致（单测覆盖）。
- **Unbound 时**：标题区以 DIM 样式显示文件名（无扩展名）或 `Untitled` 作为占位；用户一旦在标题区输入字符，即变为 `Bound { sep: "\n\n" }`，保存时在文件头插入 `# 标题\n\n`。Bound 且 text 被清空 → 保存时整行 H1 连同 sep 删除。
- **编辑方式**：焦点 `enum Focus { Title, Body }`。
  - 正文第一视觉行按 `↑`、或 `Ctrl+Home` 后再按 `↑` → 进入标题，光标置于标题末尾；
  - 标题中按 `Enter` 或 `↓` → 回到正文开头；
  - 标题是单行：粘贴内容中的换行替换为空格；超过 `w` 时以光标为中心水平滚动，未超时**居中显示**、BOLD。
- **被否决**：
  - *仅显示文件名*：标题与内容脱节，改名要离开编辑器；
  - *独立 front matter `title:` 字段*：引入 YAML 解析，且多数 Markdown 文件没有 front matter；
  - *只读显示首个 H1 但仍在正文里编辑*：标题会在正文和标题区重复出现。
- **代价**：文档模型多一个字段，所有 char 坐标仅指正文；undo 需覆盖标题（§4.5）；字数统计需合并两者。

---

## 6. Markdown 呈现策略

### 6.1 结论：源码 + 标记变暗（iA Writer 式）

所有字符原样显示，光标坐标与源文本一一对应；只通过样式区分语义：

| 元素 | 标记部分 | 内容部分 |
|---|---|---|
| ATX 标题 `## x` | `##␠` DIM | BOLD |
| 强调 `*x*` / `_x_` | DIM | ITALIC |
| 加粗 `**x**` / `__x__` | DIM | BOLD |
| 行内代码 `` `x` `` | DIM | 配置 `code_color`（默认 `cyan`） |
| fenced code ```` ``` ```` | 栅栏行 DIM | 整块使用 `code_color` |
| 引用 `> ` | DIM | ITALIC |
| 列表 `- ` `* ` `1. ` `- [ ]` | DIM | 默认 |
| 链接 `[t](url)` | `[ ]( )` 与 url DIM | 文本 UNDERLINED |
| 分隔线 `---` | 整行 DIM | — |

只用 `Modifier` + 1 个 ANSI 基本色，不写死 RGB，不设背景——随终端主题自适应。`DIM` 在 Windows Terminal / iTerm2 / WezTerm / macOS Terminal / GNOME Terminal 均已支持；不支持时退化为正常显示，不影响可用性。

### 6.2 被否决：WYSIWYG（隐藏标记）

需要维护"源坐标 ↔ 显示坐标"的第二套映射，且光标进入/离开时标记出现/消失会改变行宽、触发软换行重排，导致整段文字左右跳动。估算额外 1.5k+ 行代码和大量边界 bug，换来的体验在终端里反而更差。

### 6.3 解析：自研逐行轻量扫描器（结论）

```text
enum BlockState { Normal, Fenced { fence: char, len: u8 } }
struct StyledRange { start: usize, end: usize, kind: Token }   // 行内 char 偏移
fn scan_line(line: &str, state_in: BlockState) -> (Vec<StyledRange>, BlockState)
```

- 每个逻辑行只依赖"进入时的块状态"（目前只有是否在 fenced code 内），因此可缓存 `Vec<(BlockState, Vec<StyledRange>)>`，编辑某行后只需从该行起重扫，直到某行的输出 `state` 与缓存一致即停止（通常 1 行）。
- 行内规则（顺序匹配、不回溯、不跨行）：行首块标记（标题 / 引用 / 列表 / 分隔线 / 栅栏）→ 行内代码优先 → `**`/`__` → `*`/`_`（要求左侧非字母数字或行首，避免 `snake_case` 误判）→ 链接。未闭合的标记**不着色**（正在输入 `**加` 时不会把后文全部变粗）。
- **被否决：`pulldown-cmark` 0.13.5**。它精确，但 ① 只能全文解析，每次按键全量重解析 + 重新映射；② 正在输入的半截语法（`**未闭合`、`[链接未完`）CommonMark 语义会让着色在输入中频繁闪变；③ 不支持跨行 setext / 懒续行这类"精确性"对写作者没有价值。代价：我们的高亮在嵌套强调、跨行强调等边角情况下与 CommonMark 不一致——**可接受，这只是视觉提示，不影响文件内容**。

---

## 7. 键位表

### 7.1 结论：默认非 modal、M4 提供 --vim

- **默认**：非 modal，使用 Ctrl 系键位（micro / GUI 习惯），见 §7.2。理由：写作时绝大部分时间都在输入文字，默认就有模式切换会增加心智负担。不加 `--vim` 时，M1–M4 的行为和本节键位表完全一致。
- **`--vim`（M4 交付，M1–M3 完全不实现）**：加上这个启动参数后，启用 vim-like 的 Normal / Insert 两种状态（§7.4）。
- **modal 只存在于 keymap 层**：`enum Mode { Normal, Insert }` 由 `App` 持有，只传给 `Keymap::map_event(ev, mode, prompt_active)`。`Action`、`Editor`、`Document` 一律不感知 Mode。Normal 态的每个键都映射成已有的 `Action` / `Motion`，模式切换由 keymap 直接改 `mode`，不产生任何 Action。因此 M4 时**编辑内核零改动**。前提是 vim 映射需要的几个与模式无关的 Action（`OpenLineBelow`、`DeleteLine`、`PasteRegister`、`Motion::WordEnd`）在 M1 时一次性写进 Action 定义，并在 M3 随选区 / 寄存器一起实现和单测（§11.2、§12）。
- 不加 `--vim` 时 `mode` 恒为 `Insert`，keymap 对 `Esc` 不做模式切换。

**事件过滤**：只处理 `KeyEventKind::Press`（以及 `Repeat`）。Windows 上 crossterm 会同时上报 `Release`，不过滤会导致每个字符输入两次。**不启用** kitty keyboard enhancement（`PushKeyboardEnhancementFlags`）：macOS Terminal 与 Windows conhost 不支持，且与部分终端的 IME 交互有问题；因此 `Ctrl+Shift+Z`、`Shift+Enter` 不可区分，键位设计避开它们。

### 7.2 键位表

| 键 | 动作 | 里程碑 |
|---|---|---|
| 可打印字符 / IME 提交文本 | 在光标处插入 | M1 |
| `Enter` | 插入 `\n`（标题区：跳到正文） | M1 |
| `Backspace` / `Delete` | 删除前 / 后一个 grapheme；有选区时删除选区 | M1 |
| `Tab` | 插入 `tab_width` 个空格（默认 4，§8.5） | M1 |
| `←` `→` | 按 grapheme 移动 | M0（只读滚动）/ M1 |
| `↑` `↓` | 按视觉行移动（正文首行 `↑` 进入标题） | M0 / M1 |
| `Home` / `End` | 视觉行首 / 尾 | M1 |
| `PageUp` / `PageDown` | 光标与视口移动 `body_h - 2` 行 | M1 |
| `Ctrl+Home` / `Ctrl+End` | 正文开头 / 结尾 | M1 |
| `Ctrl+S` | 保存（Untitled 时弹出路径提示） | M1 |
| `Ctrl+Q` | 退出（有未保存修改且不可自动保存时需再按一次确认） | M0 |
| `Ctrl+Z` | 撤销 | M2 |
| `Ctrl+Y` | 重做 | M2 |
| 粘贴（终端的 Cmd+V / Ctrl+Shift+V / 右键） | 作为一个 Transaction 插入 | M2 |
| `Ctrl+←` `Ctrl+→`、`Alt+B` `Alt+F` | 按词移动（后两者兼容 macOS 终端的 Option 键） | M3 |
| `Ctrl+Backspace`、`Ctrl+W` | 删除前一个词 | M3 |
| `Shift+方向键` / `Shift+Home/End` | 扩展选区 | M3 |
| `Ctrl+A` | 全选正文 | M3 |
| `Ctrl+C` / `Ctrl+X` | 复制 / 剪切选区（内部寄存器 + OSC 52） | M3 |
| `Ctrl+V` | 粘贴内部寄存器（系统剪贴板粘贴走终端 bracketed paste） | M3 |
| `Ctrl+T` | 切换 Typewriter 模式 | M4 |
| `Ctrl+F` | 切换 Focus 模式（非当前段落 DIM） | M4 |
| `Esc` | 取消选区 / 关闭提示行（`--vim` 下的含义见 §7.4） | M1 |

说明：crossterm raw mode 通过 `cfmakeraw` 关闭了 `ISIG` 与 `IXON`，因此 `Ctrl+Z` 不会挂起进程、`Ctrl+S` 不会触发 XOFF 流控，可安全占用。

### 7.3 最小可用键位集（M1 验收必须全部可用）

字符输入、`Enter`、`Backspace`、`Delete`、四个方向键、`Home/End`、`PageUp/PageDown`、`Ctrl+S`、`Ctrl+Q`、`Esc`。

剪贴板：M3 的复制使用 **OSC 52**（`ESC ] 52 ; c ; <base64> BEL`）写系统剪贴板，可穿透 SSH / tmux（需 `set -g set-clipboard on`）；不支持 OSC 52 的终端（macOS Terminal）复制仅进内部寄存器，status bar 提示一次。

### 7.4 `--vim` 模式键位（M4）

**状态**：`Mode::Insert` 是启动态，按键行为与 §7.2 完全相同；`Mode::Normal` 下按键见下表。Visual 不是第三个 Mode，而是 Normal 内部的子状态 `visual: bool`。它和待完成的按键序列（`g`、`d` 前缀）一起存放在 `Mode::Normal` 的载荷里：

```rust
pub enum Mode { Normal(NormalState), Insert }
pub struct NormalState { pub pending: Option<char>, pub visual: bool }   // pending ∈ {'g','d'}
```

**模式切换**（由 keymap 直接修改 `mode`；"返回"一栏是 map_event 的返回值）

| 当前 | 键 | 新状态 | 返回 Action |
|---|---|---|---|
| Insert | `Esc` | Normal | `Some(Cancel)`（顺带清掉选区；vim 回 Normal 时光标左移一格，这里不模拟） |
| Normal | `i` | Insert | `None` |
| Normal | `a` | Insert | `Some(Move(Right, false))` |
| Normal | `o` | Insert | `Some(OpenLineBelow)` |
| Normal | `v` | Normal(visual = !visual) | 进入时 `None`，退出时 `Some(Cancel)` |
| Normal(visual) | `Esc` | Normal(visual = false) | `Some(Cancel)` |
| Normal | `Esc` | 不变（清掉 pending） | `None` |

**Normal 态动作映射**（visual 为真时，所有 Motion 的 `extend = true`）

| 键 | Action / Motion |
|---|---|
| `h` `j` `k` `l` | `Move(Left/Down/Up/Right, visual)`。`j/k` 按视觉行移动，与方向键一致 |
| `w` / `b` / `e` | `Move(WordRight / WordLeft / WordEnd, visual)` |
| `0` / `$` | `Move(LineStart / LineEnd, visual)`，按视觉行，与 `Home/End` 一致 |
| `g` `g` / `G` | `Move(DocStart, visual)` / `Move(DocEnd, visual)` |
| `x` | 非 visual：`DeleteForward`；visual：`Cut` 并退出 visual |
| `d` `d` | 非 visual：`DeleteLine`（整行剪切进寄存器，行级） |
| `d`（visual 下） | `Cut` 并退出 visual |
| `y`（visual 下） | `Copy` 并退出 visual |
| `p` | `PasteRegister`（寄存器为行级时粘贴到下一行，否则粘贴到光标后） |
| `u` | `Undo` |
| `Ctrl+R` | `Redo`（仅 Normal 态绑定，与 vim 一致；Insert 态和默认模式下 `Ctrl+R` 不绑定，重做仍用 `Ctrl+Y`） |
| 方向键、`Home/End`、`PageUp/PageDown` | 与 §7.2 相同 |
| 其他可打印字符（含 IME 提交的文字） | 忽略，返回 `None`，status bar 不提示 |

`pending` 规则：`g`、`d` 之后若下一个键不能组成合法序列（例如 `gx`），则丢弃 pending，第二个键也不执行。不设超时。

**status bar 模式标识**：开启 vim 后，status bar 左段最前面显示 `NOR`（Normal）、`INS`（Insert）或 `VIS`（Normal 且 visual = true），后面跟一个空格，再是文件名（§9）。标识由 `App` 根据 `Mode` 生成，传给 `StatusModel`；不开启 vim 时不显示，也不占宽度。

**两种模式都生效**：所有 `Ctrl+` 组合键（`Ctrl+S/Q/Z/Y/C/X/V/T/F`）、bracketed paste、鼠标事件（开启 `mouse` 时：Normal 态拖拽出选区会同时把 `visual` 置为 true，之后可以直接用 `d`/`y`）。Normal 态收到 `Event::Paste` 时**仍然插入**，因为终端粘贴是用户的明确意图，不应被丢弃。

**冲突点说明**

1. **Esc**：`--vim` 下，Insert 态的 `Esc` 变成"回 Normal"（同时取消选区）；Normal 态的 `Esc` 只退出 visual、清掉 pending。默认模式下 §7.2 的"Esc 取消选区"语义不变。提示行（`保存为:` 等）打开时 `prompt_active = true`，`Esc` 一律只关闭提示行、不切换模式，优先级最高。
2. **u / p / x 与 Ctrl+Z / Ctrl+V 不冲突**：前者只是 Normal 态里的普通字符键，后者是 Ctrl 组合键，在两种模式下都绑定到同一组 Action（`Undo` / `PasteRegister`）。终端的系统粘贴（Cmd+V / Ctrl+Shift+V）走 `Event::Paste`，与 `p` 读取的内部寄存器是两条独立路径。`Ctrl+V` 在 Normal 态**不是**块选择。
3. **光标形状**：Normal 态 `SetCursorStyle::SteadyBlock`，Insert 态 `SteadyBar`。由 `App` 在模式变化时 `execute!`，渲染层不读 Mode。
4. **按键事件**：与 §7.1 一样只处理 `KeyEventKind::Press/Repeat`，所以 Windows 上 `dd` 不会因为同时上报 Release 而被识别成 4 次按键。

---

## 8. 文件与持久化

### 8.1 启动方式（唯一的"文件管理"）

| 命令 | 行为 |
|---|---|
| `mustty notes.md`（存在） | 打开 |
| `mustty notes.md`（不存在） | 新文档，路径记住；首次保存时创建（父目录不存在则报错，不自动 mkdir） |
| `mustty` | Untitled 文档；`Ctrl+S` 时底部提示行 `保存为: ` 输入路径（相对 cwd，不做 `~` 展开），目标已存在需再按一次 `Enter` 确认覆盖 |
| `mustty dir/` | 报错退出：`mustty 只编辑文件` |

不提供打开其他文件、最近文件、文件浏览器——要换文件就退出重开。

### 8.2 保存：原子写入

```text
fn save(doc: &Document, path: &Path) -> Result<SaveReport, IoError>
```

1. `serialize(doc)`：BOM? + 标题行 + sep + 正文；若 `meta.line_ending == CrLf` 则 `\n → \r\n`。
2. `NamedTempFile::new_in(path.parent())` 写入 → `sync_all()`。
3. Unix：复制原文件权限（`fs::metadata(path).permissions()`）到临时文件。
4. `persist(path)` 原子替换。
5. 记录保存时刻的 `revision`，`dirty = (doc.revision != saved_revision)`——undo 回到保存点时 dirty 自动消失。

否决"直接 `fs::write` 覆盖"：写到一半崩溃会截断用户文件。代价：会替换 inode（硬链接 / 部分同步盘会察觉），属于可接受的行业惯例（vim `backupcopy=no`、VS Code 同样如此）。

### 8.3 编码与换行符

- **只支持 UTF-8**。读入后 `String::from_utf8` 失败 → 拒绝打开，退出并提示 `不是 UTF-8 文件（第 N 字节）`。否决 `encoding_rs` 自动探测：GBK 误判会导致回写损坏，风险大于收益。
- **BOM**：读入时剥离并记录 `has_bom`，保存时原样写回。
- **换行符**：检测**第一个**换行是 `\r\n` 还是 `\n`，记为 `LineEnding`；内存统一 `\n`；保存时整体还原。混合换行的文件会被规范化为首个换行的风格（status bar 首次打开时提示一次）。新文件：Windows 上为 `CrLf`、其他平台 `Lf`。
- **文件末尾换行**：原样保留，不自动添加。
- 孤立 `\r`：按普通字符保留。

### 8.4 自动保存（默认开启）

- 条件：文档有路径 && `dirty` && 距最后一次编辑 **≥ 2 s 空闲**；另外在 `FocusLost`（`EnableFocusChange`）和退出时立即保存。
- 实现：事件循环 `poll` 的超时取 `min(autosave_deadline, message_expiry)`，无需线程（§11.3）。
- Untitled 文档不自动保存；退出时若有内容则要求确认。
- 外部修改检测：保存前比较文件 `mtime` 与上次读/写时记录的值，若不同 → **放弃本次自动保存**，status bar 常驻提示 `文件已被外部修改，Ctrl+S 强制覆盖`。
- 用配置 `autosave = false` 或 `--no-autosave` 关闭；空闲时长由 `autosave_idle_ms` 决定（默认 2000，§8.5）。
- 否决 swap 文件：增加清理与恢复 UI 的复杂度；有原子保存 + 2 s 自动保存，最坏丢 2 s 输入。

### 8.5 配置文件 `~/.mustty.toml`（M4）

**路径**：`dirs::home_dir()/.mustty.toml`，即 macOS/Linux 的 `~/.mustty.toml` 和 Windows 的 `%USERPROFILE%\.mustty.toml`。路径固定，不能通过命令行或环境变量更换。只读这一个文件，不按 XDG 或 AppData 路径查找，也不向上搜索项目目录。

**格式**：TOML，只有顶层键，没有表。示例：

```toml
width = 64
vim = true
code_color = "yellow"
```

**完整配置表**

| 键 | 类型 | 默认值 | 合法范围 / 取值 | 对应命令行 | 说明 |
|---|---|---|---|---|---|
| `width` | 整数 | `72` | `40..=120`，越界钳制 | `--width N` | 正文最大宽度 `MEASURE`（§5.1） |
| `vim` | 布尔 | `false` | — | `--vim` / `--no-vim` | 启用 vim-like 模式（§7.4） |
| `mouse` | 布尔 | `false` | — | `--mouse` / `--no-mouse` | 启用鼠标（§10.5） |
| `autosave` | 布尔 | `true` | — | `--autosave` / `--no-autosave` | 自动保存（§8.4） |
| `autosave_idle_ms` | 整数 | `2000` | `500..=60000`，越界钳制 | — | 停止输入多久后自动保存 |
| `typewriter` | 布尔 | `false` | — | — | Typewriter 模式的**初始状态**，运行中仍可用 `Ctrl+T` 切换（§4.4） |
| `focus_mode` | 布尔 | `false` | — | — | Focus 模式的**初始状态**，运行中仍可用 `Ctrl+F` 切换 |
| `scrolloff` | 整数 | `3` | `0..=10`，越界钳制；正文高度不足时仍按 §4.4 的 `(body_h - 1) / 2` 封顶；typewriter 开启时不生效 | — | 光标距视口上下边的最少行数 |
| `tab_width` | 整数 | `4` | `1..=16`，越界钳制 | — | `Tab` 键插入的空格数，以及文件中已有 `\t` 的显示宽度（§4.2） |
| `code_color` | 字符串 | `"cyan"` | 只接受 `black` `red` `green` `yellow` `blue` `magenta` `cyan` `white` 这 8 个标准 ANSI 名字（大小写不敏感），对应前景色 SGR 30–37。`gray` 作为 `white` 的别名同样接受（两者都映射到 ratatui 的 `Color::Gray`），其余名字（如 `grey`、`bright*`）一律非法。注意 ratatui 的命名有偏差：SGR 37 在 ratatui 里叫 `Color::Gray`，而 `Color::White` 是亮白 SGR 97。所以 `white` 映射到 `Color::Gray`，其余 7 个映射到 ratatui 的同名颜色 | — | 行内代码和 fenced code 的前景色（§6.1） |

`autosave_idle_ms` 和 `scrolloff` 的取值范围是本方案定的，其余范围来自需求。

**优先级**：命令行 > 配置文件 > 内置默认值，按键逐个合并：

```text
effective = Config::default().merge(file_layer).merge(cli_layer)   // 每层都是 PartialConfig（字段均为 Option）
```

**容错策略**（任何情况都不阻止启动）

| 情况 | 行为 |
|---|---|
| 文件不存在 | 静默使用默认值 |
| 文件存在但读不了（权限、不是 UTF-8） | 使用默认值，并显示启动警告 |
| TOML 语法错误 | 整个文件作废，使用默认值；启动警告带行号（来自 `toml::de::Error::span()`），例如 `~/.mustty.toml 第 3 行语法错误，已使用默认配置` |
| 某个键类型错误（如 `width = "wide"`）或 `code_color` 不是这 8 个名字 | 只有这个键回退到默认值，其他键照常生效，启动警告列出该键 |
| 数值越界 | 钳制到边界值，启动警告列出该键，例如 `width=200 → 120` |
| 未知键 | 忽略，不报错也不显示警告，只写一条 `tracing::debug!`（设置 `MUSTTY_LOG` 时可见） |

**启动警告的显示方式**：TUI 一启动就进入 alternate screen，所以 stderr 上的输出用户看不到。警告改为汇总成**一条** status bar 错误消息，保留到用户第一次按键（§9）；同时写一条 `tracing::warn!`。

**明确不做**：自定义键位、主题配色（除 `code_color` 外不开放任何颜色或样式）、项目级配置（如 `./.mustty.toml`）、status line 开关或布局定制、热重载（修改配置后需要重启 mustty）、写回配置（mustty 从不修改这个文件）、环境变量覆盖（`MUSTTY_LOG` 是日志开关，不属于配置）。

**模块与边界**：`config.rs` 产出不可变的 `Config`，`main.rs` 合并完成后交给 `App`。`Editor`、`Document` 不读取 `Config`：`tab_width` 由 `Keymap` 用来生成 `Insert(" ".repeat(n))`，由 `WrapCache` 用来计算 `\t` 宽度；`width`、`scrolloff`、`typewriter`、`code_color` 只交给 view 层。

---

## 9. status bar 内容定义

单行、宽度 = 正文宽度 `w`、整体 DIM（保存状态标记除外），左右两段：

```text
notes.md ●                                    1,284 字 · 12:7
└ 左段 ──┘                                    └ 右段 ────────┘
```

| 位置 | 内容 | 规则 |
|---|---|---|
| 左 | 文件名 | 仅 basename；Untitled 显示 `Untitled` |
| 左 | 保存状态标记 | 紧跟文件名，中间一个空格，计入左段宽度；**不 DIM**，与 DIM 的文件名形成对比。有未保存修改（无论有没有路径）→ `*`（U+002A），默认前景色；内容与磁盘一致（打开的已有文件未改动、`Ctrl+S` 或自动保存成功）→ `●`（U+25CF），`Color::Green`；无路径且从未修改（刚启动的空 Untitled）以及尚未创建的新文件 → 不显示。`●` 的 East Asian Width 为 Ambiguous，与 §14 风险 1 一致按 narrow（1 列）计算，不做终端探测；若渲染宽度有问题，备选字符为 `•`（U+2022）。保存成功**不显示**「已保存」类文案 |
| 右 | 字数 | `unicode_words().count()`（标题 + 正文）：每个汉字 / 假名计 1、英文按词计；千分位；随编辑重算（只在 revision 变化时） |
| 右 | 位置 | `行:列`，行 = 逻辑行号（1 起，标题不计），列 = 行内 grapheme 序号（1 起）；焦点在标题时显示 `标题` |
| 左（最前） | 模式（仅 `--vim`） | `NOR` / `INS` / `VIS`，没有加 `--vim` 时不显示（M4） |
| 覆盖 | 临时消息 | 错误 / 提示（外部修改、配置警告、保存失败、模式切换等）覆盖左段（连同保存状态标记），3 s 后消失（错误类消息保持到下一次按键） |
| 覆盖 | 提示行 | `保存为: ` / `未保存，再按 Ctrl+Q 放弃` 时整个 status bar 变为单行输入，光标在其中；保存状态标记随之隐藏，关闭后恢复 |

宽度不足时的裁剪优先级：先省略位置，再省略字数，文件名从左侧截断为 `…tes.md`；保存状态标记始终保留在截断后的文件名右侧（`…tes.md ●`）。

**启动提示**：打开后前 5 s 右段显示 `^S 保存  ^Q 退出`，之后恢复正常——不常驻帮助，保持界面干净。

---

## 10. 输入法（IME）与跨平台风险

### 10.1 中文输入的工作原理与结论

终端程序**拿不到预编辑（preedit）状态**：IME 由终端模拟器（或 OS 窗口）处理，预编辑文字由终端绘制在**硬件光标位置**，候选框也定位于此；用户确认后，提交的字符串以普通按键序列（多个 `KeyCode::Char`）送达程序。

由此得出三条硬性设计：

1. **必须使用硬件光标并在每帧精确放置**（§5.4），这是放弃 `ratatui-textarea` 自绘光标的主要原因。光标位置算错 1 列，候选框就会偏。
2. **不隐藏光标**；只在 `too_small` 时隐藏。
3. 一次提交的多个字符在同一批事件中到达 → 事件循环**先排空所有 pending 事件再重绘**（`while event::poll(Duration::ZERO)? { … }`），避免逐字重绘闪烁，并且把它们合并成同一个 undo Transaction（§4.5 的合并规则天然满足）。

### 10.2 bracketed paste

- Unix：启动时 `EnableBracketedPaste`，收到 `Event::Paste(String)` → `\r\n`、`\r` 归一化为 `\n` → 作为**单个 Transaction** 插入。避免粘贴大段文字时被当作逐字输入（触发 autoindent、产生几千个 undo 步）。
- Windows：crossterm 的 Windows 事件读取走 Console API，**不产生 `Event::Paste`**，粘贴表现为快速连续的按键事件。对策：同一批排空的事件中若连续插入 ≥ 16 个字符，视为粘贴，合并为一个 Transaction。

### 10.3 终端差异矩阵

| 终端 | IME 预编辑位置 | bracketed paste | DIM / ITALIC | 已知注意点 |
|---|---|---|---|---|
| **iTerm2** | 硬件光标处 inline | ✔ | ✔ / ✔ | 无 |
| **macOS Terminal.app** | 硬件光标处 | ✔ | ✔ / ✔ | 不支持 OSC 52；Option 键需开启 "Use Option as Meta" 才有 `Alt+B/F` |
| **WezTerm** | 硬件光标处（`use_ime = true`，默认开） | ✔ | ✔ / ✔ | 无 |
| **Windows Terminal** | 硬件光标处 | 无 `Event::Paste`（见 §10.2） | ✔ / ✔ | 必须过滤 `KeyEventKind::Release`；新文件默认 CRLF |
| **conhost（旧 cmd 窗口）** | 基本可用 | ✘ | DIM ✘ | 仅保证"能用"，不保证美观 |
| **GNOME Terminal / Kitty / Alacritty** | 硬件光标处 | ✔ | ✔ / ✔ | Alacritty 早期版本 IME 需较新版本 |
| **tmux 内** | 由外层终端决定 | ✔ | ✔ | 光标形状需 tmux `Ss/Se` terminal-overrides，否则保持块状（不影响使用） |

### 10.4 光标形状

启动 `SetCursorStyle::SteadyBar`（竖线，更像写作器），退出与 panic 时恢复 `DefaultUserShape`。不支持 DECSCUSR 的终端会忽略，无副作用。

### 10.5 鼠标：配置项 `mouse`，默认关闭（M4）

**默认关闭（`mouse = false`）**：行为与 M0–M3 完全一致，**不执行 `EnableMouseCapture`**。终端原生的拖选复制照常可用；滚轮在 alternate screen 下由多数终端转换成 `↑/↓` 键（alternate scroll mode），所以仍然能滚动。默认关闭的理由：用鼠标选一段文字复制出去，是写作者最常见的鼠标需求，而开启捕获后这项功能会失效。

**开启后（`mouse = true` 或 `--mouse`）**：启动时 `EnableMouseCapture`，鼠标事件由 `view::mouse::map_mouse` 转换成 Action。它不走 `Keymap`，因为坐标映射需要用到 Layout 和 Viewport。

| 鼠标事件 | 行为 |
|---|---|
| 左键单击正文 | 光标移到点击处：屏幕坐标 → `VisualPos`（`row = y - body.y - top_padding`，从 `ViewTop` 开始数视觉行；`col = x - body.x`）→ `WrapCache::visual_to_char`。点在行尾右侧时落到该视觉行行尾；点在宽字符第二格时吸附到左侧（与 §4.2 相同）。产出 `MoveTo(Target::Body(idx), false)`，有选区时清除选区 |
| 左键单击标题区 | 焦点切到标题，按列位置定位标题光标（扣除居中偏移），产出 `MoveTo(Target::Title(idx), false)` |
| 单击 status bar、左右留白、上下留白 | 忽略 |
| 左键拖拽 | 以按下位置为 anchor 扩展选区：`MoveTo(_, true)`。拖到正文上方或下方时**自动滚动**：`App` 记住最后一次拖拽坐标，并设置 50 ms 的 deadline（§11.3）；每到一次 deadline，视口滚动 1 行，光标跟随到新的边缘行，直到鼠标回到视口内或松开。crossterm 在鼠标静止时不会发送事件，所以只能用定时器驱动 |
| 左键松开 | 结束拖拽，取消自动滚动定时器 |
| 滚轮上 / 下 | `ScrollView(∓3)`：只移动视口，不改动文档。若光标因此离开 scrolloff 区间，就把它拉回可见区域的边缘行，保持 `preferred_col`。光标必须始终在屏幕上，否则 IME 候选框会无处锚定（§10.1） |
| 双击、右键、中键、修饰键 + 点击 | 不处理 |

提示行（`prompt_active`）打开时，所有鼠标事件都被忽略。

**代价（写进 README 和 `--help`）**：开启后，**终端原生的拖选复制失效**，因为鼠标事件全部交给 mustty 处理。需要原生选择时，**按住 Shift 再拖拽**即可绕过，这是 xterm、GNOME Terminal、Windows Terminal、WezTerm、Alacritty、kitty 等终端的通用约定。macOS 上 iTerm2 通常用 ⌥ Option，Terminal.app 用 Fn，具体以终端设置为准；手工清单（§13.3）会逐个核对。另外，mustty 内部选区的复制仍然走 `Ctrl+C` / OSC 52（§7.3）。

**恢复**：退出时 `DisableMouseCapture`；panic 时也必须执行（见 §10.6）。否则用户 shell 里会充满鼠标转义序列。

### 10.6 终端生命周期

```text
ratatui::init()                     // raw mode + alternate screen + panic hook
execute!(EnableBracketedPaste, EnableFocusChange, SetCursorStyle::SteadyBar)   // Windows 跳过 BracketedPaste
if config.mouse { execute!(EnableMouseCapture) }
… run …
if config.mouse { execute!(DisableMouseCapture) }
execute!(DisableBracketedPaste, DisableFocusChange, SetCursorStyle::DefaultUserShape)
ratatui::restore()
```

用 `TerminalGuard`（`Drop` 中执行恢复）保证 `?` 早退路径也能恢复。ratatui 的 panic hook 只负责恢复 raw mode 和 alternate screen，**不会关闭鼠标捕获和 bracketed paste**。因此 `TerminalGuard::enter` 会在 ratatui hook 之外再包一层自己的 hook：先 `execute!(DisableMouseCapture, DisableBracketedPaste, DisableFocusChange, SetCursorStyle::DefaultUserShape)`，再调用原 hook。所有 disable 命令都可以重复执行，没有副作用。

---

## 11. 模块划分

### 11.1 文件树

```text
mustty/
├── Cargo.toml
├── src/
│   ├── main.rs              语言初始化、CLI 解析(clap)、日志初始化、调用 app::run，错误转退出码
│   ├── lib.rs               re-export 各模块，供集成测试使用
│   ├── app.rs               App 状态机 + 事件循环 + Action 分发
│   ├── terminal.rs          TerminalGuard：进入/恢复终端模式
│   ├── action.rs            enum Action（与按键解耦的编辑语义）
│   ├── keymap.rs            Keymap、Mode：KeyEvent / Paste → Action（M4 的 --vim 状态机也只在此文件）
│   ├── editor/
│   │   ├── mod.rs           Editor：Document + Cursor + History + Focus 的组合与编辑 API
│   │   ├── document.rs      Document、TitleField、FileMeta、serialize/parse
│   │   ├── cursor.rs        Cursor、Affinity、grapheme 级移动
│   │   └── history.rs       Edit、Transaction、History 与合并规则
│   ├── text/
│   │   ├── grapheme.rs      边界与显示宽度工具函数
│   │   ├── wrap.rs          wrap_line、WrapCache、char_to_visual/visual_to_char
│   │   └── count.rs         字数统计
│   ├── markdown.rs          BlockState、Token、StyledRange、scan_line、HighlightCache
│   ├── view/
│   │   ├── layout.rs        compute_layout（纯函数）
│   │   ├── viewport.rs      ViewTop、scrolloff / typewriter 滚动
│   │   ├── theme.rs         Token → Style 映射（code_color 由此注入）
│   │   ├── mouse.rs         map_mouse：屏幕坐标 → Target / ScrollView（M4，仅 mouse = true 时调用）
│   │   └── widgets.rs       TitleWidget、BodyWidget、StatusWidget
│   ├── config.rs            PartialConfig / Config：读取 ~/.mustty.toml、逐键容错、钳制、三层合并（M4）
│   ├── status.rs            StatusModel：模式标识、文件名、dirty、消息队列、提示行状态
│   ├── i18n.rs              LOCALES 表、语言协商、t / t_args、缺 key 回退 en（§11.4）
│   └── io/
│       ├── file.rs          load / save（原子写、BOM、换行、mtime）
│       └── autosave.rs      AutosaveTimer（deadline 计算）
├── locales/
│   ├── en.toml              源语言 + 回退语言，扁平的 id = "文案"
│   └── zh-CN.toml           首份译文
└── tests/
    ├── file_roundtrip.rs
    └── render_snapshots.rs
```

### 11.2 关键签名草案（不含实现）

```rust
// editor/document.rs
pub enum LineEnding { Lf, CrLf }
pub struct FileMeta { pub path: Option<PathBuf>, pub line_ending: LineEnding, pub has_bom: bool, pub mtime: Option<SystemTime> }
pub enum TitleField { Unbound, Bound { text: String, sep: String } }
pub struct Document { pub title: TitleField, pub body: ropey::Rope, pub meta: FileMeta, revision: u64 }
impl Document {
    pub fn parse(raw: &str, meta: FileMeta) -> Self;        // raw 已去 BOM、已转 \n
    pub fn serialize(&self) -> String;                      // 含换行还原与 BOM
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Edit; // 正文唯一写入口，revision += 1
    pub fn set_title(&mut self, text: String);
    pub fn revision(&self) -> u64;
}

// editor/cursor.rs
pub enum Affinity { Upstream, Downstream }
pub struct Cursor { pub head: usize, pub anchor: Option<usize>, pub preferred_col: Option<u16>, pub affinity: Affinity }

// editor/history.rs
pub struct History { /* … */ }
impl History {
    pub fn record(&mut self, edit: Edit, before: &Cursor, after: &Cursor, now: Instant, kind: EditKind);
    pub fn seal(&mut self);                                  // 强制结束当前合并组
    pub fn undo(&mut self, doc: &mut Document) -> Option<Cursor>;
    pub fn redo(&mut self, doc: &mut Document) -> Option<Cursor>;
}

// editor/mod.rs
pub enum Focus { Title, Body }
pub struct Editor { pub doc: Document, pub cursor: Cursor, pub title_cursor: usize, pub focus: Focus, history: History, saved_revision: u64 }
impl Editor {
    pub fn apply(&mut self, action: &Action, wrap: &mut WrapCache, now: Instant) -> Effect;
    pub fn is_dirty(&self) -> bool;
}
pub enum Effect { None, Redraw, Save, SaveAs, Quit, Message(String) }

// text/wrap.rs
pub struct VisualRow { pub start: usize, pub end: usize, pub width: u16 }
pub struct VisualPos { pub line: usize, pub row: usize, pub col: u16 }
pub fn wrap_line(line: &str, width: u16, tab_width: u8) -> Vec<VisualRow>;   // tab_width 来自配置（§8.5）
pub struct WrapCache { /* … */ }
impl WrapCache {
    pub fn new(width: u16) -> Self;
    pub fn set_width(&mut self, width: u16);                 // 变化则全部失效
    pub fn invalidate(&mut self, lines: Range<usize>, inserted: isize);
    pub fn rows(&mut self, rope: &Rope, line: usize) -> &[VisualRow];
    pub fn char_to_visual(&mut self, rope: &Rope, idx: usize, aff: Affinity) -> VisualPos;
    pub fn visual_to_char(&mut self, rope: &Rope, pos: VisualPos) -> usize;
}

// text/grapheme.rs
pub fn prev_boundary(s: &str, char_idx: usize) -> usize;
pub fn next_boundary(s: &str, char_idx: usize) -> usize;
pub fn grapheme_width(g: &str, col: u16) -> u16;

// markdown.rs
pub enum BlockState { Normal, Fenced { fence: char, len: u8 } }
pub enum Token { HeadingMark, Heading(u8), EmphMark, Emph, StrongMark, Strong, CodeMark, Code, FenceLine, FencedCode, QuoteMark, Quote, ListMark, LinkPunct, LinkText, LinkUrl, Rule }
pub struct StyledRange { pub start: usize, pub end: usize, pub token: Token }
pub fn scan_line(line: &str, state_in: BlockState) -> (Vec<StyledRange>, BlockState);

// view/layout.rs
pub struct Layout { pub title: Option<Rect>, pub body: Rect, pub status: Option<Rect>, pub too_small: bool }
pub fn compute_layout(area: Rect, measure: u16) -> Layout;

// view/viewport.rs
pub struct ViewTop { pub line: usize, pub row: usize }
pub struct Viewport { pub top: ViewTop, pub top_padding: u16, pub typewriter: bool }
impl Viewport { pub fn follow(&mut self, cursor: VisualPos, body_h: u16, wrap: &mut WrapCache, rope: &Rope); }

// io/file.rs
#[derive(Debug)]                                             // Display 手写，文案经 i18n::t 取（§11.4）
pub enum IoError { NotUtf8 { offset: usize }, IsDirectory, ExternallyModified, Io(std::io::Error) }
pub fn load(path: &Path) -> Result<Document, IoError>;       // 不存在 → Ok(空文档 + path)
pub fn save(doc: &Document, path: &Path, check_mtime: bool) -> Result<SystemTime, IoError>;

// action.rs
// 与 Mode 无关；M1 时全部变体一次定稿，M4 不再增删
pub enum Action { Insert(String), Paste(String), Newline, DeleteBack, DeleteForward, DeleteWordBack,
                  Move(Motion, /*extend*/ bool), Undo, Redo, Save, Quit, Cancel, SelectAll, Copy, Cut,
                  PasteRegister,          // Ctrl+V / vim p：粘贴内部寄存器（行级寄存器粘贴到下一行）
                  DeleteLine,             // vim dd：剪切整个逻辑行进寄存器（行级）
                  OpenLineBelow,          // vim o：移到逻辑行尾并插入 \n，作为一个 Transaction
                  MoveTo(Target, /*extend*/ bool), // 鼠标单击 / 拖拽：直接定位到 char 索引（M3 随选区实现，M4 由 mouse 触发）
                  ScrollView(i32),        // 鼠标滚轮：由 App/Viewport 消费，不进入 Editor::apply
                  ToggleTypewriter, ToggleFocusMode, PromptInput(KeyEvent) }
pub enum Target { Body(usize), Title(usize) }   // 正文全局 char 索引 / 标题内 char 索引
pub enum Motion { Left, Right, Up, Down, LineStart, LineEnd, PageUp, PageDown, DocStart, DocEnd,
                  WordLeft,   // 上一个词首（Ctrl+← / vim b）
                  WordRight,  // 下一个词首（Ctrl+→ / vim w）
                  WordEnd }   // 当前或下一个词尾（仅 vim e 绑定）

// keymap.rs
pub enum Mode { Normal(NormalState), Insert }                     // 定义见 §7.4；M1–M3 恒为 Insert
pub struct NormalState { pub pending: Option<char>, pub visual: bool }
pub struct Keymap { pub vim: bool, pub tab_width: u8 }            // 均来自 Config；vim 在 M1–M3 恒为 false
impl Keymap {
    // 签名从 M1 起固定。mode 用 &mut，是为了让模式切换在 keymap 内部完成，不经过 Action
    pub fn map_event(&self, ev: &crossterm::event::Event, mode: &mut Mode, prompt_active: bool) -> Option<Action>;
}

// view/mouse.rs（M4）
pub struct DragState { pub last: Position, pub deadline: Option<Instant> }
pub fn map_mouse(ev: &crossterm::event::MouseEvent, layout: &Layout, vp: &Viewport,
                 wrap: &mut WrapCache, rope: &Rope, title: &str, drag: &mut Option<DragState>) -> Option<Action>;

// config.rs（M4）
#[derive(serde::Deserialize, Clone, Copy)] #[serde(rename_all = "lowercase")]
pub enum CodeColor { Black, Red, Green, Yellow, Blue, Magenta, Cyan, White }
#[derive(Default)]
pub struct PartialConfig { pub width: Option<u16>, pub vim: Option<bool>, pub mouse: Option<bool>, pub autosave: Option<bool>,
                           pub autosave_idle_ms: Option<u64>, pub typewriter: Option<bool>, pub focus_mode: Option<bool>,
                           pub scrolloff: Option<u16>, pub tab_width: Option<u8>, pub code_color: Option<CodeColor> }
pub struct Config { pub width: u16, pub vim: bool, pub mouse: bool, pub autosave: bool, pub autosave_idle_ms: u64,
                    pub typewriter: bool, pub focus_mode: bool, pub scrolloff: u16, pub tab_width: u8, pub code_color: CodeColor }
pub struct ConfigWarning { pub key: Option<&'static str>, pub message: String }
impl PartialConfig {
    pub fn from_toml_str(s: &str) -> (PartialConfig, Vec<ConfigWarning>);   // 永不失败：语法错误 → 空 Partial + 警告；未知键直接丢弃
}
pub fn load(cli: PartialConfig) -> (Config, Vec<ConfigWarning>);           // 读 ~/.mustty.toml；合并顺序：Default ← File ← Cli，然后钳制
```

### 11.3 事件循环（同步，单线程）

```text
loop {
    let timeout = app.next_deadline().map(|d| d.saturating_duration_since(now)); // autosave / 消息过期 / 拖拽自动滚动（50 ms）
    if event::poll(timeout.unwrap_or(Duration::MAX))? {
        loop { app.handle(event::read()?); if !event::poll(Duration::ZERO)? { break } }   // 排空批次
    }
    app.tick(Instant::now());            // 触发自动保存、消息过期
    if app.should_quit { break }
    if app.needs_redraw { terminal.draw(|f| view::render(f, &mut app))?; }
}
```

否决 `tokio` + `EventStream`：没有网络 / 并发 IO，异步运行时只增加编译时间和二进制体积。保存在主线程同步执行（典型文件 < 1 MB，fsync 数毫秒，用户无感）。

### 11.4 界面文案（i18n）

评估见 `docs/feasibility-i18n-and-settings-panel.md` 议题一；实现采用手写 TOML 表，与评估推荐的 fluent 不同。

- **文件**：`locales/<tag>.toml` 是扁平的 `id = "文案"` 表，`include_str!` 编译期嵌入。`en` 是源语言和回退语言，id 用英文 kebab-case。
- **API**：只有 `t("id") -> &'static str` 和 `t_args("id", &[("name", value)]) -> String`，没有宏。第一个参数必须是字符串字面量。占位符写成 `{name}`，做简单替换；缺参数时原样保留并 `debug_assert`。
- **复数**：只有英文字数一处，拆成 `status-words-one` / `status-words-other` 两条 key，不做复数规则。
- **回退**：当前语言缺 key 时取 `en`，每个 (语言, id) 只 `tracing::warn!` 一次；`en` 也没有则返回 id 本身。
- **协商**：依次取 `MUSTTY_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` 中第一个非空值，都没有时用 `sys-locale`，再没有用 `en`。环境变量必须排在前面，因为 macOS 的 `sys-locale` 返回 `zh-Hans-CN`，且忽略 `LANG`。
- **匹配**：先比较语言 + 地区，再只比较语言。`C` / `POSIX` → `en`。`zh` 的 script 为 `Hant` 或地区为 TW / HK / MO 时请求 `zh-TW`，目前没有这份译文，所以回退到 `zh-CN`。不设 `lang` 配置键。
- **初始化**：`i18n::init()` 在 `main()` 第一行调用，早于 clap 解析，因为 `--help` 也要翻译。clap 保留 derive，用 `Cli::command()` + `mut_arg` 在运行时填入 about / after_help / 各参数说明。clap 自带的 `Usage:` / `Options:` 和解析错误无法本地化。
- **测试**：
  - 每份 toml 都能解析，且与 `LOCALES` 表一一对应；
  - 译文的 key 必须是 `en` 的子集，占位符集合与 `en` 一致；
  - 扫描 `src/**/*.rs` 中非测试部分的 `t(` / `t_args(` 字面量，`en` 必须恰好覆盖这些 id；
  - 语言协商用表驱动测试。
  - 测试通过 `i18n::init_for_test(tag)` 按线程固定语言；快照按 `zh-CN` 录制。

---

## 12. 分层里程碑

每个里程碑结束时都是一个可以交给用户使用的二进制。

### M0 — 居中阅读器（read-only）

- 内容：`mustty FILE` 打开 UTF-8 文件；三区域布局（§5.2 全部退化级别）；软换行（§4.3）；硬件光标 + `↑↓←→`/`PageUp/PageDown` 浏览；scrolloff；resize；`Ctrl+Q` 退出；panic 恢复终端；标题区显示首行 H1 / 文件名；status bar 显示文件名 + `行:列`。
- 验收：
  1. 在 iTerm2 / Windows Terminal 打开一篇含中英混排、长 URL、emoji 的 3000 行 Markdown，视觉上居中、无越界、中文行不在 `，。` 前断开；
  2. 把终端从 200×60 拖到 80×24、60×10、20×4、8×3 再拖回，任何尺寸下不 panic、不残影，退回大尺寸后光标仍在原文字处；
  3. 用 debug 构建设置 `MUSTTY_DEBUG_PANIC=1`（按 `F12` 主动触发 panic，仅 debug 构建编译此分支）后，shell 恢复正常回显与光标，panic 信息打印在主屏幕；
  4. `cargo test` 通过 layout 与 wrap 单测。

### M1 — 可用的编辑器

- 内容：最小键位集（§7.3）；Unicode grapheme 级插入删除；CJK 光标正确；`Ctrl+S` 原子保存；Untitled 的 `保存为:` 提示行；dirty 标记；`Ctrl+Q` 未保存确认；BOM / CRLF / 末尾换行保真；标题区可编辑（§5.5）。`Action` / `Motion` 全部变体和 `Keymap::map_event(ev, mode, prompt_active)` 签名在本里程碑定稿：`Keymap.vim` 恒为 `false`，`mode` 恒为 `Insert`，不实现任何 Normal 态逻辑。
- 验收：
  1. 用 macOS 拼音、Windows 微软拼音分别输入 200 字中文，候选框始终贴在光标处，无重复字符、无丢字；
  2. 在 `e\u{301}`、`👨‍👩‍👧`、`🇨🇳` 上左右移动与 Backspace 各一步一个字形；
  3. 打开 CRLF + BOM 文件、不做修改保存，`cmp` 原文件与新文件一致；
  4. 保存过程中 `kill -9`（脚本循环）不出现截断文件；
  5. 能用 mustty 完整写完一篇 1000 字文章并保存（dogfood）。

### M2 — 不丢字

- 内容：undo/redo 与合并规则；bracketed paste（及 Windows 批量合并）；自动保存（2 s 空闲 / FocusLost / 退出）；外部修改检测；字数统计；`已保存` 提示；`--no-autosave`；`MUSTTY_LOG` 日志。
- 验收：
  1. 粘贴 5000 行文本 < 200 ms 显示完毕，一次 `Ctrl+Z` 全部撤销；
  2. 连续打 "hello world foo" 后 `Ctrl+Z` 三次依次撤回三个词；undo 回到保存点时 `•` 消失；
  3. 停止输入 2 s 后另一个终端 `cat` 文件可见新内容；切走窗口（FocusLost）立即保存；
  4. 用另一个编辑器修改同一文件后，mustty 不再自动覆盖并提示。

### M3 — Markdown 写作体验

- 内容：标记变暗高亮（§6）与增量缓存；按词移动 / 删除；选区（Shift+移动、`Ctrl+A`）；`Ctrl+C/X/V` 内部寄存器 + OSC 52；视觉行 `Home/End` 的 affinity 细节。另外实现并单测为 M4 预留的、与模式无关的内核能力：`Motion::WordEnd`、`Action::DeleteLine`、`Action::OpenLineBelow`、行级 / 字符级寄存器与 `PasteRegister`，以及为鼠标预留的 `MoveTo(Target, extend)`。默认键位只绑定 `PasteRegister`（`Ctrl+V`），其余几个此时没有绑定按键。**不实现任何 modal 逻辑。**
- 验收：
  1. `insta` 快照覆盖 §6.1 表中每一种元素以及未闭合标记；
  2. 输入 ```` ``` ```` 开启代码块时，只有其后的行变色，在 1 万行文档中输入延迟无可感知变化（< 16 ms/帧，`MUSTTY_LOG` 中记录帧耗时）；
  3. iTerm2 / WezTerm 中 `Ctrl+C` 选区后可在其他应用粘贴；tmux + SSH 下同样可用。

### M4 — 打磨与发布

- 内容：Typewriter 模式（`Ctrl+T`）；Focus 模式（`Ctrl+F`，当前段落外 DIM）；`--width`；**`--vim` 模式（§7.4）**：`keymap.rs` 中的 Normal/Insert 状态机、visual 子状态、`gg`/`dd` 序列、Normal 态 `Ctrl+R` 重做、status bar 的 `NOR/INS/VIS` 标识、按模式切换光标形状；**配置文件 `~/.mustty.toml`（§8.5）**：`config.rs` 实现 10 个键、逐键容错、钳制、命令行 > 配置 > 默认的合并，CLI 增加 `--vim/--no-vim`、`--mouse/--no-mouse`、`--autosave`（与已有的 `--width`、`--no-autosave` 一起，共四个覆盖项），`tab_width` 接入 Keymap 和 WrapCache，`code_color` 接入 theme，`typewriter` / `focus_mode` 作为初始状态；**鼠标（§10.5）**：`view/mouse.rs`，`mouse = true` 时启用单击定位、拖拽选区（出视口后以 50 ms 间隔自动滚动）、滚轮滚动，退出和 panic 时 `DisableMouseCapture`；README 写明"开启鼠标后，原生选择需按住 Shift"；启动 5 s 键位提示；GitHub Actions 三平台 CI（test + clippy + release 构建）；macOS/Linux/Windows 预编译二进制；README。
- 验收：
  1. Typewriter 模式下连续输入 100 行，光标行始终在正文中线；
  2. CI 三平台全绿，Release 附带 4 个二进制，`mustty --version` 正确；
  3. 按 §13.3 手工清单在 6 个终端全部打勾；
  4. **`--vim` 内核零改动**：M4 中加入 `--vim` 的那组提交，`git diff --stat` 只涉及 `keymap.rs`、`app.rs`（持有 Mode、切换光标形状）、`status.rs`、`main.rs`（clap 参数），以及测试文件和文档，`editor/`、`text/`、`markdown.rs`、`io/` 中没有任何改动；
  5. **`--vim` 功能**：带 `--vim` 启动后，依次执行 `i` 输入 → `Esc` → `gg` → `dd` → `G` → `p`，结果是第一行被移到文末；`v` + `w` + `d` 删除一个词；`u` 与 `Ctrl+Z` 效果相同；status bar 正确显示 `INS/NOR/VIS`，光标在块状与竖线之间切换；
  6. **默认行为不回退**：不带 `--vim` 时，M1–M3 的全部验收项重跑通过，`Esc` 仍然只取消选区；
  7. `--vim` 下 Normal 态按 `j`，在 macOS 拼音、Windows 微软拼音"英文模式"下都正常移动。中文模式下的行为按 §14 风险 2 处理，并写进 README；Normal 态 `Ctrl+R` 能撤回 `u`；
  8. **配置**：在 200×50 的终端里，分别准备 6 份 `~/.mustty.toml` 启动，用界面上能直接看到的行为验证。宽度的测法：打开一行 200 个 `x` 的文件，数第一个视觉行有几个 `x`，或者看光标在 `End` 后的列号减去起始列：
     ① 文件不存在 → 正文实测 72 列宽，status bar 没有警告，不显示 `NOR/INS` 标识，`Tab` 插入 4 个空格；
     ② 10 个键全部写上合法值（`width = 64`、`vim = true`、`mouse = true`、`autosave = false`、`autosave_idle_ms = 5000`、`typewriter = true`、`focus_mode = true`、`scrolloff = 5`、`tab_width = 2`、`code_color = "yellow"`）→ 正文实测 64 列宽；status bar 显示 `INS`；单击可定位光标；停止输入 10 s 后文件仍未写盘；光标行固定在正文中线；非当前段落为 DIM；`Tab` 插入 2 个空格；行内代码显示为黄色；
     ③ 配置 `width = 64`、`vim = true`，启动时带 `--width 90 --no-vim` → 正文实测 90 列宽，status bar 不显示模式标识，`Esc` 只取消选区；
     ④ TOML 语法错误 → TUI 仍能正常启动，正文 72 列宽，status bar 显示含行号的警告（例如 `~/.mustty.toml 第 3 行语法错误，已使用默认配置`），按任意键后警告消失；
     ⑤ `width = 200`、`tab_width = 0`、`code_color = "orange"` → 正文实测 120 列宽，`Tab` 插入 1 个空格，行内代码为默认的青色，status bar 显示一条汇总警告，列出这 3 个键；
     ⑥ 写入未知键 `colour = "red"` 和 `width = 64` → 启动时没有任何警告，正文实测 64 列宽（未知键不影响其他键）；
  9. **鼠标关闭（默认）**：与 M3 的行为逐项一致。可用 `script` 录制启动时输出的转义序列来验证：其中不出现 `\x1b[?1000h` / `\x1b[?1006h`；原生拖选复制可用；
  10. **鼠标开启**：单击中文行中任意一个字的左半或右半格，光标都落在该字前面或后面的正确边界；拖拽选中跨 3 行的文字后 `Ctrl+X` 剪切正确；拖到正文下方并保持不动，视口持续滚动，松开后停止；滚轮滚动 30 行后，光标仍然可见且 IME 候选框跟随；按住 Shift（iTerm2 为 ⌥）可以原生选择；
  11. **恢复**：在 `mouse = true` 的 debug 构建中触发 panic（§12 M0 验收 3），退出后在 shell 里移动、点击鼠标，不会出现转义字符乱码。

---

## 13. 测试策略

### 13.1 纯逻辑单测（`cargo test`，不依赖终端）

| 模块 | 测试内容 | 方式 |
|---|---|---|
| `view::layout` | §5.2 每个退化级别的边界值：`W ∈ {8,9,10,23,24,75,76,200}`、`H ∈ {1,2,4,5,11,12,24,60}`；断言 x 居中、rect 不越界、`body_h ≥ 1` | 表驱动 |
| `text::wrap` | 纯英文、纯中文、中英混排、避头尾、长 URL 强制断行、悬挂空格、Tab、emoji ZWJ、空行、`width = 1` | 表驱动 + `proptest` 性质：① 各 row 区间首尾相接覆盖整行；② 每个 row 的 `width ≤ 宽度`，除非该 row 只有一个 grapheme；③ 断点总在 grapheme 边界 |
| 光标映射 | `visual_to_char(char_to_visual(i)) == i` 对所有 grapheme 边界成立；宽字符中间列吸附左侧；上下移动保持 preferred_col | `proptest`（随机中英混排串 × 随机宽度 10..80） |
| `editor::cursor` | grapheme 移动、Backspace 在组合字符 / 旗帜 / ZWJ 上 | 表驱动 |
| `editor::history` | 合并规则（按词、1 s 间隔、方向改变、换行、粘贴）；undo 到保存点 dirty 消失；标题编辑可撤销 | 表驱动，`Instant` 通过参数注入 |
| `editor::document` | H1 识别规则全部分支；`parse → serialize` 字节级往返（LF/CRLF/BOM/无末尾换行/只有标题/front matter） | 表驱动 + `proptest`（任意 UTF-8 文本往返不变） |
| `io::file` | 原子保存后内容正确、权限保留（Unix）、非 UTF-8 拒绝、目录拒绝、mtime 冲突检测 | `tempfile` 目录中的集成测试 |
| `markdown` | 每种 Token、未闭合、`snake_case` 不误判、fenced 状态传递 | `insta` 快照 |
| `text::count` | 中文 / 英文 / 混排 / 标点不计 | 表驱动 |
| `keymap`（M1 起） | `vim = false`：任何事件都不改变 `mode`（恒为 Insert），`Esc` → `Cancel`，§7.2 每个键 → 预期 Action | 表驱动：`(Event, Mode, prompt_active) → (Option<Action>, Mode)` |
| `keymap` mode 分支（M4） | Insert `Esc` → Normal + `Cancel`；`i/a/o` 进入 Insert 及其返回值；§7.4 映射表逐项覆盖；`v` 下 Motion 的 `extend = true`，`d/y/x` 后退出 visual；`gg`、`dd` 序列和非法序列（`gx`）丢弃 pending；`prompt_active = true` 时 `Esc` 不切换模式；Normal 态可打印字符返回 `None`；Normal 态 `Event::Paste` 仍然返回 `Paste`；Ctrl 组合键在两种模式下返回值相同 | 表驱动，同上；另用 `proptest` 生成随机按键序列，断言 `map_event` 不 panic，且 `vim = false` 时 mode 永远是 Insert |
| `keymap` vim 键位表（M4） | 把 §7.4 的两张表原样写成一个 `const VIM_TABLE: &[(&str /*按键序列*/, ModeBefore, Option<Action>, ModeAfter)]`，测试逐行回放，保证文档和实现一一对应。新增或修改键位时，必须同时改表和文档。另外单独覆盖：`Ctrl+R` 只在 Normal 态返回 `Redo`；VIS 状态下 `Esc` 返回 `Cancel` 并回到 NOR；`StatusModel` 在三种状态下渲染出 `NOR`/`INS`/`VIS`，`vim = false` 时不渲染 | 表驱动 |
| `config`（M4） | ① 空字符串 / 文件不存在 → `Config::default()` 且没有警告；② 10 个键全部合法 → 逐项相等；③ 每个数值键在边界值 −1、边界、边界 +1 上的钳制（`width` 39/40/120/121，`tab_width` 0/1/16/17，`autosave_idle_ms` 499/60001，`scrolloff` 11）；④ `code_color` 的 8 个合法名字、大小写变体和非法名字；⑤ 单个键类型错误时，只有该键回退；⑥ 语法错误时返回空 Partial，警告带行号；⑦ 未知键被忽略，不产生警告，也不影响其他键；⑧ 三层合并的优先级（default ← file ← cli），包括 `--no-vim`、`--no-mouse`、`--no-autosave` 能覆盖配置文件里的 `true`，以及同一开关出现两次时后者生效（`--vim --no-vim` 得到 false） | 表驱动 + `proptest`（随机 TOML 片段，断言 `from_toml_str` 永不 panic） |
| `view::mouse`（M4） | 屏幕坐标 → Target：正文首行 / 末行、`top_padding > 0`（typewriter）、软换行的第 2 个视觉行、行尾右侧空白、宽字符左右两格、悬挂空白处、标题区（居中偏移）、status bar 和留白（返回 `None`）；拖拽时 `extend = true`，越过上下边界设置 50 ms deadline，回到视口内或松开后清除；滚轮返回 `ScrollView(±3)`；`prompt_active` 时一律返回 `None` | 表驱动：构造固定的 `Layout` + `Viewport` + 文本；另用 `proptest` 验证"对正文内任意坐标，`map_mouse` 得到的 idx，经 `char_to_visual` 后回到同一视觉行，列偏差 < 该处 grapheme 的宽度" |
| `editor`：M4 预留动作（M3 起） | `DeleteLine` 在首行 / 末行 / 无末尾换行时的结果与寄存器内容；`PasteRegister` 行级与字符级；`OpenLineBelow` 是一次 undo；`WordEnd` 在中英混排中的落点 | 表驱动 |

### 13.2 渲染快照（无真实终端）

用 `ratatui::backend::TestBackend` 以固定尺寸（`80×24`、`120×40`、`40×10`）调用完整 `render`，把 `Buffer` 与光标位置写入 `insta` 快照，覆盖：空文档、长文档滚动到中部、CJK 内容、Unbound 标题占位、status bar 消息覆盖、提示行、`too_small`。用于防止布局回归。

### 13.3 手工验证清单（每个里程碑发布前执行）

终端：iTerm2、macOS Terminal、WezTerm、Windows Terminal、GNOME Terminal、tmux（在 iTerm2 内）。每个终端逐项打勾：

1. 启动居中、无背景色块、光标为竖线；
2. 中文 IME 输入 2 句话，候选框贴光标；
3. 粘贴 100 行（含中文与空行）后一次撤销；
4. 拖拽 resize 到极小再恢复；
5. `Ctrl+S`、`Ctrl+Z`、`Ctrl+Q` 均被程序接收（未被终端或 shell 截获）；
6. DIM / ITALIC 标记变暗可辨识；
7. 退出后 shell 光标形状、回显、换行恢复正常；
8. （M4 起）`mouse = true` 时：单击定位、拖拽选区、滚轮滚动都可用；记录该终端绕过鼠标捕获、进行原生选择所需的修饰键（Shift / ⌥ / Fn），并同步到 README；`mouse = false` 时原生拖选复制可用。

结果记录在 `docs/manual-qa.md`（每个里程碑一张表）。

---

## 14. 风险与未决问题

| # | 风险 | 推荐默认值 |
|---|---|---|
| 1 | **字符宽度与终端不一致**：① East Asian Ambiguous 字符（`“”‘’…—`）在部分 CJK 用户的终端里被设成宽字符（iTerm2 "Ambiguous characters are double-width"）；② Emoji / ZWJ 序列（如 `👨‍👩‍👧`）有的终端画 2 列、有的画 6 列。两者都会让该行后续光标错位 | 一律采用 unicode-width 0.2.2 的结果：ambiguous 按 **narrow**，与 ratatui 和绝大多数终端的默认值一致；不做终端探测。M4 之后视反馈再加 `--ambiguous-wide`，因为要同时替换 ratatui 写 Buffer 的宽度逻辑，代价较大，所以不提前做 |
| 2 | **`--vim` 下 Normal 态与中文 IME 冲突**：输入法处于中文状态时，`j`/`k`/`d` 等按键会被 IME 截获成拼音，程序收不到；IME 状态对终端程序不可见，无法自动切换 | 不尝试切换或探测 IME。Normal 态收到的可打印字符（包括 IME 提交的汉字）一律忽略，不会误插入；README 建议 vim 用户为终端单独设置英文输入源，或用 `Esc` 前先切到英文。这只影响主动加 `--vim` 的用户，默认模式不受影响 |
| 3 | **自动保存覆盖用户不想保存的修改** | 默认 **开启**（2 s 空闲）；保留完整 undo 历史作为后悔药；提供 `--no-autosave`；不做 swap / 备份文件 |
| 4 | **标题绑定 H1 的回写边界**（setext 标题 `===`、front matter、`# 标题 #`）可能让用户困惑"为什么这篇没有标题" | 只识别首行 `"# "` 且不以 `#` 结尾的 ATX 标题；其余情况 Unbound，标题区以 DIM 显示文件名作为提示；front matter 支持推迟到 M4 之后 |
| 5 | **配置文件带来的不确定性**：① 非法值（越界、类型错、`code_color` 拼错）；② TOML 语法错误；③ 配置项变多后，同一个二进制在不同机器上行为不同，bug 报告难以复现（"我这里 Esc 不取消选区"其实是对方开了 `vim = true`） | ① 逐键回退默认或钳制，汇总成一条 status bar 警告，**永不阻止启动**；② 整个文件回退默认，并给出带行号的警告；③ 配置项**冻结为 10 个**，新增需要走方案评审；不加诊断类命令行参数。issue 模板要求附上 `~/.mustty.toml` 的内容和启动命令行；`MUSTTY_LOG` 日志的第一行写入最终生效的配置；需要纯默认行为来复现问题时，临时改名 `~/.mustty.toml` 即可 |
