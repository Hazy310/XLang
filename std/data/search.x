// search.x —— 二分/插值查找与开放寻址哈希表
// 键限字符串、值限数字；哈希表容量固定 64，线性探测
// 注意：字符串内字面量 $ 须写 $$（XLang 字符串插值转义）

// 字符码辅助：单字符 -> 可打印 ASCII 码；非表内字符（含非 ASCII）返回 31.0
// 字母表内联（空格 32 到 ~ 126，共 95 个）；模块顶层 let 在函数体内不可见，故字面量内联
fn _hchar(ch) {
    let a = " !\"#$$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";
    index_of(a, ch) + 32.0
}

// 字符串哈希（32 位折叠后对 64 取模）：返回桶下标
fn _hhash(key) {
    let h = 0.0;
    let n = len(key);
    let i = 0;
    while i < n {
        let c = std.data._hchar(substr(key, i, 1));
        h = (h * 31.0 + c) % 4294967296.0;
        i = i + 1;
    }
    to_int(h % 64.0)
}

// 下界：升序数组中第一个 >= x 的下标，无则返回 len(arr)
fn binary_search_lower(arr, x) {
    let lo = 0;
    let hi = len(arr);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if arr[mid] < x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

// 上界：升序数组中第一个 > x 的下标，无则返回 len(arr)
fn upper_bound(arr, x) {
    let lo = 0;
    let hi = len(arr);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if arr[mid] <= x {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

// 插值查找：升序数组，命中返回下标，否则 -1
fn interpolation_search(arr, x) {
    let n = len(arr);
    if n == 0 { return -1; }
    let lo = 0;
    let hi = n - 1;
    while lo <= hi and arr[lo] <= x and arr[hi] >= x {
        if arr[hi] == arr[lo] {
            if arr[lo] == x { return lo; }
            return -1;
        }
        let pos = lo + to_int((to_float(hi - lo) * (to_float(x) - to_float(arr[lo]))) / (to_float(arr[hi]) - to_float(arr[lo])));
        if arr[pos] == x { return pos; }
        if arr[pos] < x {
            lo = pos + 1;
        } else {
            hi = pos - 1;
        }
    }
    -1
}

// 开放寻址哈希表：键字符串、值数字，容量 64
// used: 0=空，1=占用，2=墓碑
class HashTable {
    let keys;
    let used;
    let vals;
    fn new(self) {
        let k = [];
        let u = [];
        let v = [];
        let i = 0;
        while i < 64 {
            k = k + [""];
            u = u + [0];
            v = v + [0];
            i = i + 1;
        }
        self.keys = k;
        self.used = u;
        self.vals = v;
    }
    // 写入/更新；桶用尽返回 -1，正常返回 1
    fn put(self, key, val) {
        let idx = std.data._hhash(key);
        let i = 0;
        while i < 64 {
            let slot = (idx + i) % 64;
            if self.used[slot] == 1 and self.keys[slot] == key {
                let tv = self.vals;
                tv[slot] = val;
                self.vals = tv;
                return 1;
            }
            if self.used[slot] == 0 or self.used[slot] == 2 {
                let tk = self.keys;
                tk[slot] = key;
                self.keys = tk;
                let tv = self.vals;
                tv[slot] = val;
                self.vals = tv;
                let tu = self.used;
                tu[slot] = 1;
                self.used = tu;
                return 1;
            }
            i = i + 1;
        }
        -1
    }
    // 取值；不存在返回 -1
    fn get(self, key) {
        let idx = std.data._hhash(key);
        let i = 0;
        while i < 64 {
            let slot = (idx + i) % 64;
            if self.used[slot] == 1 and self.keys[slot] == key {
                return self.vals[slot];
            }
            if self.used[slot] == 0 {
                return -1;
            }
            i = i + 1;
        }
        -1
    }
    // 是否包含键
    fn contains(self, key) {
        let idx = std.data._hhash(key);
        let i = 0;
        while i < 64 {
            let slot = (idx + i) % 64;
            if self.used[slot] == 1 and self.keys[slot] == key {
                return true;
            }
            if self.used[slot] == 0 {
                return false;
            }
            i = i + 1;
        }
        false
    }
    // 删除（置墓碑）；命中返回 1，未命中返回 0
    fn remove(self, key) {
        let idx = std.data._hhash(key);
        let i = 0;
        while i < 64 {
            let slot = (idx + i) % 64;
            if self.used[slot] == 1 and self.keys[slot] == key {
                let tu = self.used;
                tu[slot] = 2;
                self.used = tu;
                return 1;
            }
            if self.used[slot] == 0 {
                return 0;
            }
            i = i + 1;
        }
        0
    }
}
