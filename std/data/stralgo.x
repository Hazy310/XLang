// stralgo.x —— 字符串算法（字符安全，单字符一律 substr(s,i,1)）

// 前缀函数（KMP）：返回 int 数组 pi
fn prefix_function(s) {
    let pi = [0];
    let n = len(s);
    let i = 1;
    while i < n {
        let j = pi[i - 1];
        while j > 0 and substr(s, i, 1) != substr(s, j, 1) {
            j = pi[j - 1];
        }
        if substr(s, i, 1) == substr(s, j, 1) {
            j = j + 1;
        }
        pi = pi + [j];
        i = i + 1;
    }
    pi
}

// KMP 查找：pat 在 text 中首次出现下标；pat 空返回 0，未命中返回 -1
fn kmp_search(text, pat) {
    let plen = len(pat);
    if plen == 0 { return 0; }
    let pi = std.data.prefix_function(pat);
    let j = 0;
    let n = len(text);
    let i = 0;
    while i < n {
        while j > 0 and substr(text, i, 1) != substr(pat, j, 1) {
            j = pi[j - 1];
        }
        if substr(text, i, 1) == substr(pat, j, 1) {
            j = j + 1;
        }
        if j == plen {
            return i - j + 1;
        }
        i = i + 1;
    }
    -1
}

// 编辑距离（两行滚动 DP），返回 int
fn edit_distance(a, b) {
    let nb = len(b);
    let prev = [];
    let j = 0;
    while j <= nb {
        prev = prev + [j];
        j = j + 1;
    }
    let na = len(a);
    let i = 1;
    while i <= na {
        let cur = [i];
        let j2 = 1;
        while j2 <= nb {
            let cost = 1;
            if substr(a, i - 1, 1) == substr(b, j2 - 1, 1) {
                cost = 0;
            }
            let m1 = prev[j2] + 1;
            let m2 = cur[j2 - 1] + 1;
            let m3 = prev[j2 - 1] + cost;
            let mn = m1;
            if m2 < mn { mn = m2; }
            if m3 < mn { mn = m3; }
            cur = cur + [mn];
            j2 = j2 + 1;
        }
        prev = cur;
        i = i + 1;
    }
    prev[nb]
}

// 最长公共子序列（返回字符串；平局规则：table[i-1][j] >= table[i][j-1] 时向上 i-1）
fn lcs(a, b) {
    let na = len(a);
    let nb = len(b);
    let prev = [];
    let j = 0;
    while j <= nb {
        prev = prev + [0];
        j = j + 1;
    }
    let table = [prev];
    let i = 1;
    while i <= na {
        let row = [0];
        let j2 = 1;
        while j2 <= nb {
            if substr(a, i - 1, 1) == substr(b, j2 - 1, 1) {
                row = row + [prev[j2 - 1] + 1];
            } else {
                let m1 = prev[j2];
                let m2 = row[j2 - 1];
                let mx = m1;
                if m2 > mx { mx = m2; }
                row = row + [mx];
            }
            j2 = j2 + 1;
        }
        table = table + [row];
        prev = row;
        i = i + 1;
    }
    let i2 = na;
    let j3 = nb;
    let res = "";
    while i2 > 0 and j3 > 0 {
        let ca = substr(a, i2 - 1, 1);
        let cb = substr(b, j3 - 1, 1);
        if ca == cb {
            res = ca + res;
            i2 = i2 - 1;
            j3 = j3 - 1;
        } else {
            // 平局规则：仅当上方严格大于左方才向上（i-1），否则向左（j-1）。
            // 此方向使回溯得到规范期望 BDAB（而非 BCBA）。
            if table[i2 - 1][j3] > table[i2][j3 - 1] {
                i2 = i2 - 1;
            } else {
                j3 = j3 - 1;
            }
        }
    }
    res
}
