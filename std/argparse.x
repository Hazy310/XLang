// argparse.x —— 命令行参数解析（std 根目录模块）
// 用法：import std.argparse;  std.argparse.parse(argv)
// 规则：
//   --name value / --name=value  -> 具名参数（字符串）
//   -n value 或 -n=value         -> 单字符参数
//   --flag（无值）               -> 布尔 true
//   位置参数                      -> 聚合到 out["args"] 数组
// 辅助：has(d,name) 判断存在；get(d,name,def) 取值（无则默认）

// 解析参数数组 -> dict
fn parse(argv) {
    let out = {};
    let pos = [];
    let i = 0;
    let n = len(argv);
    while i < n {
        let a = argv[i];
        if starts_with(a, "--") {
            let eq = index_of(a, "=");
            let name = "";
            let val = "";
            let has_val = 0;
            if eq >= 0 {
                name = substr(a, 2, eq - 2);
                val = substr(a, eq + 1, len(a) - eq - 1);
                has_val = 1;
            } else {
                name = substr(a, 2, len(a) - 2);
                if i + 1 < n and !starts_with(argv[i + 1], "-") {
                    val = argv[i + 1];
                    has_val = 1;
                    i = i + 1;
                }
            }
            if has_val { out[name] = val; } else { out[name] = true; }
        }
        elif starts_with(a, "-") and len(a) > 1 {
            let eq = index_of(a, "=");
            if eq >= 0 {
                let name = substr(a, 1, eq - 1);
                out[name] = substr(a, eq + 1, len(a) - eq - 1);
            } else {
                let name = substr(a, 1, len(a) - 1);
                if i + 1 < n and !starts_with(argv[i + 1], "-") {
                    out[name] = argv[i + 1];
                    i = i + 1;
                } else {
                    out[name] = true;
                }
            }
        }
        else {
            pos = pos + [a];
        }
        i = i + 1;
    }
    out["args"] = pos;
    out
}

// 判断参数 dict 中是否存在某键
fn has(d, name) {
    let ks = keys(d);
    let i = 0;
    while i < len(ks) {
        if ks[i] == name { return true; }
        i = i + 1;
    }
    false
}

// 取值：存在返回，不存在返回默认
fn get(d, name, def) {
    if std.argparse.has(d, name) {
        return d[name];
    }
    def
}
