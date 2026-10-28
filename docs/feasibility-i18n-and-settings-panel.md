# 可行性评估：多语言（i18n）与配置面板

> 状态：评估结论，未改代码。基线：M4 完成后的 `main`（`9ec2db6`）。
> 依据：`docs/implementation-plan.md` §7 / §8.5 / §9 / §11，`src/{config,keymap,app,status,main,terminal}.rs`、`src/view/*`、`src/text/wrap.rs`。
> 数据：crates.io API（2026-10-28 查询）；体积与依赖数为本机实测（macOS arm64，`--release`，`strip = true`，最小样例程序，见附录 A）。

---

## 议题一：多语言

### 1.0 先校正现状盘点

需求里写的是"17 条字符串，常驻 UI 只有 2 条"。按源码逐条数，实际数量比这个多，而且**全部是中文**：

| 类别 | 条数 | 位置 |
|---|---|---|
| 常驻 UI | 6 | `{n} 字`、`Untitled`（`status.rs` 和 `app.rs::placeholder` 各一处）、`标题`（位置栏）、`窗口太小`、启动提示 `^S 保存  ^Q 退出`（这条显示 5 s，但每次启动都会出现） |
| 提示行 | 2 | `保存为: `、`文件已存在，Enter 覆盖: ` |
| 临时消息 | 8 | Typewriter / Focus 开关（含 `开`/`关`）、`未保存，再按 Ctrl+Q 放弃`、OSC 52 提示、5 MB 大文件提示、`保存失败：{e}`、`IoError` 的 3 个 `#[error]`、`无法读取保存后的文件信息` |
| 配置警告 | 7 | `config.rs` 中语法错误、应为整数、应为布尔、`code_color` 无效、无法读取、`配置已修正：`、分隔符 `；` |
| 启动错误（stderr） | 2 | `无法打开日志文件`、`无法打开 {path}` |
| clap `--help` | 9 | `about`、`after_help`、7 个参数说明 |

合计约 **34 条**（其中带参数的 11 条），复数只有英文的 `word/words` 一处。数量级仍然是"几十条"，不影响结论，但迁移成本要按 34 条估算。

更重要的一点是：**目前 UI 只有中文**。开源后，大多数潜在用户和贡献者第一次运行时就会被语言挡住。所以 i18n 不只是"方便贡献翻译"，而是开源前的必要工作。

### 1.1 调研表

| 方案 | 最新版本 | 最近发布 | 近 90 天下载 | 许可证 | 文件格式 | 加载方式 | 相对 hello world 的体积增量 | 相对 mustty 现有依赖新增 crate 数 |
|---|---|---|---|---|---|---|---|---|
| **fluent-bundle**（直接用） | 0.16.0 | 2025-05 | 800 万 | Apache-2.0 OR MIT | `.ftl` | 由自己决定（`include_str!` 编译期嵌入） | **+257 KB** | **13** |
| fluent（重导出 fluent-bundle） | 0.17.0 | 2025-05 | 670 万 | Apache-2.0 OR MIT | `.ftl` | 同上 | 同上 | 同上 |
| fluent-templates | 0.15.1 | 2026-08 | 25 万 | MIT/Apache-2.0 | `.ftl` | `static_loader!` 宏，编译期嵌入 | +304 KB | 29 |
| i18n-embed + i18n-embed-fl + rust-embed | 0.16.0 / 0.10.1 / 8.x | 2025-07 / 2026-07 | 430 万 | MIT | `.ftl` + `i18n.toml` | rust-embed 嵌入，运行时按语言请求选择；`fl!` 编译期校验 message id | +356 KB | 40（其中 24 个 proc-macro 依赖） |
| rust-i18n | 4.2.4 | 2026-09 | 158 万 | MIT | YAML / TOML / JSON | `i18n!` 宏，编译期嵌入 | **+57 KB** | 37（globwalk、ignore、serde_json、regex 系、toml_edit 等，均为构建期依赖） |
| gettext-rs | 0.8.0 | 2026-08 | 66 万 | MIT | `.po` → `.mo` | 运行时 `bindtextdomain` 从磁盘读取 | 未测（链接 libintl） | 依赖 `gettext-sys`；没有系统 libintl 时会从源码编译 GNU gettext |
| gettext（纯 Rust 读 `.mo`） | 0.4.0 | **2019-06** | 47 万 | MIT | `.mo` | 运行时读文件 | — | 已停止维护 |
| tr | 0.1.11 | 2025-06 | 21 万 | MIT | `.po` | gettext 风格 | — | 生态很小 |
| es-fluent / fluent-static | 0.21.1 / 0.6.0 | 2026-10 / 2026-08 | 5 千 / 1 万 | MIT(/Apache) | `.ftl` | derive / 代码生成 | — | 太年轻，API 仍在变 |
| 辅助：sys-locale | 0.3.2 | 2024-11 | 1338 万 | MIT OR Apache-2.0 | — | 读取系统语言 | ≈ 0 | 1 |

参照：mustty 当前 release 二进制约 3.16 MB（未 strip）。fluent-bundle 的 +257 KB 约占 8%。

### 1.2 逐维度对比

**贡献者加一门语言要做什么**

| 方案 | 步骤 | 需要重新编译 |
|---|---|---|
| fluent-bundle（推荐的写法，见 1.3） | 复制 `locales/en/mustty.ftl` 为 `locales/<lang>/mustty.ftl` 并翻译；在 `src/i18n.rs` 的 `LOCALES` 表里加一行 | 是 |
| fluent-templates | 新建 `locales/<lang>/` 目录即可，宏会扫描目录 | 是 |
| i18n-embed | 新建 `i18n/<lang>/mustty.ftl` | 是（嵌入模式下） |
| rust-i18n | 在同一个 YAML 文件的每个 key 下加一行 `<lang>: ...`，或者新增一个按语言拆分的文件 | 是 |
| gettext-rs | 用 `msginit` 生成 `.po`、翻译、用 `msgfmt` 编译成 `.mo`、安装到 `share/locale/<lang>/LC_MESSAGES/` | 否，但需要随包分发和安装文件 |

`.ftl` 和 `.po` 都受 Weblate / Pontoon / Crowdin 原生支持，贡献者不需要了解 Rust。rust-i18n 的 v2 YAML 格式把所有语言放在同一个 key 下面，多个人同时翻译不同语言时容易在同一个文件里冲突。

**编译期嵌入 vs 运行时加载**：选编译期嵌入。mustty 的分发方式是 `cargo install`、Homebrew 和 GitHub release 的单个二进制，运行时加载需要解决"翻译文件放在哪"的问题（XDG / Windows / brew prefix 各不相同），而 34 条字符串、每门语言约 3 KB 文本，嵌入的代价可以忽略。翻译者验证译文本来就要跑一次 `cargo run`，需要重新编译不是障碍。

**缺翻译时的回退**：fluent-bundle 自己负责解析和格式化，回退链由调用方实现，写法完全由我们控制：请求的语言 → `en` → message id 本身，同时 `debug_assert!` 并写一条 `tracing::warn!`。rust-i18n 缺 key 时直接返回 key 字符串（实测：key 写错时输出 `words`），没有中间层可以插入日志。fluent-templates 和 i18n-embed 内置了回退，但都是"语言级"的回退。

**与 clap `--help` 整合**：derive 的 doc comment 是编译期 `&'static str`，不能直接接 i18n。可行的做法是：保留 `#[derive(Parser)]`，运行时用 `Cli::command().about(t("cli-about")).mut_arg("width", |a| a.help(t("cli-width")))…` 覆盖每段说明（clap 4 的 `help()` 接受 `String`），然后 `Cli::from_arg_matches(&cmd.get_matches())`，大约 15 行。clap 自带的骨架文案（`Usage:`、`Options:`、`Print help`）和解析错误（`error: unexpected argument`）**无法本地化**，clap 不提供这个能力。所有方案在这一点上没有区别。

**二进制体积 / 依赖树**：见 1.1 的表。rust-i18n 体积最小，因为它在构建期就把 YAML 展开成 match，但构建期依赖最重（37 个，包括文件遍历和 regex）。fluent-bundle 运行时依赖最少，而且全部来自 projectfluent / unicode 生态。

### 1.3 推荐：直接用 `fluent-bundle` + `include_str!` 嵌入 + `sys-locale`，自己写约 120 行的 `src/i18n.rs`

```text
locales/
  en/mustty.ftl        # 源语言，也是回退语言；message id 用英文 kebab-case
  zh-CN/mustty.ftl
src/i18n.rs            # LOCALES 表 + 语言协商 + t()/t_args() + 回退
```

关键决策：

1. **源语言和回退语言是 `en`**。现在的中文文案移到 `zh-CN/mustty.ftl`，作为第一份译文。message id 用英文（`status-words`、`prompt-save-as`），不要用中文原文当 key。
2. **`bundle.set_use_isolating(false)`**。Fluent 默认在每个变量两侧插入 Unicode 双向隔离符 U+2068 / U+2069（实测输出 `⁨1⁩ word`）。在终端里，这两个字符会被 `unicode-width` 计为 0 宽，但有些终端会画成方框，还会打乱 status bar 的宽度计算。mustty 不支持 RTL，直接关闭。
3. **语言协商自己写，不依赖 `fluent-langneg` 的默认行为**。优先级：`MUSTTY_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `sys-locale`。有两个实测发现：
   - macOS 上 `sys-locale` 返回 `zh-Hans-CN`（带 script 子标签），而且**完全忽略 `LANG` / `LC_ALL`**（实测设置 `LC_ALL=fr_FR.UTF-8` 后返回值不变）。所以必须先读环境变量，否则用户没法临时切换语言测试。
   - 匹配规则：先比较 language + region，再只比较 language。`zh` 单独处理：script 为 `Hant` 或 region 为 TW/HK/MO 时映射到 `zh-TW`（以后有人贡献时启用），其余映射到 `zh-CN`。`C` / `POSIX` 映射到 `en`。
   - **不新增 `lang` 配置键**。环境变量已经够用，配置面板也不需要多一个选项。
4. **API 只有两个函数**：`t(id: &'static str) -> Cow<str>` 和 `t_args(id, &[(&str, FluentValue)]) -> String`。bundle 放在 `OnceLock` 里，`main()` 的第一行完成初始化（clap 解析之前，因为 `--help` 也需要它）。不写宏：34 条字符串用不到宏带来的好处，函数也更容易让新贡献者读懂。
5. **用测试代替编译期检查**（这正是 i18n-embed `fl!` 的卖点，这里用约 40 行测试实现）：
   - 每个 `.ftl` 都能解析，没有 `FluentError`；
   - 每份译文里的 id 都是 `en` 的子集，不允许出现孤儿 key；
   - 每条消息的变量集合与 `en` 一致（防止译者把 `{ $n }` 拼错）；
   - 扫描 `src/**/*.rs` 里 `t("…")` / `t_args("…"` 的字面量，确认它们都存在于 `en`。
6. `IoError` 的 `#[error("…")]` 改成 `Display` 手写实现，调用 `t()`。`thiserror` 的 attribute 只接受字面量。

**为什么不选更轻的 rust-i18n**：它的体积最小，但使用的是项目私有的 YAML 约定（`_version: 2`、`%{n}` 占位符），不是"标准方案"；复数要靠 `t!` 外面手写分支；缺 key 时静默返回 key；构建期依赖多 37 个 crate，比 fluent-bundle 还多。用户要求的是"更标准、方便贡献者"，Fluent 在格式标准化和翻译平台支持上明显更好。

**为什么不选更重的 i18n-embed / fluent-templates**：它们解决的问题（运行时资源发现、多 crate 共享翻译、桌面语言请求、目录扫描宏）在 34 条字符串、单个 crate、单个二进制的场景下都不存在。代价是多 27–40 个依赖、多 50–100 KB，还多一个 `i18n.toml` 配置文件。它们省下的只是自己写的 120 行，而这 120 行全部可测试，也完全可控（特别是第 2、3 条，现成的封装里不一定方便定制）。

**为什么不选 gettext-rs**：Windows 上没有系统 libintl，`gettext-sys` 会从源码编译 GNU gettext，需要 MSYS 或 autotools，MSVC 工具链下经常失败。运行时还要从磁盘查找 `.mo`，和单二进制分发冲突。纯 Rust 的 `gettext` crate 从 2019 年起就没有再发布。

### 1.4 迁移成本

| 项 | 规模 |
|---|---|
| 新依赖 | `fluent-bundle = "0.16"`、`unic-langid = "0.9"`（不开 `macros` feature）、`sys-locale = "0.3"` |
| 新文件 | `src/i18n.rs`（约 120 行 + 约 80 行测试）、`locales/en/mustty.ftl`、`locales/zh-CN/mustty.ftl`（各约 50 行） |
| 改动 | 34 处字符串替换，涉及 `status.rs`、`app.rs`、`config.rs`、`view/mod.rs`、`io/file.rs`、`main.rs`，约 80 行 diff；`main.rs` 的 clap 改成 `command()` + `mut_arg`，约 20 行 |
| 测试改动 | 现有测试断言中文文案的地方（`config.rs` 的 `第 3 行语法错误`、`配置已修正：`，`status.rs` 的消息测试等）需要固定语言：测试里调用 `i18n::init_for_test("zh-CN")`，或者改为断言 message id。约 10 处 |
| 文档 | `README` 增加 "Translating" 一节；`CONTRIBUTING` 写明 1.3 第 5 条的四个测试 |
| 合计 | **约 350 行新增 / 100 行改动，1 个人 1–1.5 天** |

---

## 议题二：配置面板

### 2.1 面板形态 —— 结论：**复用正文区**

| 形态 | 代价 |
|---|---|
| 全屏覆盖 | 标题、status bar 都被盖住，和"沉浸写作"的界面差异太大；而且整屏重绘一套新布局，`compute_layout` 要多一种分支 |
| 居中浮层 | 需要 `Clear` + 边框 + 背景，否则和下面的正文混在一起分不清。这违反了 §5.4 "不设置任何背景色，不画边框"的约定；而在浅色 / 深色两种主题下都好看的背景色只能硬编码 RGB 或依赖 REVERSED 大面积反色，二者都很丑 |
| **复用正文区** | 面板只替换 `layout.body` 这块矩形的内容；标题行显示 `设置`（BOLD），status bar 显示操作提示。不需要边框和背景，沿用现有 Widget 的风格。窗口高度不够时让列表滚动 |

复用正文区还有一个副作用是好处：调整 `width` 时面板本身随之变宽变窄，**这就是实时预览**。`code_color` 看不到正文，用行内的色块和样例代替（2.2）。

最小可用尺寸：正文宽度最小 40 列（`WIDTH_RANGE`），一行 `标签 + 值` 最长约 32 列，放得下。高度为 10 行设置 + 1 行说明 = 11 行。正文区低于 11 行时，列表跟随选中项滚动（复用 scrolloff 的思路，不做通用滚动组件）。

### 2.2 控件 —— 结论：**表驱动的 `enum Field`，不做通用 trait**

10 个配置项只有 3 种控件。为 3 种控件定义 trait + 动态分发，代码量反而比直接写 match 多，而且以后不会有第 4 种控件。方案：

```rust
// src/settings.rs（草图，不是最终代码）
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field { Width, Vim, Mouse, Autosave, AutosaveIdle, Typewriter, FocusMode, Scrolloff, TabWidth, CodeColor }
pub enum Kind { Toggle, Int { min: i64, max: i64, step: i64, unit: &'static str }, Choice(&'static [CodeColor]) }
impl Field {
    pub const ALL: [Field; 10] = [...];               // 显示顺序 = 这里的顺序
    pub fn key(self) -> &'static str;                  // toml 键名，复用 config::KEYS
    pub fn kind(self) -> Kind;                         // 范围复用 config.rs 里的 *_RANGE 常量
    pub fn get(self, c: &Config) -> Value;             // 一个 match
    pub fn set(self, c: &mut Config, v: Value);        // 一个 match，内部钳制
}
pub enum Value { Bool(bool), Int(i64), Color(CodeColor) }
```

控件行为：

| 控件 | 显示 | 调整 |
|---|---|---|
| 开关 | `开` / `关`（`关` 为 DIM） | `Space` / `←` / `→` 切换（`Enter` 留给"保存并关闭"，见 2.3） |
| 数字 | `‹ 72 ›`，带单位（`ms`、`列`、`行`、`格`） | `←` / `→` 按步进调整：width 2、autosave_idle_ms 500、scrolloff 1、tab_width 1。`PageUp` / `PageDown` 一次调 10 步。到边界后不再变化，不循环。**不支持直接输入数字**：面板里放文本输入框会引入 IME 和光标问题，步进加 ×10 已经足够覆盖最大的范围（500–60000 ms，最多约 12 次按键） |
| 单选（8 色） | `‹ ■ cyan ›`：`■` 用该颜色作为前景色，后面跟颜色名（颜色名本身不着色）；说明行显示一个着色的 `` `code` `` 样例 | `←` / `→` 循环切换 |

每个选项下面不放常驻说明。选中某一项时，在面板底部的说明行显示一句话（例如"停止输入多久后自动保存"），这样 10 行列表保持紧凑。

### 2.3 交互

| 问题 | 结论 |
|---|---|
| 打开 | **`Ctrl+P`**（Preferences）。`Ctrl+O`、`Ctrl+,` 等候选的问题：`Ctrl+,` 在 legacy 键盘协议下无法表示；`Ctrl+P` 当前未被占用，vim Normal 态也没有绑定；`cfmakeraw` 已经关闭 `IEXTEN`，不会被终端拦截。新增 `Action::OpenSettings`（这偏离了 §11.2 "Action M1 定稿"的约定，属于有意的修订，需要同步更新方案文档） |
| 导航 | `↑` / `↓` 和 `Tab` / `Shift+Tab`（BackTab）移动选中项，首尾**不循环**。`j` / `k` **不绑定**：中文 IME 开启时这两个键会被截获（§14 风险 2），而且面板不应该依赖 vim 开关 |
| 生效时机 | **实时生效 + 打开时拍快照**。每次调整都立刻调用 `App::apply_config`，用户能马上看到效果 |
| 确认 / 取消 | **`Enter` 一律表示保存并关闭**（写回文件），不论选中的是哪一行；开关只用 `Space` / `←` / `→` 切换，避免 `Enter` 有两种含义。`Esc`：恢复快照（`apply_config(snapshot)`）并关闭，不写文件。`Ctrl+P`：等同于 `Esc` |
| 与 `--vim` 共存 | 面板打开时**吞掉所有按键**，不经过 `Keymap::map_event`，`mode` 保持原样。唯一例外是 `Ctrl+Q`：先执行 `Esc` 的逻辑（恢复快照），再走正常的退出流程（包括未保存时的二次确认）。`Event::Paste` 直接丢弃；鼠标事件忽略（不做面板的点击 hit-test） |
| vim 被关闭时 | 如果 `mode` 是 `Normal`，强制切到 `Insert`，并推送 `SetCursorStyle::SteadyBar`，同时清除 pending / visual 和选区。vim 从关闭切到开启时 `mode` 保持 `Insert` |
| 与「保存为:」提示行互斥 | 两者都是"抢占输入的浮层"，规则是**同一时刻最多存在一个**：提示行打开时，`Ctrl+P` 会进入 `PromptInput` 分支，`Prompt::edit` 会忽略 Ctrl 组合键，所以天然打不开面板；面板打开时会吞掉 `Ctrl+S`，所以也弹不出提示行。在 `App::handle_at` 里加一个 `debug_assert!(!(settings.is_some() && status.prompt.is_some()))`。**不**把两者重构成统一的 `enum Overlay`：只有两个浮层，重构带来的收益不值得改动 `StatusModel` |
| 后台行为 | 自动保存、消息过期、窗口 resize 都照常进行。面板打开期间窗口缩到 `too_small` 时显示"窗口太小"，面板状态保留，窗口恢复后继续显示 |

### 2.4 持久化 —— 结论：**`Enter` 时写回 `~/.mustty.toml`，用 `toml_edit` 只改动变化过的键**

只在本次会话生效是不够的：用户在面板里改完设置，下次启动又恢复原样，会以为是 bug。而且不写回的话，面板只是 `Ctrl+T` / `Ctrl+F` 的增强版，价值很低。

写回规则：

1. **只写差异**：对比"打开面板时的快照"和"关闭时的值"，只写真正变化的键。这样可以避免把命令行参数泄漏进配置文件：比如用 `--width 90` 启动、配置文件里是 64，用户在面板里只改了 vim，就只写 `vim`，`width = 64` 保持不变。
2. **用 `toml_edit::DocumentMut` 保留注释和键顺序**。实测把 `width = 72` 改成 64 时，同一文件里的 `# my width` 注释和 `vim = true # keep` 的行尾注释都原样保留。已有的键原地修改；新键追加到末尾；未知键和其他表（`[keys]` 之类）不动。依赖代价：+115 KB，新增 3 个 crate（`toml_edit` 与 `toml` 共用 `toml_parser` / `toml_datetime` / `serde_spanned`）。
3. **原子写入**：把 `io::file` 里"写临时文件 → fsync → rename"的逻辑抽成 `io::file::atomic_write(path, bytes)`，配置和文档共用。
4. **符号链接**：很多人用 dotfiles 管理工具把 `~/.mustty.toml` 链接到仓库里。直接对链接路径做 rename 会把链接替换成普通文件。**写入前先 `fs::canonicalize`，对真实目标做原子替换**。
5. **文件有语法错误**：拒绝写入，在 status bar 显示"配置文件有语法错误，修改仅本次有效"（一条错误消息），面板照常关闭，修改仍然在当前会话生效。不尝试修复用户的文件。
6. **被命令行覆盖的键**：在值后面以 DIM 显示 `(命令行)`，提示用户"你改的值下次启动时仍会被命令行参数覆盖"。这要求 `App` 保留启动时的 CLI `PartialConfig`。
7. 写入失败（权限等问题）：显示错误消息，修改仍然在当前会话生效。

### 2.5 渲染 —— 结论：**不破坏现有风格：无边框、无背景**

- 选中行：整行 `Modifier::REVERSED`，和文本选区使用同一种样式（`theme::selection()`）。反色由终端主题决定，浅色和深色背景下都清晰可见。
- 标签用默认前景色；值靠右对齐；`关` 和单位用 DIM；`(命令行)` 用 DIM。
- 标题行 `设置`（BOLD）；status bar 显示 `↑↓ 选择  ←→ 调整  Enter 保存  Esc 取消`（整体 DIM，和现有 status bar 一致）。
- 只用 8 个基本 ANSI 色和 Modifier，不写死 RGB。已知的缺陷：`■ black` 在深色背景下、`■ white` 在浅色背景下几乎看不见。这是 `code_color` 本身就存在的问题，色块旁边的颜色名可以兜底，**不另做处理**。

### 2.6 成本估算

**会波及的结构**：目前 `Config` 只在 `App::with_config` 里被读取一次，然后**拆散到 9 个字段和 3 个子对象**里。面板需要能把它们重新设置一遍。逐项检查运行时修改是否安全：

| 配置项 | 当前存放位置 | 运行时修改 |
|---|---|---|
| width | `App.measure` | 不需要改动：`sync_body` 已经会在 `wrap.width() != body.width` 时 rewrap |
| autosave / autosave_idle | `App.autosave` / `App.autosave_idle` | 不需要改动：每次 tick 都会重新读取。从关切到开且文档有修改时，令 `last_edit = Some(now)`，让它在空闲后自动保存 |
| typewriter / focus_mode / scrolloff | `Viewport.typewriter` / `App.focus_mode` / `Viewport.scrolloff` | 不需要改动（前两者已经有 toggle 路径） |
| code_color | `App.code_color: Color` | **类型要改**：`Config.code_color` 现在存的是 `ratatui::Color`，`CodeColor → Color` 的映射是有损的（`White` 和 `gray` 都映射到 `Color::Gray`），面板无法从 `Color` 还原出选项。改为存 `CodeColor`，使用处调用 `.color()`。会波及 `config.rs` 的测试、`app.rs`、`view/mod.rs` |
| vim | `Keymap.vim` + `App.mode` | 需要处理 2.3 里的 Normal → Insert 重置和光标形状 |
| tab_width | **三份副本**：`App.tab_width`、`Keymap.tab_width`、`Editor` 里的 `WrapCache.tab_width` | **最大的改动点**：`WrapCache` 没有 `set_tab_width`，需要新增（清空缓存 + `viewport.rewrap`）；三份副本必须同步更新 |
| mouse | `App.mouse` + `TerminalGuard.mouse`（还被 panic hook 捕获） | 运行时开关需要通过 `App.output` 推送 `EnableMouseCapture` / `DisableMouseCapture` 序列，同时清除 `drag`。**`TerminalGuard` 的 `Drop` 和 panic hook 改为无条件发送 `DisableMouseCapture`**（对未开启鼠标的终端无副作用），否则启动时没开鼠标、运行中开启后退出，终端会残留鼠标模式 |

做法：新增 `App::apply_config(&mut self, new: &Config)` 作为**唯一入口**，逐项比较差异并分发到上表的存放位置。`with_config` 也改为调用它，把初始化逻辑去重。**不需要**把配置改成 `Rc<RefCell<Config>>` 之类的共享可变结构，"启动时读取一次"的架构保持不变，只是多了一个"重新应用"的入口。

| 模块 | 内容 | 估算行数（代码 + 测试） |
|---|---|---|
| `src/settings.rs`（新） | `Field` / `Kind` / `Value`、面板状态（选中项、快照、滚动）、按键处理 | 220 + 150 |
| `src/view/settings.rs`（新） | `SettingsWidget`：行布局、右对齐、色块、说明行、滚动 | 120 + insta 快照 |
| `src/config.rs` | `code_color` 改类型；`write_back(diff)`（toml_edit、符号链接、语法错误拒绝写入） | 90 + 100 |
| `src/io/file.rs` | 抽出 `atomic_write` | 20 |
| `src/app.rs` | `apply_config`、面板按键路由、`Ctrl+Q` 例外、保留 CLI 层 | 110 + 80 |
| `src/text/wrap.rs` | `set_tab_width` | 10 + 15 |
| `src/terminal.rs` | 鼠标捕获改为无条件关闭 | 5 |
| `src/action.rs`、`src/keymap.rs` | `OpenSettings` + `Ctrl+P` 绑定 | 5 + 10 |
| 合计 | | **约 580 行代码 + 约 400 行测试，2 个新模块，新增依赖 `toml_edit`；1 个人 3–4 天** |

需要新增的测试：
- `Field`：每一项 get/set 往返；数字到边界后不再变化；颜色循环。
- 面板状态机：`Esc` 恢复快照、`Enter` 生成正确的 diff、导航到首尾不循环、`Ctrl+Q` 先回滚。
- `write_back`：保留注释、行尾注释和键顺序；新键追加到末尾；未知键和其他表不动；符号链接目标被更新而链接本身保留；语法错误时拒绝写入；只写有变化的键（用 `--width` 启动的场景）。
- `apply_config`：修改 tab_width 后 `Editor` 现有的 `is_consistent` 检查通过；vim 从开到关时 Normal 被重置为 Insert 且推送了光标序列；mouse 开关推送了正确的序列。
- 渲染：`insta` 快照，正文宽度 40 和 72 各一份，外加高度不足时的滚动场景。

### 2.7 风险清单（最可能出问题的 3 个点）

1. **tab_width 的三份副本不同步**：漏改任何一份都会造成渲染位置、光标位置、鼠标命中、`Tab` 插入的空格数四者之间不一致，而且只在包含 `\t` 的文档里才会出现，很难发现。对策：`apply_config` 是唯一入口；加一个 proptest：随机文本（含 `\t`）+ 随机修改 tab_width 后，`WrapCache::is_consistent` 必须成立，`ScreenMap` 的往返偏差 < 1 个 grapheme 宽度。
2. **写回 `~/.mustty.toml` 破坏用户文件**：包括符号链接被替换成普通文件、命令行参数被写进文件、并发编辑（用户在另一个窗口里同时编辑配置文件）时覆盖对方的修改。对策：canonicalize + 只写差异；写入前重新读取并解析文件，在最新内容上应用差异，而不是用打开面板时的旧内容覆盖。
3. **运行时切换 mouse 的终端状态泄漏**：Windows 的 crossterm 鼠标捕获走 WinAPI 而不是 ANSI 序列，`write_ansi` 路径在 Windows 上不生效；panic hook 捕获的是启动时的 `mouse` 值。对策：Windows 上鼠标开关改为直接 `execute!`；`Drop` 和 panic hook 无条件关闭鼠标捕获；手工验证清单（§13.3）增加"面板里开启鼠标 → 退出 → shell 拖选正常"。

---

## 如果只做其中一个：**先做 i18n**

1. **i18n 是开源的门槛，配置面板不是**。现在 UI 全是中文，不懂中文的用户第一次运行就会卡住；而配置面板缺失的代价只是"手动编辑 10 行 TOML"，README 写清楚就够了。
2. **i18n 更便宜**（约 1–1.5 天、3 个小依赖、几乎不改架构），面板约 3–4 天，还要改动状态管理和终端生命周期。
3. **先后顺序本身也有影响**：面板会新增约 30 条字符串（10 个标签、10 条说明、操作提示、颜色名、错误消息），**比现在全部的字符串还多**。先做 i18n，面板从第一天起就直接写 `.ftl`；反过来做的话，要先写一遍中文硬编码，再迁移一遍。

---

## 附录 A：实测方法

最小样例程序（打印一条带变量的消息）分别使用各个方案，`cargo build --release`，`[profile.release] strip = true`，与 `fn main(){println!(..)}` 的基线（324 KB）对比。"新增 crate 数"为样例的 `cargo tree -e normal` 与 mustty 当前依赖集合（113 个）的差集。样例代码在会话 scratchpad 中，没有提交到仓库。

| 样例 | 二进制 | 增量 |
|---|---|---|
| 基线 | 324 KB | — |
| sys-locale | 320 KB | ≈ 0 |
| rust-i18n 4.2 | 381 KB | +57 KB |
| fluent-bundle 0.16（无 macros feature、关闭 isolating） | 581 KB | +257 KB |
| fluent-templates 0.15 | 628 KB | +304 KB |
| i18n-embed 0.16 + i18n-embed-fl 0.10 + rust-embed 8 | 681 KB | +356 KB |
| toml 1.1（mustty 现有依赖） | 536 KB | — |
| toml 1.1 + toml_edit 0.25 | 651 KB | +115 KB |
