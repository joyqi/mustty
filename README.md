# mustty

一个跨平台的终端 Markdown 写作器：在终端中央一栏固定宽度的纸面上安静地码字。
当前为 M2：可编辑、undo/redo、粘贴、停手 2 秒自动保存（BOM / CRLF 原样还原）。
设计方案见 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 运行

```sh
cargo run -- samples/demo.md
```

`mustty [FILE] [--no-autosave]`：`Ctrl+S` 保存（无文件名时提示「保存为」），`Ctrl+Z` / `Ctrl+Y` 撤销 / 重做，`Ctrl+Q` 退出；正文首行按 `↑` 编辑标题。设置 `MUSTTY_LOG=<path>` 写调试日志。
