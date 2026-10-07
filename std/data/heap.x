// heap.x —— 二叉堆与堆排序（数组承载，元素为数字）
// 内部辅助函数均返回修改后的数组（值语义，调用处须回写）
// 数组下标赋值仅限局部变量：读-改-写回模式

// 最小堆下沉：a[0..n-1] 内从 i 下沉，返回调整后的数组
fn sift_down_min(a, n, i) {
    let go = 1;
    while go == 1 {
        let l = i * 2 + 1;
        let r = l + 1;
        let m = i;
        if l < n and a[l] < a[m] { m = l; }
        if r < n and a[r] < a[m] { m = r; }
        if m == i {
            go = 0;
        } else {
            let t = a[m];
            a[m] = a[i];
            a[i] = t;
            i = m;
        }
    }
    a
}

// 最大堆下沉：同上，比较方向相反
fn sift_down_max(a, n, i) {
    let go = 1;
    while go == 1 {
        let l = i * 2 + 1;
        let r = l + 1;
        let m = i;
        if l < n and a[l] > a[m] { m = l; }
        if r < n and a[r] > a[m] { m = r; }
        if m == i {
            go = 0;
        } else {
            let t = a[m];
            a[m] = a[i];
            a[i] = t;
            i = m;
        }
    }
    a
}

// 最小堆
class MinHeap {
    let a;
    fn new(self) { self.a = []; }
    fn size(self) { len(self.a) }
    fn peek(self) {
        if len(self.a) == 0 { -1 } else { self.a[0] }
    }
    fn push(self, v) {
        self.a = self.a + [v];
        let i = len(self.a) - 1;
        let go = 1;
        while go == 1 {
            if i == 0 {
                go = 0;
            } else {
                let p = (i - 1) / 2;
                if self.a[i] < self.a[p] {
                    let t = self.a;
                    let tmp = t[i];
                    t[i] = t[p];
                    t[p] = tmp;
                    self.a = t;
                    i = p;
                } else {
                    go = 0;
                }
            }
        }
    }
    fn pop(self) {
        let n = len(self.a);
        if n == 0 { return -1; }
        let v = self.a[0];
        if n == 1 {
            self.a = [];
        } else {
            let last = self.a[n - 1];
            let t = [];
            for k in 0..(n - 1) {
                t = t + [self.a[k]];
            }
            t[0] = last;
            self.a = std.data.sift_down_min(t, n - 1, 0);
        }
        v
    }
}

// 最大堆
class MaxHeap {
    let a;
    fn new(self) { self.a = []; }
    fn size(self) { len(self.a) }
    fn peek(self) {
        if len(self.a) == 0 { -1 } else { self.a[0] }
    }
    fn push(self, v) {
        self.a = self.a + [v];
        let i = len(self.a) - 1;
        let go = 1;
        while go == 1 {
            if i == 0 {
                go = 0;
            } else {
                let p = (i - 1) / 2;
                if self.a[i] > self.a[p] {
                    let t = self.a;
                    let tmp = t[i];
                    t[i] = t[p];
                    t[p] = tmp;
                    self.a = t;
                    i = p;
                } else {
                    go = 0;
                }
            }
        }
    }
    fn pop(self) {
        let n = len(self.a);
        if n == 0 { return -1; }
        let v = self.a[0];
        if n == 1 {
            self.a = [];
        } else {
            let last = self.a[n - 1];
            let t = [];
            for k in 0..(n - 1) {
                t = t + [self.a[k]];
            }
            t[0] = last;
            self.a = std.data.sift_down_max(t, n - 1, 0);
        }
        v
    }
}

// 就地建堆（最小堆）：返回堆化后的新数组
fn heapify(arr) {
    let a = arr;
    let n = len(a);
    let i = (n - 1) / 2;
    while i > -1 {
        a = std.data.sift_down_min(a, n, i);
        i = i - 1;
    }
    a
}

// 堆排序：asc=1 升序，asc=0 降序（不修改入参数组）
fn heap_sort(arr, asc) {
    let a = std.data.heapify(arr);
    let out = [];
    let n = len(a);
    let k = n;
    while k > 0 {
        let v = a[0];
        if k > 1 {
            let last = a[k - 1];
            let t = [];
            for j in 0..(k - 1) {
                t = t + [a[j]];
            }
            t[0] = last;
            a = std.data.sift_down_min(t, k - 1, 0);
        }
        out = out + [v];
        k = k - 1;
    }
    if asc == 1 {
        out
    } else {
        let rev = [];
        let m = len(out);
        let i = m - 1;
        while i >= 0 {
            rev = rev + [out[i]];
            i = i - 1;
        }
        rev
    }
}
