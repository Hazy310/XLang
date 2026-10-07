// csv.x —— CSV 读写（std 根目录模块）
// 用法：import std.csv;
//   std.csv.parse(text)        -> 二维数组（处理引号/逗号/换行转义）
//   std.csv.read(file)         -> 读文件并解析
//   std.csv.write(rows)        -> 二维数组 -> CSV 字符串（字段自动转义）
//   std.csv.save(file, rows)   -> 写入 CSV 文件

// 解析 CSV 文本 -> 二维数组
fn parse(text) {
    let rows = [];
    let row = [];
    let field = "";
    let inq = 0;
    let i = 0;
    let n = len(text);
    while i < n {
        let c = substr(text, i, 1);
        if inq == 1 {
            if c == "\"" {
                if i + 1 < n and substr(text, i + 1, 1) == "\"" {
                    field = field + "\"";
                    i = i + 1;
                } else {
                    inq = 0;
                }
            } else {
                field = field + c;
            }
        } else {
            if c == "\"" {
                inq = 1;
            }
            elif c == "," {
                row = row + [field];
                field = "";
            }
            elif c == "\n" {
                row = row + [field];
                rows = rows + [row];
                row = [];
                field = "";
            }
            elif c == "\r" {
                // 忽略回车（兼容 CRLF）
            }
            else {
                field = field + c;
            }
        }
        i = i + 1;
    }
    // 末尾未换行时补最后一行
    if field != "" or len(row) > 0 {
        row = row + [field];
        rows = rows + [row];
    }
    rows
}

// 读文件并解析为二维数组
fn read(file) {
    std.csv.parse(read_file(file))
}

// 字段转义：含逗号/引号/换行时加引号，内部引号加倍
fn esc_field(f) {
    let s = to_str(f);
    if index_of(s, ",") >= 0 or index_of(s, "\"") >= 0 or index_of(s, "\n") >= 0 or index_of(s, "\r") >= 0 {
        let out = "\"";
        let i = 0;
        let n = len(s);
        while i < n {
            let c = substr(s, i, 1);
            if c == "\"" { out = out + "\"\""; }
            else { out = out + c; }
            i = i + 1;
        }
        return out + "\"";
    }
    s
}

// 二维数组 -> CSV 字符串
fn write(rows) {
    let out = "";
    let i = 0;
    while i < len(rows) {
        if i > 0 { out = out + "\n"; }
        let row = rows[i];
        let j = 0;
        while j < len(row) {
            if j > 0 { out = out + ","; }
            out = out + std.csv.esc_field(row[j]);
            j = j + 1;
        }
        i = i + 1;
    }
    out
}

// 写入 CSV 文件
fn save(file, rows) {
    write_file(file, std.csv.write(rows))
}
