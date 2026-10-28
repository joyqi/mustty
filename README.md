# mustty

一个跨平台的终端 Markdown 写作器：在终端中央一栏固定宽度的纸面上安静地码字。
当前为 M0：居中只读阅读器（软换行、中英混排避头尾、resize 自适应），尚不能编辑。
设计方案见 [docs/implementation-plan.md](docs/implementation-plan.md)。

## 运行

```sh
cargo run -- samples/demo.md
```

`↑ ↓ ← →` 移动光标，`PageUp / PageDown` 翻页，`Ctrl+Q` 退出。
