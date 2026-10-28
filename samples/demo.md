# mustty 演示文档

这是一份用来手工验收 M0 的示例文档。它混合了中文、English、emoji 🎉、长 URL、多级标题和代码块，用来检查居中布局、软换行和避头尾规则是否正确。

## 一、中英混排

终端写作器最难的部分之一，是在等宽网格里正确处理中英混排：English words should break at spaces, 而中文字符之间可以在任意位置断开，但是逗号、句号、右引号这类标点不能出现在行首，左引号、左括号不能出现在行尾。

Rust 是一门注重安全与性能的系统编程语言，它的所有权（ownership）模型在编译期就消除了数据竞争。ratatui 是基于它的终端 UI 库，crossterm 负责跨平台的终端后端。

「引号里的句子应该完整地待在一起吗？」不一定——但右引号」绝不能孤零零地跑到下一行的行首，这就是所谓的避头尾。（括号同理。）

一段很长很长的中文句子，没有任何空格，用来测试纯中文在七十二列宽度下的断行效果，每一行应该恰好容纳三十六个汉字，标点符号如果刚好落在行首就要把前一个字一起带下来，保证版面整齐、阅读舒适、没有奇怪的空洞。

### 1.1 Emoji 与组合字符

家庭表情 👨‍👩‍👧 是一个由 ZWJ 连接的字形，国旗 🇨🇳 🇯🇵 🇺🇸 由两个区域指示符组成，组合字符 é（e + ́）也是一个字形。光标左右移动时，它们都应当一步跨过。

Emoji in English text 🚀 should wrap just like any other wide character 🌏, taking two columns each 🎨🎭🎪🎢🎡🎠.

### 1.2 长 URL

下面这个 URL 比一整行还要长，必须被强制断行：

https://example.com/a/very/long/path/that/keeps/going/and/going/and/going/without/any/spaces/at/all?query=string&with=many&parameters=true#and-a-fragment-too

再来一个嵌在句子里的链接：参见 [ratatui 文档](https://docs.rs/ratatui/latest/ratatui/terminal/struct.Frame.html#method.set_cursor_position) 了解硬件光标的设置方法。

## 二、代码块

```rust
fn main() {
    let greeting = "你好，世界";
    println!("{greeting}");
}
```

```text
	Tab 缩进的一行（Tab 宽 4 列）
    四个空格缩进的一行
```

## 三、列表与引用

- 第一项：简短
- 第二项：稍微长一点的条目，用来观察列表项在软换行之后的效果，后续行不会自动缩进
- Third item in English with some *emphasis* and **strong** text
  - 嵌套的子项
  - 另一个子项

1. 有序列表第一项
2. 有序列表第二项
3. 有序列表第三项

> 引用块：写作是一种思考方式。The act of writing forces you to clarify what you actually think, 而不是你以为自己在想什么。

---

## 四、长篇正文
### 4.1 小节 1

第1段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 2. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第3段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 4. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第5段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 6. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第7段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 8. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第9段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 10. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

### 4.2 小节 2

第11段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 12. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第13段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 14. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第15段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 16. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第17段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 18. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第19段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 20. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

### 4.3 小节 3

第21段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 22. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第23段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 24. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第25段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 26. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第27段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 28. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第29段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 30. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

### 4.4 小节 4

第31段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 32. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第33段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 34. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第35段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 36. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第37段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 38. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第39段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 40. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

### 4.5 小节 5

第41段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 42. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第43段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 44. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第45段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 46. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第47段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 48. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第49段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 50. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

### 4.6 小节 6

第51段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 52. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第53段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 54. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第55段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 56. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第57段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 58. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第59段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 60. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

### 4.7 小节 7

第61段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

Paragraph 62. The quick brown fox jumps over the lazy dog, while a 敏捷的棕色狐狸 leaps across the sleepy hound — again and again, until the line wraps.

第63段。春江潮水连海平，海上明月共潮生。滟滟随波千万里，何处春江无月明！江流宛转绕芳甸，月照花林皆似霰；空里流霜不觉飞，汀上白沙看不见。

## 五、结尾

#### 四级标题
##### 五级标题
###### 六级标题

最后一行没有特别之处，按 Ctrl+Q 退出。
