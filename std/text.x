// text.x —— 文本处理（std 根目录模块）
// 用法：import std.text;  std.text.replace(...) 等

// 是否包含子串
fn contains(s, sub) {
    index_of(s, sub) >= 0
}

// 全量替换（支持非重叠）
fn replace(s, old, new) {
    let ol = len(old);
    if ol == 0 { return s; }
    let out = "";
    let i = 0;
    let n = len(s);
    while i < n {
        let rest = substr(s, i, n - i);
        let found = index_of(rest, old);
        if found < 0 {
            out = out + rest;
            i = n;
        } else {
            out = out + substr(rest, 0, found) + new;
            i = i + found + ol;
        }
    }
    out
}

// 分割：sep 为空则逐字符
fn split(s, sep) {
    let out = [];
    let n = len(s);
    if sep == "" {
        let i = 0;
        while i < n { out = out + [substr(s, i, 1)]; i = i + 1; }
        return out;
    }
    let sl = len(sep);
    let start = 0;
    let i = 0;
    while i < n {
        if substr(s, i, sl) == sep {
            out = out + [substr(s, start, i - start)];
            i = i + sl;
            start = i;
        } else {
            i = i + 1;
        }
    }
    out = out + [substr(s, start, n - start)];
    out
}

// 重复 n 次
fn repeat(s, n) {
    let out = "";
    let i = 0;
    while i < n { out = out + s; i = i + 1; }
    out
}

// 去首尾空白
fn trim(s) {
    trim_str(s)
}

// 子串出现次数（非重叠）
fn count(s, sub) {
    let sl = len(sub);
    if sl == 0 { return 0; }
    let c = 0;
    let i = 0;
    let n = len(s);
    while i + sl <= n {
        if substr(s, i, sl) == sub { c = c + 1; i = i + sl; }
        else { i = i + 1; }
    }
    c
}

// 反转字符串
fn reverse(s) {
    let out = "";
    let i = len(s) - 1;
    while i >= 0 {
        out = out + substr(s, i, 1);
        i = i - 1;
    }
    out
}

// 左侧填充到 width（ch 默认空格）
fn pad_left(s, width, ch) {
    let out = s;
    let cs = to_str(ch);
    if cs == "" { cs = " "; }
    while len(out) < width { out = cs + out; }
    out
}

// 右侧填充到 width
fn pad_right(s, width, ch) {
    let out = s;
    let cs = to_str(ch);
    if cs == "" { cs = " "; }
    while len(out) < width { out = out + cs; }
    out
}

// 居中填充到 width
fn center(s, width, ch) {
    let out = s;
    let cs = to_str(ch);
    if cs == "" { cs = " "; }
    while len(out) < width {
        if (width - len(out)) % 2 == 0 { out = cs + out; }
        else { out = out + cs; }
    }
    out
}

// 截断：超长保留前 n 字符 + suffix
fn truncate(s, n, suffix) {
    if len(s) <= n { return s; }
    let head = substr(s, 0, n);
    if suffix != "" { return head + suffix; }
    head
}
