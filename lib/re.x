// lib/re.x —— 字符串正则-like 便捷封装
// 基于内置字符安全函数：split_str / index_of / len

// 分割字符串：按分隔符拆分为字符串数组（字符安全，支持中文）
fn split(s, sep) {
    return split_str(s, sep);
}

// 是否包含子串
fn contains(s, sub) {
    return index_of(s, sub) != -1;
}

// 替换所有子串：将 s 中所有 old 替换为 rep（字符安全，支持中文）
// 空 old 视为不替换，直接返回原串
fn replace(s, old, rep) {
    if old == "" {
        return s;
    }
    let parts = lib::re::split(s, old);
    // 未找到分隔符：split 返回单元素数组，直接返回原串
    if len(parts) == 1 {
        return s;
    }
    let result = "";
    for i in 0..len(parts) {
        result = result + parts[i];
        if i < len(parts) - 1 {
            result = result + rep;
        }
    }
    return result;
}

// 仅替换第一处：s 中第一个 old 替换为 rep，其余保留
fn replace_first(s, old, rep) {
    if old == "" {
        return s;
    }
    let parts = lib::re::split(s, old);
    if len(parts) == 1 {
        return s;
    }
    // 首段 + rep + 其余段（其余段之间用 old 重新连接，保留剩余分隔符）
    let result = parts[0] + rep;
    for i in 1..len(parts) {
        result = result + old + parts[i];
    }
    return result;
}