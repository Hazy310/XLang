// stats.x —— 统计（std 根目录模块）
// 用法：import std.stats;  数组为数字数组
// 提供：sum mean median mode range variance stddev min max percentile sort

// 插入排序（返回新数组，不修改原数组）
fn sort(a) {
    let n = len(a);
    let out = [];
    let i = 0;
    while i < n { out = out + [a[i]]; i = i + 1; }
    let i = 1;
    while i < n {
        let key = out[i];
        let j = i - 1;
        while j >= 0 and out[j] > key {
            out[j + 1] = out[j];
            j = j - 1;
        }
        out[j + 1] = key;
        i = i + 1;
    }
    out
}

fn sum(a) {
    let s = 0;
    let i = 0;
    while i < len(a) { s = s + a[i]; i = i + 1; }
    s
}

fn mean(a) {
    if len(a) == 0 { return 0; }
    to_float(std.stats.sum(a)) / to_float(len(a))
}

fn min(a) {
    if len(a) == 0 { return 0; }
    let m = a[0];
    let i = 1;
    while i < len(a) { if a[i] < m { m = a[i]; } i = i + 1; }
    m
}

fn max(a) {
    if len(a) == 0 { return 0; }
    let m = a[0];
    let i = 1;
    while i < len(a) { if a[i] > m { m = a[i]; } i = i + 1; }
    m
}

fn range(a) {
    to_float(std.stats.max(a)) - to_float(std.stats.min(a))
}

fn median(a) {
    let n = len(a);
    if n == 0 { return 0; }
    let srt = std.stats.sort(a);
    if n % 2 == 1 {
        return to_float(srt[n / 2]);
    }
    (to_float(srt[n / 2 - 1]) + to_float(srt[n / 2])) / 2.0
}

// 众数：出现最多的值（返回原值）
fn mode(a) {
    if len(a) == 0 { return 0; }
    let best = a[0];
    let bestc = 0;
    let i = 0;
    while i < len(a) {
        let c = 0;
        let j = 0;
        while j < len(a) { if a[j] == a[i] { c = c + 1; } j = j + 1; }
        if c > bestc { bestc = c; best = a[i]; }
        i = i + 1;
    }
    best
}

// 方差（总体）
fn variance(a) {
    let n = len(a);
    if n == 0 { return 0; }
    let m = std.stats.mean(a);
    let s = 0.0;
    let i = 0;
    while i < n {
        let d = to_float(a[i]) - m;
        s = s + d * d;
        i = i + 1;
    }
    s / to_float(n)
}

// 平方根（牛顿迭代，自实现避免外部依赖）
fn sqrt(n) {
    if n < 0 { return 0; }
    if n == 0 { return 0; }
    let x = to_float(n);
    for i in 0..10 {
        x = (x + to_float(n) / x) / 2.0;
    }
    x
}

// 标准差（总体）
fn stddev(a) {
    std.stats.sqrt(std.stats.variance(a))
}

// 百分位（线性插值），p 为 0~100
fn percentile(a, p) {
    let n = len(a);
    if n == 0 { return 0; }
    let srt = std.stats.sort(a);
    let rank = (to_float(p) / 100.0) * to_float(n - 1);
    let low = to_int(rank);
    let frac = rank - to_float(low);
    if low >= n - 1 { return to_float(srt[n - 1]); }
    to_float(srt[low]) + frac * (to_float(srt[low + 1]) - to_float(srt[low]))
}
