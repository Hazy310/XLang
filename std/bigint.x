// bigint.x —— 大整数（十进制小端位数组，每元素 0-9，非负）
// 内部表示：数组 [个位, 十位, 百位, ...]
// 用法：import std.bigint;   std.bigint.add(a, b) 或 bigint.add(a, b)
// 提供：parse to_string from_int add sub mul div mod cmp eq gt lt ge le ne pow fact gcd is_zero

// 去除前导 0，保证至少一位
fn trim(a) {
    let i = len(a) - 1;
    while i > 0 and a[i] == 0 { i = i - 1; }
    let r = [];
    let j = 0;
    while j <= i { r = r + [a[j]]; j = j + 1; }
    r
}

// 字符串 -> 大整数（非负）
fn parse(s) {
    let n = len(s);
    let a = [];
    let i = 0;
    while i < n {
        a = a + [index_of("0123456789", substr(s, n - 1 - i, 1))];
        i = i + 1;
    }
    std.bigint.trim(a)
}

// 大整数 -> 字符串
fn to_string(a) {
    let out = "";
    let i = len(a) - 1;
    while i >= 0 {
        out = out + substr("0123456789", a[i], 1);
        i = i - 1;
    }
    out
}

// 普通整数 -> 大整数
fn from_int(n) {
    let a = [];
    let x = n;
    while x > 0 {
        a = a + [x % 10];
        x = (x - (x % 10)) / 10;
    }
    if len(a) == 0 { [0] } else { a }
}

// 比较：a>b -> 1, a==b -> 0, a<b -> -1
fn cmp(a, b) {
    if len(a) > len(b) { return 1; }
    if len(a) < len(b) { return -1; }
    let i = len(a) - 1;
    while i >= 0 {
        if a[i] > b[i] { return 1; }
        if a[i] < b[i] { return -1; }
        i = i - 1;
    }
    0
}

fn eq(a, b) { if std.bigint.cmp(a, b) == 0 { 1 } else { 0 } }
fn gt(a, b) { if std.bigint.cmp(a, b) > 0 { 1 } else { 0 } }
fn lt(a, b) { if std.bigint.cmp(a, b) < 0 { 1 } else { 0 } }
fn ge(a, b) { if std.bigint.cmp(a, b) >= 0 { 1 } else { 0 } }
fn le(a, b) { if std.bigint.cmp(a, b) <= 0 { 1 } else { 0 } }
fn ne(a, b) { if std.bigint.cmp(a, b) != 0 { 1 } else { 0 } }

// 是否为零
fn is_zero(a) {
    if len(a) == 1 and a[0] == 0 { 1 } else { 0 }
}

// 加法
fn add(a, b) {
    let r = [];
    let carry = 0;
    let n = 0;
    if len(a) > len(b) { n = len(a) } else { n = len(b) }
    let i = 0;
    while i < n {
        let da = 0;
        let db = 0;
        if i < len(a) { da = a[i]; }
        if i < len(b) { db = b[i]; }
        let s = da + db + carry;
        r = r + [s % 10];
        if s >= 10 { carry = 1 } else { carry = 0 }
        i = i + 1;
    }
    if carry == 1 { r = r + [1] }
    r
}

// 减法（要求 a >= b，否则结果未定义）
fn sub(a, b) {
    let r = [];
    let borrow = 0;
    let n = len(a);
    let i = 0;
    while i < n {
        let db = 0;
        if i < len(b) { db = b[i]; }
        let d = a[i] - db - borrow;
        if d < 0 { d = d + 10; borrow = 1 } else { borrow = 0 }
        r = r + [d];
        i = i + 1;
    }
    std.bigint.trim(r)
}

// 一位数乘法（a * d，d 为 0-9）
fn mul_digit(a, d) {
    let r = [];
    let carry = 0;
    let i = 0;
    while i < len(a) {
        let p = a[i] * d + carry;
        r = r + [p % 10];
        carry = (p - (p % 10)) / 10;
        i = i + 1;
    }
    while carry > 0 {
        r = r + [carry % 10];
        carry = (carry - (carry % 10)) / 10;
    }
    r
}

// 乘法
fn mul(a, b) {
    let r = [0];
    let i = 0;
    while i < len(b) {
        let row = std.bigint.mul_digit(a, b[i]);
        let sh = row;
        let k = 0;
        while k < i { sh = [0] + sh; k = k + 1; }
        r = std.bigint.add(r, sh);
        i = i + 1;
    }
    std.bigint.trim(r)
}

// 长除法：返回 [商, 余]
fn divmod(a, b) {
    if std.bigint.is_zero(b) { return [[0], [0]]; }
    let q = [];
    let rem = [0];
    let i = len(a) - 1;
    while i >= 0 {
        rem = std.bigint.add(std.bigint.mul_digit(rem, 10), [a[i]]);
        let d = 0;
        while d < 10 and std.bigint.cmp(rem, std.bigint.mul_digit(b, d + 1)) >= 0 {
            d = d + 1;
        }
        q = [d] + q;
        if d > 0 { rem = std.bigint.sub(rem, std.bigint.mul_digit(b, d)); }
        i = i - 1;
    }
    [std.bigint.trim(q), std.bigint.trim(rem)]
}

fn div(a, b) { std.bigint.divmod(a, b)[0] }
fn mod(a, b) { std.bigint.divmod(a, b)[1] }

// 幂：a^n（n 为非负整数，快速幂）
fn pow(a, n) {
    let r = [1];
    let base = a;
    let e = n;
    while e > 0 {
        if e % 2 == 1 { r = std.bigint.mul(r, base); }
        base = std.bigint.mul(base, base);
        e = (e - (e % 2)) / 2;
    }
    r
}

// 阶乘：n!（n 为非负整数）
fn fact(n) {
    let r = [1];
    let k = 2;
    while k <= n {
        r = std.bigint.mul(r, std.bigint.from_int(k));
        k = k + 1;
    }
    r
}

// 最大公约数（欧几里得）
fn gcd(a, b) {
    let x = a;
    let y = b;
    while !std.bigint.is_zero(y) {
        let t = std.bigint.mod(x, y);
        x = y;
        y = t;
    }
    x
}
