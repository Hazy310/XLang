// set_theory.x —— 集合运算（并入 std.math，集合用无重复数值数组表示，元素为标量）
// 成员 / 并 / 交 / 差 / 对称差 / 子集 / 相等

// 是否包含元素 v：包含返回 1 否则 0
fn set_has(a, v) {
    for i in 0..len(a) {
        if a[i] == v {
            return 1;
        }
    }
    return 0;
}

// 并集：先拷贝 a，再把 b 中不重复的元素追加
fn set_union(a, b) {
    let out = [];
    for i in 0..len(a) {
        out = out + [a[i]];
    }
    for i in 0..len(b) {
        if std.math.set_has(out, b[i]) == 0 {
            out = out + [b[i]];
        }
    }
    return out;
}

// 交集：a 中也在 b 中的元素
fn set_intersect(a, b) {
    let out = [];
    for i in 0..len(a) {
        if std.math.set_has(b, a[i]) == 1 {
            out = out + [a[i]];
        }
    }
    return out;
}

// 差集 a-b：a 中不在 b 中的元素
fn set_diff(a, b) {
    let out = [];
    for i in 0..len(a) {
        if !std.math.set_has(b, a[i]) {
            out = out + [a[i]];
        }
    }
    return out;
}

// 对称差：(a-b) ∪ (b-a)
fn set_symdiff(a, b) {
    return std.math.set_union(std.math.set_diff(a, b), std.math.set_diff(b, a));
}

// 子集判断 a⊆b：成立返回 1 否则 0
fn set_subset(a, b) {
    for i in 0..len(a) {
        if std.math.set_has(b, a[i]) == 0 {
            return 0;
        }
    }
    return 1;
}

// 集合相等：a⊆b 且 b⊆a
fn set_equal(a, b) {
    if std.math.set_subset(a, b) == 1 and std.math.set_subset(b, a) == 1 {
        return 1;
    }
    return 0;
}
