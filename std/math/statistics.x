// statistics.x —— 描述性统计（并入 std.math，import std.math 即可用）
// 求和 / 均值 / 中位数 / 方差 / 标准差 / 极值

// 数组元素求和，空数组返回 0
fn sum(arr) {
    let s = 0.0;
    for i in 0..len(arr) {
        s = s + arr[i];
    }
    return s;
}

// 算术均值：sum/len，空数组返回 0
fn mean(arr) {
    let n = len(arr);
    if n == 0 {
        return 0;
    }
    return to_float(std.math.sum(arr)) / to_float(n);
}

// 中位数：升序排序后取中位，偶数个取中间两数均值（float）
fn median(arr) {
    let n = len(arr);
    if n == 0 {
        return 0;
    }
    let s = sort(arr);
    if n % 2 == 1 {
        return s[n / 2];
    }
    return (s[n / 2 - 1] + s[n / 2]) / 2.0;
}

// 总体方差：Σ(x-μ)²/n
fn variance(arr) {
    let n = len(arr);
    if n == 0 {
        return 0;
    }
    let mu = std.math.mean(arr);
    let s = 0.0;
    for i in 0..n {
        let d = to_float(arr[i]) - mu;
        s = s + d * d;
    }
    return s / to_float(n);
}

// 总体标准差：sqrt(variance)
fn stddev(arr) {
    return std.math.sqrt(std.math.variance(arr));
}

// 数组最小值，空数组返回 0
fn arr_min(arr) {
    let n = len(arr);
    if n == 0 {
        return 0;
    }
    let m = arr[0];
    for i in 1..n {
        if arr[i] < m {
            m = arr[i];
        }
    }
    return m;
}

// 数组最大值，空数组返回 0
fn arr_max(arr) {
    let n = len(arr);
    if n == 0 {
        return 0;
    }
    let m = arr[0];
    for i in 1..n {
        if arr[i] > m {
            m = arr[i];
        }
    }
    return m;
}
