// sort.x —— 排序算法集合（元素为数字，全部返回新数组，不修改入参）
// cmp 为双参闭包，返回 true 表示 a 应排在 b 前；升序用 std.data.cmp_asc

// 升序比较器：a < b
fn cmp_asc(a, b) { a < b }

// 降序比较器：a > b
fn cmp_desc(a, b) { a > b }

// 快速排序（递归闭包，比较器必传）
fn quick_sort(arr, cmp) {
    let qs = fn(a) {
        let m = len(a);
        if m <= 1 {
            a
        } else {
            let pivot = a[0];
            let lt = [];
            let ge = [];
            let j = 1;
            while j < m {
                if cmp(a[j], pivot) {
                    lt = lt + [a[j]];
                } else {
                    ge = ge + [a[j]];
                }
                j = j + 1;
            }
            qs(lt) + [pivot] + qs(ge)
        }
    };
    qs(arr)
}

// 归并排序（递归闭包，比较器必传）
fn merge_sort(arr, cmp) {
    let ms = fn(a) {
        let m = len(a);
        if m <= 1 {
            a
        } else {
            let mid = m / 2;
            let left = [];
            let right = [];
            let i = 0;
            while i < mid {
                left = left + [a[i]];
                i = i + 1;
            }
            while i < m {
                right = right + [a[i]];
                i = i + 1;
            }
            let sl = ms(left);
            let sr = ms(right);
            let out = [];
            let p = 0;
            let q = 0;
            let nl = len(sl);
            let nr = len(sr);
            while p < nl and q < nr {
                if cmp(sl[p], sr[q]) {
                    out = out + [sl[p]];
                    p = p + 1;
                } else {
                    out = out + [sr[q]];
                    q = q + 1;
                }
            }
            while p < nl {
                out = out + [sl[p]];
                p = p + 1;
            }
            while q < nr {
                out = out + [sr[q]];
                q = q + 1;
            }
            out
        }
    };
    ms(arr)
}

// 插入排序（升序，无比较器；就地修改局部副本）
fn insertion_sort(arr) {
    let a = arr;
    let n = len(a);
    let i = 1;
    while i < n {
        let key = a[i];
        let j = i - 1;
        while j >= 0 and a[j] > key {
            a[j + 1] = a[j];
            j = j - 1;
        }
        a[j + 1] = key;
        i = i + 1;
    }
    a
}

// 计数排序（非负整数，升序）
fn counting_sort(arr) {
    let n = len(arr);
    if n == 0 { return []; }
    let maxv = arr[0];
    let i = 1;
    while i < n {
        if arr[i] > maxv { maxv = arr[i]; }
        i = i + 1;
    }
    let counts = [];
    let k = 0;
    while k <= maxv {
        counts = counts + [0];
        k = k + 1;
    }
    let j = 0;
    while j < n {
        let v = arr[j];
        counts[v] = counts[v] + 1;
        j = j + 1;
    }
    let out = [];
    let k2 = 0;
    while k2 <= maxv {
        let c = 0;
        while c < counts[k2] {
            out = out + [k2];
            c = c + 1;
        }
        k2 = k2 + 1;
    }
    out
}

// 基数排序（非负整数，低位到高位稳定收集）
fn radix_sort(arr) {
    let n = len(arr);
    if n == 0 { return arr; }
    let a = arr;
    let maxv = a[0];
    let i = 1;
    while i < n {
        if a[i] > maxv { maxv = a[i]; }
        i = i + 1;
    }
    let exp = 1;
    while maxv / exp >= 1 {
        let out = [];
        let d = 0;
        while d < 10 {
            let j = 0;
            while j < n {
                let v = a[j];
                if (v / exp) % 10 == d {
                    out = out + [v];
                }
                j = j + 1;
            }
            d = d + 1;
        }
        a = out;
        exp = exp * 10;
    }
    a
}
