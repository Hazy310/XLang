# X 编程语言使用文档

## 目录

1. 语言介绍
2. 环境搭建
3. 基础语法
4. 数据类型
5. 运算符
6. 控制流
7. 函数
8. 类与对象
9. 异常处理
10. 模块与导入
11. 内置函数
12. os 系统模块
13. GUI 图形界面（xlang_gui）
14. MiniX 子语言
15. unsafe 与内存池
16. 版本信息

-----------------------------

## 1. 语言介绍

XLang 是一门**嵌入式解释型脚本语言**，核心定位是「安全兜底 + 底层逃逸」：既提供默认的内存安全保障、避免原生 C 指针的各类悬空 / 越界问题，又保留 unsafe 裸指针、手动内存池等底层操作能力，适合需要精细控存、性能敏感的脚本化场景。

XLang 由 Rust 编写，单文件解释器，自带词法 / 语法 / 解释器三层，无需外部依赖即可运行。

-----------------------------

## 2. 环境搭建

### 2.1 编译

项目为 Cargo workspace，默认只编译根包 `xlang`，GUI 是独立子包。

```
cargo build --release               :: 只编 xlang.exe（根目录默认当前包）
cargo build --release -p xlang-gui  :: 只编 xlang_gui.exe
cargo build --release --workspace   :: 两个都编（xlang.exe + xlang_gui.exe）
```

产物位于 `target\release\`。

### 2.2 运行方式

```
xlang.exe run test.x            # 运行 .x 脚本
xlang.exe run -                 # 从 stdin 读取源码直接运行（IDE 直跑）
xlang.exe run minix.x hello.mx  # 运行 MiniX 子语言脚本（见第 14 章）
xlang_gui.exe run calc2.x       # 运行 GUI 脚本（见第 13 章）
xlang.exe --version             # 输出版本号
xlang.exe                       # 无参数直接进入 REPL
```

命令行选项：

```
xlang.exe run test.x --typecheck=true   # 开启全局静态类型检查
xlang.exe run test.x --clean=true       # 关闭运行提示信息（静默模式）
```

脚本参数（`--` 开头的 CLI 选项除外）会收集进脚本内的 `args` 数组，可通过 `args[0]`、`args[1]` 访问。

> **REPL**：无参数运行进入交互式解释器，提示符 `>>>`，多行语句自动续行 `...`，输入 `exit` 或 `quit` 退出，变量跨行持久保留。

### 2.3 REPL 常驻会话（免进程冷启动）

XLang 每次 `xlang.exe run file.x` 都是**独立进程**，其中 Windows 进程启动（进程创建 + Defender 扫描）约占 35–45ms，而解释器本身的解析 + 执行仅约 0.5ms。若需反复运行脚本并追求极速，可在 **REPL 常驻会话**内用 `run("file.x")` 执行，复用同一个解释器进程，**省去每次的进程启动开销**（实测约 1ms-2ms，比独立进程快约 40 倍）。

```
xlang.exe                # 启动一次常驻 REPL（进程启动只付一次）
>>> run("test15.x")      # 会话内执行文件，复用解释器
>>> run("test_class.x")
```

- `run("file.x")` / `run('file.x')`：会话内读取并执行文件；脚本定义的变量/函数会保留在会话中，供后续命令继续使用。
- 可传参数：`run("file.x", arg1, arg2)`，参数可以是**字面量**或 **REPL 中已有的变量/表达式**，求值后以字符串形式注入脚本的 `args` 数组。

  ```
  >>> let n = 5;
  >>> run("calc.x", n, 42, "a,b")   // 脚本内 args = [5, 42, a,b]
  ```

  字符串内的逗号不会被误拆为参数分隔。
- 时间测量：`run()` 默认打印耗时；`time off` 关闭、`time on`（或 `time`）重新开启。仅对 `run()` 生效，不影响普通表达式。

  ```
  >>> run("test15.x")
  ...
  [run] test15.x 耗时: 1.176 ms
  >>> time off          // 关闭耗时打印
  ```

- 该功能**仅在 REPL 模式可用**；`xlang.exe run file.x` 独立进程的行为不受影响。

-----------------------------

## 3. 基础语法

### 3.1 变量定义

XLang 与 Rust 一样使用 `let` 定义变量，但没有类型标注和 `mut`——变量默认就是可变的。

```XLang
let 变量名 = 变量值;
变量名 = 变量值;  // 重新赋值，变量需提前定义
```

```XLang
let num = 10;
let name = "XLang";
let flag = true;

num = 12;  // 此时 num 的值为 12
```

### 3.2 常量定义

常量一旦定义不可修改。常量名习惯用大写。

```XLang
const 常量名 = 常量值;
```

```XLang
const XNAME = "XLANG";
XNAME = "C";  // 出错：常量不可重新赋值
```

### 3.3 注释

```XLang
// 行注释
/* 块注释
   可跨多行 */
```

### 3.4 输入输出

```XLang
print(expr);        // 输出任意值到控制台
let s = input();    // 读取一行输入；输入结束（EOF）返回字符串 "__EOF__"
```

### 3.5 分号规则

- **普通语句必须以 `;` 结尾**（表达式语句、赋值、let、const 等）。
- **函数/闭包末尾的返回值不加分号**——函数体最后一个表达式即返回值：

```XLang
fn add(a, b) { a + b }        // 不加分号，返回 a+b
fn add2(a, b) { return a + b; }  // 显式 return，必须加分号
```

-----------------------------

## 4. 数据类型

XLang 是动态类型语言，变量可以随时切换类型。支持的类型：**整数、浮点数、布尔、字符串、数组、字典、闭包、类实例**。

### 4.1 整数

```XLang
let n = 42;
let m = -7;
```

### 4.2 浮点数

```XLang
let pi = 3.14159;
let half = 1.0 / 2.0;   // 0.5
```

### 4.3 布尔值

```XLang
let ok = true;
let no = false;
```

### 4.4 字符串

```XLang
let s = "hello";
let t = s + " world";     // 字符串拼接
let n = len(s);           // 字符串长度（字符安全）
let m = s.len;            // 等价取长度
print("hello ${name}");   // 字符串插值：${表达式} 会被求值替换
```

字符串插值示例：

```XLang
let x = 42;
print("x=${x}");          // 输出 x=42
```

### 4.5 数组

```XLang
let arr = [1, 2, 3, true, "hello"];  // 异构数组
let empty = [];                       // 空数组
print(arr[0]);            // 1，下标从 0 开始
arr[1] = 999;             // 元素赋值
print(len(arr));          // 数组长度
```

### 4.6 字典（dict / map）

```XLang
let dict = { "name": "Tom", "age": 18 };  // 字符串键
let d2 = { name: "Jack", age: 20 };       // 标识符键
let e = {};                                // 空字典

print(dict["name"]);        // 读：Tom
dict["age"] = 19;           // 改值
dict["city"] = "成都";       // 新增键
print(dict["age"]);         // 19

let nested = { "a": { "b": 1 } };  // 嵌套字典
print(nested["a"]["b"]);           // 1
```

### 4.7 闭包 / 函数值

```XLang
let f = fn(x) { x * 2 };   // 匿名函数（闭包）
print(f(5));               // 10
```

闭包可以捕获并修改外部共享变量（详见第 7 章）。

### 4.8 区间（Range）

`a..b` 是一个**区间值**，含左右端点，`print` 显示为 `a..b`。`..` 不再仅用于 for，也可以作普通表达式：

```XLang
let r = 1..101;
print(r);           // 1..101
print(5.5..10.5);   // 5.5..10.5
print(-5..-1);      // -5..-1
```

区间端点可为整数或浮点数，支持负数。区间也用作 `match` 的模式（见 6.5 节）。

-----------------------------

## 5. 运算符

### 5.1 算术运算符

```
+  加（也用于字符串拼接）
-  减（也用作一元负号，如 -n）
*  乘
/  除（整数相除得整数）
%  取模
```

### 5.2 比较运算符

```
==  等于      !=  不等于
<   小于      >   大于
<=  小于等于   >=  大于等于
```

比较返回布尔值。

### 5.3 逻辑运算符

使用关键字 `and` / `or`：

```XLang
if a > 0 and a < 10 { ... }
if a == 0 or a == 100 { ... }
```

### 5.4 优先级

优先级从高到低：`() 括号` → `一元负号` → `* / %` → `+ -` → `比较` → `and / or`。

-----------------------------

## 6. 控制流

### 6.1 if / elif / else

```XLang
if a == 1 {
    print("a = 1");
} elif a == 2 {
    print("a = 2");
} else {
    print("其他");
}
```

### 6.2 while

```XLang
let cnt = 0;
while cnt < 5 {
    print(cnt);
    cnt = cnt + 1;
}
```

### 6.3 for 循环（两种语法，均为排他上界）

```XLang
// 语法一：for 变量 in 起始..结束
for i in 0..10 { print(i); }      // 输出 0..9

// 语法二：for 变量 from 起始 to 结束
for i from 1 to 5 { print(i); }   // 输出 1..4（to 5 不含 5）
```

### 6.4 break / continue

```XLang
while true {
    if cond { break; }      // 跳出循环
    if skip { continue; }   // 跳过本轮
}
```

### 6.5 match 模式匹配

`match` 是**表达式**：按顺序匹配分支并返回命中分支的值；无任何分支命中且未写 `_` 通配，则运行时报错。

```XLang
let z = 50;
let r = match z {
    1 => "one",
    2 => "two",
    _ => "other"
};
print(r);              // other
```

**字面量模式**（整数 / 浮点 / 字符串 / 布尔 / 负数）：

```XLang
print(match 7 { 7 => "lucky", _ => "no" });   // lucky
print(match -3 { -3 => "neg three", _ => "no" }); // neg three
```

**区间模式**：`a..b` 表示闭区间 `[a, b]`，subject 落在其中即命中；body 也可直接返回区间对象：

```XLang
let x = match z {
    1..101 => 1..101,   // z ∈ [1,101] 时命中，返回区间对象 1..101
    _ => -1
};
print(x);                       // 1..101
print(match 5.5 { 1.5..10.5 => "float", _ => "no" }); // float
print(match -3 { -5..-1 => -5..-1, _ => 0 });         // -5..-1
```

- 分支用逗号分隔，最后一个可省逗号。
- `_` 为通配，可选，建议放最后。
- 区间模式含两端点，端点可为整数或浮点（含负数）。

-----------------------------

## 7. 函数

### 7.1 定义与返回值

```XLang
fn add(x, y) {
    x + y;        // 末尾表达式即返回值（带分号也可）
}

fn max(a, b) {
    if a > b { return a; }   // 显式 return 必须加分号
    return b;
}

print(add(1, 2));  // 3
```

**返回值规则**：函数体最后一个表达式作为返回值（不加分号）；`return` 语句必须加分号。

### 7.2 闭包与捕获

```XLang
let base = 100;
let addbase = fn(x) { x + base };   // 捕获外部变量 base
print(addbase(5));                   // 105

// 闭包可修改捕获的共享变量
let count = 0;
let inc = fn() { count = count + 1 };
inc(); inc();
print(count);                        // 2

// 函数返回闭包（制造器）
fn makeAdder(n) {
    fn(x) { x + n }
}
let add7 = makeAdder(7);
print(add7(10));                     // 17
```

### 7.3 数组高阶函数

```XLang
let arr = [3, 1, 2];

print(map(arr, fn(x) { return x * 2; }));        // [6, 2, 4]
print(filter(arr, fn(x) { return x > 1; }));     // [3, 2]
print(reduce(arr, fn(a, b) { return a + b; }, 0)); // 6
print(join(arr, "-"));                           // 3-1-2
print(sort(arr));                                // [1, 2, 3]

// 链式调用
let nums = [1, 2, 3, 4, 5];
print(map(filter(nums, fn(x) { return x % 2 == 0; }), fn(x) { return x * 10; }));
// [20, 40]
```

`map` / `filter` 接受闭包；`reduce` 接受闭包和初始值；`join` 用分隔符连接为字符串；`sort` 升序排序。

-----------------------------

## 8. 类与对象

### 8.1 定义、实例化与方法

```XLang
class Person {
    // 实例字段（公开）
    let name;
    let age;
    // 构造函数 new，创建实例时自动调用（第一个参数必须是 self）
    fn new(self, n, a) {
        self.name = n;
        self.age = a;
    }
    // 实例方法：第一个参数固定 self
    fn say(self) {
        print("My name is ${self.name}.");
    }
    // 静态方法：不带 self，属于类本身
    static fn hello() {
        print("hello from Person");
    }
}

let p = Person("Tom", 18);  // 自动调用 new
p.say();                    // 调用实例方法
Person.hello();             // 调用静态方法
print(p.name);              // 访问公开字段
```

> **约束**：方法 / 构造器第一个参数**必须显式声明 `self`**；私有字段用 `#` 前缀，仅类内部可访问。

### 8.2 私有字段

```XLang
class Secret {
    #let password;               // 私有字段
    fn new(self, pwd) {
        self.#password = pwd;
    }
    fn getPwd(self) {
        return self.#password;
    }
}
let sec = Secret("123456");
print(sec.getPwd());             // 123456
// sec.#password;   // 编译错误：外部不能访问私有字段
```

### 8.3 继承

```XLang
class Student : Person {         // 单继承
    let id;
    fn new(self, n, a, i) {
        super(n, a);             // 调用父类构造
        self.id = i;
    }
    // 重写父类方法
    fn say(self) {
        print("Student " + self.name + " id:" + self.id);
    }
}
let s = Student("Jack", 20, 1001);
s.say();              // 调用重写后的方法
print(s.getAge());    // 继承父类方法
print(s.name);        // 继承父类字段（super 设置）
```

支持单继承、父类构造 `super(...)`、方法重写、继承父类方法与字段。

-----------------------------

## 9. 异常处理

使用 `try` / `throw` / `catch`：

```XLang
try {
    print("进入 try");
    throw "程序出错";        // 抛出字符串异常
    print("这行不会执行");
} catch (e) {
    print("捕获到异常: " + e);
}

// 抛数字异常
try {
    throw 100;
} catch (e) {
    print("数字异常: " + e);
}

// catch 不带变量（仅兜底）
try {
    throw "内容";
} catch {
    print("catch 无变量：仅兜底");
}
```

异常支持**跨函数冒泡**和**循环中断**：

```XLang
fn check(n) {
    if n > 10 { throw "数值过大: ${n}"; }
    return n;
}
try {
    check(20);                       // 抛异常，从函数内冒泡到调用处
} catch (e) {
    print("校验失败: " + e);
}
```

-----------------------------

## 10. 模块与导入

### 10.1 导入模块

```XLang
import "lib::math";    // 用 :: 分隔路径，映射到 lib/math.x
import "lib::re" as r; // 可选别名（as 关键字）
```

### 10.2 调用模块内函数

```XLang
import "lib::re";
print(lib::re::split("a,b,c", ","));   // [a, b, c]
print(lib::re::replace("hello world hello", "hello", "hi"));  // hi world hi
print(lib::re::contains("abc", "b"));  // true
```

### 10.3 内置标准库

`lib/` 目录内置两个模块：

**lib/math.x**：`abs` `pow` `sqrt` `max` `min` `factorial` `gcd` `lcm` `is_prime` `clamp` `sign` `floor` `ceil`，常量 `PI`、`E`。

**lib/re.x**：`split` `contains` `replace` `replace_first`（字符安全，支持中文）。

-----------------------------

## 11. 内置函数

### 11.1 字符串函数

```XLang
len(s)            // 字符串/数组长度
substr(s, start, n)  // 取子串
index_of(s, sub)  // 子串位置，找不到返回 -1
split_str(s, sep) // 按分隔符拆成数组
trim_str(s)       // 去首尾空白
starts_with(s, p) // 是否以 p 开头
ends_with(s, p)   // 是否以 p 结尾
```

### 11.2 数值函数

```XLang
to_str(x)       // 转字符串
str_to_num(s)   // 字符串转数字
to_float(x)     // 转浮点
to_int(x)       // 转整数
pow(a, b)       // a 的 b 次方
abs(x)          // 绝对值
sqrt(x)         // 平方根
round(x)        // 四舍五入
version()       // 版本号
```

### 11.3 文件函数

```XLang
read_file(path)          // 读取整个文件为字符串
write_file(path, text)   // 覆写文件
append_file(path, text)  // 追加内容
read_lines(path)         // 按行读取为数组
file_line_count(path)    // 文件行数
read_line_at(path, i)    // 读取第 i 行
file_truncate(path, n)   // 截断文件
edit_line(path, i, text) // 替换第 i 行
append_line(path, text)  // 追加一行
append_at_line(path, i, text) // 在第 i 行后插入
file_exists(path)        // 文件是否存在
delete_file(path)        // 删除文件
```

### 11.4 其他

```XLang
print(x)    // 输出
input()     // 读一行输入，EOF 返回 "__EOF__"
args        // 脚本命令行参数数组
http_server(port, handler)  // 简易 HTTP 服务器
```

-----------------------------

## 12. os 系统模块

`os::` 前缀调用，功能类似 Python 的 `os` 模块：

```XLang
// 路径
os::getcwd()      // 当前工作目录
os::chdir(path)   // 切换目录
os::join("a","b") // 拼接路径
os::basename(p)   // 取文件名
os::dirname(p)    // 取目录名
os::split(p)      // 拆分为 [目录, 文件名]
os::splitext(p)   // 拆分扩展名
os::abspath(p)    // 绝对路径
os::sep()         // 路径分隔符
os::home()        // 用户主目录

// 目录
os::listdir(path) // 列出目录项（数组）
os::mkdir(path)   // 创建目录
os::mkdirs(path)  // 递归创建目录
os::rmdir(path)   // 删除空目录

// 文件
os::remove(path)  // 删除文件
os::rename(a, b)  // 重命名/移动
os::exists(path)  // 是否存在
os::isdir(path)   // 是否目录
os::isfile(path)  // 是否文件
os::filesize(path)// 文件大小

// 环境变量
os::getenv(name)    // 读取
os::setenv(n, v)    // 设置
os::unsetenv(name)  // 删除

// 系统
os::name()        // 操作系统名（如 "windows"）
os::system(cmd)   // 执行命令，返回退出码
os::exec(cmd)     // 执行命令，返回 stdout 输出
os::exec_stdin(cmd, input) // 执行命令并喂 stdin，返回 stdout
```

示例：

```XLang
print(os::getcwd());
print(os::join("a", "b", "c"));
print(os::name());
os::setenv("VAR", "hello");
print(os::getenv("VAR"));
```

-----------------------------

## 13. GUI 图形界面（xlang_gui）

GUI 基于 egui，**必须用 `xlang_gui.exe` 运行**。通过闭包式的声明语法组织界面。

```XLang
// 用法: xlang_gui.exe run calc.x
gui_window("标题", 420.0, 520.0, fn() {
    gui_heading("计算器");
    gui_output("disp");        // 绑定变量的只读输出
    gui_separator();
    gui_col(fn() {
        gui_row(fn() {
            gui_button("7", fn() { digit("7"); }, 46.0);  // 按钮回调 + 高度
            gui_button("8", fn() { digit("8"); }, 46.0);
        });
    });
});
print("窗口已关闭");
```

### 13.1 窗口与布局

```XLang
gui_window(title, w, h, builder)   // 指定窗口大小
gui_window(title, builder)         // 自适应
gui_row(fn(){...})                 // 横向容器（嵌套子控件）
gui_col(fn(){...})                 // 纵向容器（嵌套子控件）
gui_topbar(fn(){...})              // 顶部工具条
gui_topbar_v(fn(){...})            // 纵向顶栏
gui_sidebar(fn(){...})             // 左侧栏
gui_bottombar(fn(){...})           // 底部栏（固定）
gui_separator()                    // 分隔线
gui_spacer()                       // 空白
```

### 13.2 基础控件

```XLang
gui_text(str)                // 静态文本
gui_heading(str)             // 标题
gui_button(label, fn(){...}, h)  // 按钮（回调 + 可选高度）
gui_input(var)               // 单行输入框（绑定变量）
gui_textarea(label, var)     // 多行文本（绑定变量）
gui_output(var)              // 只读输出（绑定变量）
gui_terminal(var, fn(input){...}) // 终端，输入触发回调
gui_checkbox(label, var)     // 复选框
gui_slider(var, min, max)    // 滑杆
gui_color_edit(var)          // 颜色选择
gui_combo(var, options)      // 下拉（可多选）
gui_radio(var, options)      // 单选
gui_table(cols, rows, fn(ri){...}) // 表格，行点击回调
gui_tabs(...)                // 标签页
gui_canvas(fn(){...})        // 自定义画布
canvas_line(x1,y1,x2,y2)     // 画线
canvas_rect(x,y,w,h)         // 画矩形
canvas_circle(x,y,r)         // 画圆
```

### 13.3 运行与退出

```XLang
gui_run(cmd, stdin_text, out_var)  // 异步运行命令，输出写入变量（不阻塞 GUI）
quit()                             // 退出程序（命令行与 GUI 通用）
```

### 13.4 完整 IDE 示例

```XLang
// XLang IDE：顶栏 + 左文件列表 + 右编辑区（egui 多面板）
// 用法: xlang_gui.exe run ide.x
let files = filter(os::listdir("."), fn(f) { os::isfile(f) });
let rows = map(files, fn(f) { [f] });
let current = "";
let editor = "";
let msg = "点击左侧文件开始编辑";

gui_window("XLang IDE", fn() {
    // 顶部工具条（类似 Python IDE 的上边栏）
    gui_topbar(fn() {
        gui_heading("XLang IDE");
        gui_button("新建", fn() {
            current = "";
            editor = "";
            msg = "新建（未打开文件，请先点左侧文件再保存）";
        });
        gui_button("保存", fn() {
            if current != "" {
                write_file(current, editor);
                msg = "已保存 " + current;
            } else {
                msg = "未打开文件，无法保存";
            }
        });
        gui_button("运行", fn() {
            // 异步直跑：不阻塞 GUI，可运行 input()/带 GUI 等阻塞程序
            gui_run("target\\release\\xlang.exe run -", editor, "msg");
        });
        gui_button("退出", fn() { quit(); });
    });
    // 左侧：文件列表
    gui_sidebar(fn() {
        gui_heading("文件 (.x)");
        gui_table(["文件名"], rows, fn(ri) {
            current = files[ri];
            editor = read_file(current);
            msg = "打开 " + current + "（" + len(editor) + " 字节）";
        });
        gui_separator();
        gui_text("共 " + len(files) + " 个文件");
    });
    // 右侧：编辑区（输出区固定在下方面板，不被编辑区挤走）
    gui_heading("编辑区");
    gui_textarea("代码", "editor");
    // 底部固定输出区（必须在 gui_window 的 builder 闭包内声明，才会被收集并渲染；
    // 放在 window 外会在窗口关闭后才执行、永远不显示）
    gui_bottombar(fn() {
        gui_text("输出 / 状态（在下方自由输入，Ctrl+Enter 运行）：");
        gui_terminal("msg", fn(input) {
            if input != "" {
                gui_run("target\\release\\xlang.exe run -", input, "msg");
            }
        });
    });
});
print("IDE 已关闭");

```

> **注意**：布局容器、底部栏等必须放在 `gui_window` 的 builder 闭包内才会被渲染；放在 window 外会在窗口关闭后才执行、永远不显示。

-----------------------------

## 14. MiniX 子语言

MiniX 是用 XLang 自举的一个微型解释器，支持 `.mx` 文件，目前实现 `print`、变量、四则运算、比较、if/else、while、true/false、注释、赋值。

```
xlang.exe run minix.x hello.mx
```

```XLang
// hello.mx
print("Hello, MiniX!");
let a = 12 + 2 * 3 - 5 + 2 / 2;  // 12
print(a);

let sum = 0;
let i = 1;
while i <= 100 {
    sum = sum + i;
    i = i + 1;
}
print(sum);   // 5050

if a > 40 { print("big"); } else { print("small"); }
```

MiniX 关键字：`let print if else while true false`；支持行注释 `//` 与块注释 `/* */`；比较运算符 `== != < > <= >=`。

-----------------------------

## 15. unsafe 与内存池

XLang 内置一套手动内存池，用于精细控制内存。所有裸指针操作必须在 `unsafe {}` 块内进行。

```XLang
unsafe {
    // 裸指针操作
}
```

内存池内置函数：

```XLang
mem_global_init()      // 初始化全局内存池
mem_alloc(size)        // 分配内存
mem_free(ptr)          // 释放
mem_rollback()         // 回滚
mem_global_destroy()   // 销毁全局内存池
```

-----------------------------

## 16. 版本信息

当前版本：**xlang 0.3.0**

```XLang
print(version());   // 输出版本号
xlang.exe --version
```

---

### 附：完整关键字表

`let` `const` `fn` `if` `else` `elif` `while` `for` `in` `from` `to` `return` `break` `continue` `throw` `try` `catch` `true` `false` `import` `class` `static` `and` `or` `unsafe` `self` `super` `..`（区间）
