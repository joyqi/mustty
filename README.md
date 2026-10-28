# mustty

一个跨平台的终端 Markdown 写作器：在终端中央一栏固定宽度的纸面上安静地码字。
当前为 M1：可编辑、原子保存（BOM / CRLF 原样还原），尚无 undo 与自动保存。
设计方案见 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 运行

```sh
cargo run -- samples/demo.md
```

`mustty [FILE]`：`Ctrl+S` 保存（无文件名时提示「保存为」），`Ctrl+Q` 退出（未保存需再按一次）；正文首行按 `↑` 编辑标题。
