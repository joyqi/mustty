# mustty

一个跨平台的终端 Markdown 写作器：打开一个文件，在终端中央一栏固定宽度的纸面上安静地码字。
界面只有标题 / 正文 / status bar 三块；Markdown 标记变暗显示（iA Writer 式），不隐藏、不渲染。
设计方案见 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 运行

```sh
cargo run --release -- notes.md            # 打开；文件不存在则首次保存时创建
cargo run --release --                     # Untitled，Ctrl+S 时提示「保存为:」
cargo run --release -- --help
```

```text
mustty [FILE] [--width N] [--[no-]autosave] [--[no-]vim] [--[no-]mouse]
```

停手 2 秒自动保存，切走窗口（FocusLost）与退出时也会保存；保存为原子写入，BOM / CRLF / 末尾换行原样保留。
status bar 里文件名后的标记表示保存状态：`*` 为有未保存修改，绿色 `●` 为已与磁盘一致，空的 Untitled 不显示。
另一个程序改了文件时不会自动覆盖，status bar 会常驻提示，`Ctrl+S` 强制覆盖。
界面语言依次取 `MUSTTY_LANG`、`LC_ALL`、`LC_MESSAGES`、`LANG`，都没有时用系统语言，目前有 `en` 和 `zh-CN`（例：`MUSTTY_LANG=en mustty`）。
clap 自带的 `Usage:` / `Options:` 和参数解析错误始终是英文。
设置 `MUSTTY_LOG=<path>` 时把调试日志（含帧耗时）写到该文件，永不写 stdout / stderr。

## 键位

| 键 | 动作 |
|---|---|
| 字符 / 输入法提交 | 插入 |
| `Enter` / `Backspace` / `Delete` / `Tab` | 换行 / 删除前一个字形 / 删除后一个字形 / 插入空格 |
| `← →` `↑ ↓` | 按字形 / 按视觉行移动；正文首行按 `↑` 进入标题，标题里 `Enter` 或 `↓` 回正文 |
| `Home` / `End`、`PageUp` / `PageDown` | 视觉行首 / 尾、翻页 |
| `Ctrl+Home` / `Ctrl+End` | 正文开头 / 结尾 |
| `Ctrl+←` `Ctrl+→`、`Alt+B` `Alt+F` | 按词移动（后两者对应 macOS 终端的 Option 键） |
| `Ctrl+Backspace`、`Ctrl+W` | 删除前一个词 |
| `Shift+` 移动键、`Ctrl+A` | 扩展选区、全选正文 |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | 复制 / 剪切 / 粘贴（内部寄存器；复制同时经 OSC 52 写系统剪贴板） |
| `Ctrl+Z` / `Ctrl+Y` | 撤销 / 重做 |
| `Ctrl+S` | 保存 |
| `Ctrl+T` | Typewriter 模式：光标行始终在正文中线 |
| `Ctrl+F` | Focus 模式：当前段落之外变暗 |
| `Esc` | 取消选区 / 关闭提示行 |
| `Ctrl+Q` | 退出（有未保存修改且无法自动保存时需再按一次） |

终端的系统粘贴（Cmd+V / Ctrl+Shift+V / 右键）作为一次粘贴插入，一次 `Ctrl+Z` 即可撤销。
系统剪贴板走 OSC 52，可穿透 SSH；tmux 里需要 `set -g set-clipboard on`。macOS 自带的 Terminal 不支持 OSC 52，复制只进内部寄存器。

### `--vim`

启动时处于 Insert 态，按键与上表相同；`Esc` 进入 Normal 态（status bar 最前显示 `NOR` / `INS` / `VIS`，Normal 态光标为方块）。

| Normal 态 | 动作 |
|---|---|
| `i` / `a` / `o` | 进入 Insert；`a` 先右移一格，`o` 在下方开新行 |
| `h j k l`、`w b e`、`0 $`、`gg G` | 移动（`j k` 按视觉行） |
| `x`、`dd`、`p`、`u`、`Ctrl+R` | 删除字形、剪切整行、粘贴、撤销、重做 |
| `v` 后移动，再按 `y` / `d` / `x` / `p` | 选区：复制 / 剪切 / 剪切 / 替换粘贴 |

所有 `Ctrl+` 组合键在两种状态下都有效。不支持计数前缀、`.` 重复、宏、`:` 命令行与 text object。
与 vim 一样，visual 选区是闭区间，光标所在的字也在选区内：`v` `e` `d` 删掉整个词，`v` `w` `d` 删到下一个词的词首（含词首）。

**中文输入法与 `--vim`**：输入法处于中文状态时，`j` `k` `d` 等按键会被输入法当作拼音截获，程序收不到；
Normal 态收到的汉字一律忽略、不会误插入。建议为终端单独设置英文输入源，或在按 `Esc` 之前先切到英文。
不加 `--vim` 时不受影响。

## 配置文件

`~/.mustty.toml`（Windows 为用户主目录下的 `.mustty.toml`）。优先级：命令行参数 > 配置文件 > 内置默认值。
成对的开关（如 `--vim` / `--no-vim`）以后出现的为准，可以覆盖配置文件里的值。

文件不存在则全部使用默认值。写错也照常启动：语法错误时整个文件作废；单个键类型不对或取值无效只回退这一个键；
数值越界钳制到边界。警告会汇总成一条显示在 status bar（按任意键消失），并打印到 stderr。未知的键直接忽略。

```toml
width = 72              # 正文宽度，40..=120
vim = false
mouse = false           # 开启后：单击定位、拖拽选择、滚轮滚动（见下）
autosave = true
autosave_idle_ms = 2000 # 500..=60000
typewriter = false      # 启动时是否进入 Typewriter 模式（Ctrl+T 切换）
focus_mode = false      # 启动时是否进入 Focus 模式（Ctrl+F 切换）
scrolloff = 3           # 光标与视口上下边保持的行数，0..=10
tab_width = 4           # Tab 字符的显示宽度与 Tab 键插入的空格数，1..=16
code_color = "cyan"     # 代码的颜色：black red green yellow blue magenta cyan white（gray 同 white，大小写不敏感）
```

### 鼠标

默认关闭，此时 mustty 不接管鼠标，终端原生的拖选复制照常可用。
`mouse = true` 或 `--mouse` 开启后：单击正文定位光标（单击标题进入标题）、拖拽选择（拖到正文上下边缘之外会持续滚动）、滚轮滚动视口。
开启后终端原生的拖选复制会失效；需要原生选择时**按住 Shift 再拖拽**（xterm、GNOME Terminal、Windows Terminal、WezTerm、Alacritty、kitty 通用；
iTerm2 通常是 ⌥ Option，macOS Terminal 是 Fn，以终端设置为准）。`--vim` 的 Normal 态里拖拽会直接进入 visual，之后可以按 `d` / `y`。

## Translating

1. 复制 `locales/en.toml` 为 `locales/<tag>.toml`（如 `ja.toml`、`zh-TW.toml`），只翻译等号右边，`{n}` 这类占位符原样保留。
   缺的条目会自动用英文顶上，所以不必一次翻完。
2. 在 `src/i18n.rs` 的 `LOCALES` 表里加一行：`("<tag>", include_str!("../locales/<tag>.toml")),`。
3. 运行 `cargo test`：测试会检查文件能否解析、有没有 en 里不存在的 key、占位符是否与 en 一致。
4. 用 `MUSTTY_LANG=<tag> cargo run -- notes.md` 看效果。

英文只在字数处区分单复数（`status-words-one` / `status-words-other`），其他语言两条可以填同一个译文。
