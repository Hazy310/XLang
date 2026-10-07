# XLang Language Support

XLang 的 VSCode 扩展：完整语法高亮、代码片段、一键运行/类型检查脚本。

## 功能

- **语法高亮**：覆盖 XLang 全部语法
  - 关键字（`let` `const` `fn` `class` `static` `if/elif/else` `while` `for` `in/from/to` `return` `break` `continue` `throw/try/catch` `and` `or` `unsafe` `import`）
  - 字符串：普通 `"..."`（含转义与 `"${x}"` 插值）、原始字符串 `r"..."`
  - 数字（十进制/十六进制/浮点）
  - 注释：`//` 行注释、`/* */` 块注释
  - 内置函数（`print` `len` `input` 文件系列、数组高阶 `map/filter/reduce/join/sort`、类型转换 `to_int/to_float/to_str` 等）与 `os::` 模块函数
  - GUI 函数（`gui_*` 控件、`canvas_*` 绘制、`quit`）
  - 类/函数定义（含继承 `class X : Y`）、匿名函数 `fn(x)`、私有字段 `#field`、`self`/`super`/`new`
- **代码片段**：`print` `fn` `class` `if` `for` `while` `dict` `try` `match` `import` `gui_window` `gui_button` `gui_input_var` `gui_table` `map` `filter` `reduce` 等
- **运行命令**：直接运行当前 `.x` 脚本、静态类型检查、显示版本

## 命令

| 命令 | 说明 | 快捷键 |
|------|------|--------|
| `XLang: 运行当前脚本` | 用 `xlang.exe run <file>` 运行当前文件 | `Ctrl+F5` |
| `XLang: 运行当前脚本（静默）` | 同运行，加 `--clean=true` 静默 | - |
| `XLang: 静态类型检查当前脚本` | 加 `--typecheck=true` 检查类型 | - |
| `XLang: 显示版本` | 显示 `xlang.exe --version` | - |

## 配置

| 设置 | 默认 | 说明 |
|------|------|------|
| `xlang.executablePath` | `""` | `xlang.exe` 绝对路径；留空自动查找 PATH |
| `xlang.runInTerminal` | `true` | 在集成终端输出（支持 `input()`）；关闭则在通知中显示 |

## 安装

### 方式一：VSIX 安装包
1. 安装 [Node.js](https://nodejs.org) 与 VSCode
2. 安装打包工具：`npm install -g @vscode/vsce`
3. 在扩展目录执行 `vsce package` 生成 `.vsix`
4. VSCode 中「扩展」→ 右上角「...」→「从 VSIX 安装」选择该文件

### 方式二：开发模式（F5 调试）
1. 在扩展目录执行 `npm install -g @vscode/vsce`（或仅 `npm install -D @vscode/vsce`）
2. 用 VSCode 打开本目录，按 `F5` 启动扩展开发主机

## 使用前提

需要已编译的 `xlang.exe`（见 XLang 根目录 `cargo build --release`），并将其加入 PATH，或在设置 `xlang.executablePath` 中指定绝对路径。
