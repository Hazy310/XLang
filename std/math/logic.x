// logic.x —— 布尔逻辑连接词（并入 std.math）
// 输入按真值归一化为布尔后组合，返回 true/false

// 异或
fn xor(a, b) {
    let x = false;
    if a {
        x = true;
    }
    let y = false;
    if b {
        y = true;
    }
    if x != y {
        return true;
    }
    return false;
}

// 蕴含 a→b
fn implies(a, b) {
    let x = false;
    if a {
        x = true;
    }
    let y = false;
    if b {
        y = true;
    }
    if x == false {
        return true;
    }
    return y;
}

// 等价 a↔b
fn iff(a, b) {
    let x = false;
    if a {
        x = true;
    }
    let y = false;
    if b {
        y = true;
    }
    if x == y {
        return true;
    }
    return false;
}

// 与非 !(a and b)
fn nand(a, b) {
    let x = false;
    if a {
        x = true;
    }
    let y = false;
    if b {
        y = true;
    }
    if !(x and y) {
        return true;
    }
    return false;
}

// 或非 !(a or b)
fn nor(a, b) {
    let x = false;
    if a {
        x = true;
    }
    let y = false;
    if b {
        y = true;
    }
    if !(x or y) {
        return true;
    }
    return false;
}

// 多数表决：三个中 true 数 >= 2 返回 true
fn majority(a, b, c) {
    let x = false;
    if a {
        x = true;
    }
    let y = false;
    if b {
        y = true;
    }
    let z = false;
    if c {
        z = true;
    }
    let cnt = 0;
    if x {
        cnt = cnt + 1;
    }
    if y {
        cnt = cnt + 1;
    }
    if z {
        cnt = cnt + 1;
    }
    if cnt >= 2 {
        return true;
    }
    return false;
}
