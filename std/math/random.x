// random.x —— 随机数（并入 std.math）
// 核心：双 LCG 组合（Park-Miller a=48271 + 第二 LCG a=40692，同模数 m=2147483647），
// 状态相加取模，周期更长、序列相关性更低；两个独立种子 seed/seed2 均以系统时间(now, 微秒)初始化。
let seed = now() % 2147483647.0;
let seed2 = (now() * 7.0) % 2147483647.0;

// 设置随机种子（传入整数或浮点均可），两路种子同步设定，便于复现序列
fn srand(s) {
    std.math.seed = to_float(s) % 2147483647.0;
    std.math.seed2 = (to_float(s) * 3.0 + 987654321.0) % 2147483647.0;
    return std.math.seed;
}

// 下一个 [0,1) 浮点（双 LCG 组合，更均匀更随机）
fn rand() {
    let s1 = (std.math.seed * 48271.0) % 2147483647.0;
    let s2 = (std.math.seed2 * 40692.0) % 2147483647.0;
    std.math.seed = s1;
    std.math.seed2 = s2;
    let c = (s1 + s2) % 2147483647.0;
    return c / 2147483647.0;
}

// [a,b] 闭区间整数（均匀，含两端；a>b 时自动交换）
fn rand_int(a, b) {
    if b < a {
        let tmp = a;
        a = b;
        b = tmp;
    }
    let span = to_float(b - a + 1);
    return a + to_int(std.math.rand() * span);
}

// [a,b] 闭区间浮点（含两端；a>b 时自动交换）
fn rand_float(a, b) {
    let lo = to_float(a);
    let hi = to_float(b);
    if hi < lo {
        let tmp = lo;
        lo = hi;
        hi = tmp;
    }
    return lo + std.math.rand() * (hi - lo);
}

// 随机布尔（约 50/50）
fn rand_bool() {
    if std.math.rand() < 0.5 {
        return true;
    }
    return false;
}

// 从数组随机取一个元素（空数组返回 0）
fn rand_choice(arr) {
    let n = len(arr);
    if n == 0 {
        return 0;
    }
    return arr[to_int(std.math.rand() * to_float(n))];
}

// 洗牌（Fisher-Yates），返回新数组，不改动原数组
fn shuffle(arr) {
    let out = [];
    let n = len(arr);
    let i = 0;
    while i < n {
        out = out + [arr[i]];
        i = i + 1;
    }
    let j = n - 1;
    while j > 0 {
        let k = to_int(std.math.rand() * to_float(j + 1));
        let tmp = out[j];
        out[j] = out[k];
        out[k] = tmp;
        j = j - 1;
    }
    return out;
}

// 正态分布 N(mu, sigma)（Box-Muller 变换）
fn gauss(mu, sigma) {
    let u1 = std.math.rand();
    if u1 <= 0.0 {
        u1 = 0.0000001;
    }
    let u2 = std.math.rand();
    let z = std.math.sqrt(-2.0 * std.math.calculus.ln(u1)) * std.math.trig.cos(6.283185307179586 * u2);
    return to_float(mu) + to_float(sigma) * z;
}
