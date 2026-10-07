// json.x —— JSON 解析与序列化（std 根目录模块）
// 用法：import std.json;   std.json.parse(s) / std.json.stringify(v)
// 表示：对象->dict（键字符串），数组->array，字符串->str，数字->int/float，true/false->bool，null->字符串 "__null__"
// 说明：JSON 数字超出 i32 范围可能溢出；\uXXXX 转义不支持（跳过）；dict 键必须为字符串

let jpos = 0;  // 解析游标（模块级，函数内经 std.json.jpos 访问）

// 跳过空白
fn skip_ws(s) {
    let n = len(s);
    let go = 1;
    while go == 1 and std.json.jpos < n {
        let c = substr(s, std.json.jpos, 1);
        if c == " " or c == "\t" or c == "\n" or c == "\r" {
            std.json.jpos = std.json.jpos + 1;
        } else {
            go = 0;
        }
    }
}

// 解析 JSON 字符串（当前游标在 " 处）
fn parse_string(s) {
    let n = len(s);
    std.json.jpos = std.json.jpos + 1;  // 跳过开头 "
    let out = "";
    while std.json.jpos < n {
        let c = substr(s, std.json.jpos, 1);
        if c == "\"" {
            std.json.jpos = std.json.jpos + 1;
            return out;
        }
        if c == "\\" {
            let e = substr(s, std.json.jpos + 1, 1);
            std.json.jpos = std.json.jpos + 2;
            if e == "n" { out = out + "\n"; }
            elif e == "t" { out = out + "\t"; }
            elif e == "r" { out = out + "\r"; }
            elif e == "\"" { out = out + "\""; }
            elif e == "\\" { out = out + "\\"; }
            elif e == "/" { out = out + "/"; }
            elif e == "u" { std.json.jpos = std.json.jpos + 4; }
        } else {
            out = out + c;
            std.json.jpos = std.json.jpos + 1;
        }
    }
    out
}

// 解析数字
fn parse_number(s) {
    let n = len(s);
    let start = std.json.jpos;
    let c = substr(s, std.json.jpos, 1);
    if c == "-" { std.json.jpos = std.json.jpos + 1; }
    let go = 1;
    while go == 1 and std.json.jpos < n {
        let ch = substr(s, std.json.jpos, 1);
        if index_of("0123456789.eE+-", ch) >= 0 {
            std.json.jpos = std.json.jpos + 1;
        } else {
            go = 0;
        }
    }
    str_to_num(substr(s, start, std.json.jpos - start))
}

// 解析数组
fn parse_array(s) {
    let arr = [];
    std.json.jpos = std.json.jpos + 1;  // [
    std.json.skip_ws(s);
    if substr(s, std.json.jpos, 1) == "]" {
        std.json.jpos = std.json.jpos + 1;
        return arr;
    }
    let go = 1;
    while go == 1 {
        arr = arr + [std.json.parse_value(s)];
        std.json.skip_ws(s);
        let sep = substr(s, std.json.jpos, 1);
        if sep == "," {
            std.json.jpos = std.json.jpos + 1;
        } else {
            if sep == "]" { std.json.jpos = std.json.jpos + 1; }
            go = 0;
        }
    }
    arr
}

// 解析对象
fn parse_dict(s) {
    let d = {};
    std.json.jpos = std.json.jpos + 1;  // {
    std.json.skip_ws(s);
    if substr(s, std.json.jpos, 1) == "}" {
        std.json.jpos = std.json.jpos + 1;
        return d;
    }
    let go = 1;
    while go == 1 {
        std.json.skip_ws(s);
        let key = std.json.parse_string(s);
        std.json.skip_ws(s);
        if substr(s, std.json.jpos, 1) == ":" { std.json.jpos = std.json.jpos + 1; }
        std.json.skip_ws(s);
        let val = std.json.parse_value(s);
        d[key] = val;
        std.json.skip_ws(s);
        let sep = substr(s, std.json.jpos, 1);
        if sep == "," {
            std.json.jpos = std.json.jpos + 1;
        } else {
            if sep == "}" { std.json.jpos = std.json.jpos + 1; }
            go = 0;
        }
    }
    d
}

// 解析任意值
fn parse_value(s) {
    std.json.skip_ws(s);
    let c = substr(s, std.json.jpos, 1);
    if c == "{" { return std.json.parse_dict(s); }
    if c == "[" { return std.json.parse_array(s); }
    if c == "\"" { return std.json.parse_string(s); }
    if c == "t" { std.json.jpos = std.json.jpos + 4; return true; }
    if c == "f" { std.json.jpos = std.json.jpos + 5; return false; }
    if c == "n" { std.json.jpos = std.json.jpos + 4; return "__null__"; }
    std.json.parse_number(s)
}

// 解析 JSON 文本 -> 值
fn parse(s) {
    std.json.jpos = 0;
    std.json.skip_ws(s);
    std.json.parse_value(s)
}

// 字符串转义（序列化用）
fn escape(s) {
    let out = "";
    let i = 0;
    let n = len(s);
    while i < n {
        let c = substr(s, i, 1);
        if c == "\"" { out = out + "\\\""; }
        elif c == "\\" { out = out + "\\\\"; }
        elif c == "\n" { out = out + "\\n"; }
        elif c == "\t" { out = out + "\\t"; }
        elif c == "\r" { out = out + "\\r"; }
        else { out = out + c; }
        i = i + 1;
    }
    out
}

fn stringify_arr(a) {
    let out = "[";
    let i = 0;
    while i < len(a) {
        if i > 0 { out = out + ","; }
        out = out + std.json.stringify(a[i]);
        i = i + 1;
    }
    out + "]"
}

fn stringify_dict(d) {
    let out = "{";
    let ks = keys(d);
    let i = 0;
    while i < len(ks) {
        if i > 0 { out = out + ","; }
        let k = ks[i];
        out = out + "\"" + std.json.escape(k) + "\":" + std.json.stringify(d[k]);
        i = i + 1;
    }
    out + "}"
}

// 值 -> JSON 字符串
fn stringify(v) {
    let t = type(v);
    if t == "str" {
        if v == "__null__" { return "null"; }
        return "\"" + std.json.escape(v) + "\"";
    }
    if t == "dict" { return std.json.stringify_dict(v); }
    if t == "array" { return std.json.stringify_arr(v); }
    if t == "bool" { if v { return "true"; } else { return "false"; } }
    if t == "int" or t == "float" { return to_str(v); }
    to_str(v)
}
