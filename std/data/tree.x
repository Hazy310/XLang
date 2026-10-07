// tree.x —— 二叉搜索树 class BST（3 个平行数组承载，全迭代实现）
// vals/left/right 平行存储；节点下标即数组下标；-1 表示空子节点
// 实例字段改数组一律"读-改-写回"：let t = self.x; t[i]=v; self.x = t;
// 重复元素：插入返回 false 且不改变树

class BST {
    let vals;
    let left;
    let right;
    let root;

    fn new(self) {
        self.vals = [];
        self.left = [];
        self.right = [];
        self.root = -1;
    }

    // 删除节点 cur（父 p，方向 d：0=左 1=右；p=-1 表示 cur 是根），用 child 顶替
    fn _transplant(self, p, d, child) {
        if p == -1 {
            self.root = child;
        } else {
            if d == 0 {
                let t = self.left;
                t[p] = child;
                self.left = t;
            } else {
                let t = self.right;
                t[p] = child;
                self.right = t;
            }
        }
    }

    // 插入：重复返回 false；成功返回 true
    fn insert(self, v) {
        if self.root == -1 {
            self.vals = [v];
            self.left = [-1];
            self.right = [-1];
            self.root = 0;
            return true;
        }
        let cur = self.root;
        let go = 1;
        while go == 1 {
            let cv = self.vals[cur];
            if v == cv {
                go = 0;
                return false;
            }
            if v < cv {
                let lc = self.left[cur];
                if lc == -1 {
                    let idx = len(self.vals);
                    let nv = self.vals + [v];
                    let nr = self.right + [-1];
                    let nl = self.left + [-1];
                    nl[cur] = idx;
                    self.vals = nv;
                    self.right = nr;
                    self.left = nl;
                    go = 0;
                } else {
                    cur = lc;
                }
            } else {
                let rc = self.right[cur];
                if rc == -1 {
                    let idx = len(self.vals);
                    let nv = self.vals + [v];
                    let nr = self.right + [-1];
                    let nl = self.left + [-1];
                    nr[cur] = idx;
                    self.vals = nv;
                    self.left = nl;
                    self.right = nr;
                    go = 0;
                } else {
                    cur = rc;
                }
            }
        }
        true
    }

    fn contains(self, v) {
        let cur = self.root;
        let go = 1;
        while go == 1 {
            if cur == -1 {
                go = 0;
                return false;
            }
            let cv = self.vals[cur];
            if v == cv {
                go = 0;
                return true;
            }
            if v < cv { cur = self.left[cur]; } else { cur = self.right[cur]; }
        }
        false
    }

    fn min(self) {
        if self.root == -1 { return -1; }
        let cur = self.root;
        let go = 1;
        while go == 1 {
            let lc = self.left[cur];
            if lc == -1 {
                go = 0;
            } else {
                cur = lc;
            }
        }
        self.vals[cur]
    }

    fn max(self) {
        if self.root == -1 { return -1; }
        let cur = self.root;
        let go = 1;
        while go == 1 {
            let rc = self.right[cur];
            if rc == -1 {
                go = 0;
            } else {
                cur = rc;
            }
        }
        self.vals[cur]
    }

    // 删除值 v：找到并删返回 true；找不到返回 false
    fn remove(self, v) {
        if self.root == -1 { return false; }
        let cur = self.root;
        let p = -1;
        let d = 0;
        let go = 1;
        while go == 1 {
            if cur == -1 {
                go = 0;
                return false;
            }
            let cv = self.vals[cur];
            if v == cv {
                go = 0;
            } else {
                if v < cv {
                    p = cur;
                    d = 0;
                    cur = self.left[cur];
                } else {
                    p = cur;
                    d = 1;
                    cur = self.right[cur];
                }
            }
        }
        let lc = self.left[cur];
        let rc = self.right[cur];
        if lc == -1 {
            self._transplant(p, d, rc);
        } else {
            if rc == -1 {
                self._transplant(p, d, lc);
            } else {
                // 双子：右子树最小后继 s，父 sp
                let sp = cur;
                let s = rc;
                let g = 1;
                while g == 1 {
                    let sl = self.left[s];
                    if sl == -1 {
                        g = 0;
                    } else {
                        sp = s;
                        s = sl;
                    }
                }
                let t = self.vals;
                t[cur] = self.vals[s];
                self.vals = t;
                // s 无左子，用其右子顶替 s；sp==cur 时 s 是右子(d=1)，否则是左子(d=0)
                let sd = 0;
                if sp == cur { sd = 1; }
                self._transplant(sp, sd, self.right[s]);
            }
        }
        true
    }

    // 前序：显式栈，先压右再压左
    fn preorder(self) {
        let out = [];
        if self.root == -1 { return out; }
        let st = [self.root];
        let go = 1;
        while go == 1 {
            let ln = len(st);
            if ln == 0 {
                go = 0;
            } else {
                let n = st[ln - 1];
                let ns = [];
                for k in 0..(ln - 1) { ns = ns + [st[k]]; }
                st = ns;
                out = out + [self.vals[n]];
                let rc = self.right[n];
                let lc = self.left[n];
                if rc != -1 { st = st + [rc]; }
                if lc != -1 { st = st + [lc]; }
            }
        }
        out
    }

    // 中序：cur 一路压栈向左，弹出后转右
    fn inorder(self) {
        let out = [];
        if self.root == -1 { return out; }
        let st = [];
        let cur = self.root;
        let go = 1;
        while go == 1 {
            if cur != -1 {
                st = st + [cur];
                cur = self.left[cur];
            } else {
                let ln = len(st);
                if ln == 0 {
                    go = 0;
                } else {
                    let n = st[ln - 1];
                    let ns = [];
                    for k in 0..(ln - 1) { ns = ns + [st[k]]; }
                    st = ns;
                    out = out + [self.vals[n]];
                    cur = self.right[n];
                }
            }
        }
        out
    }

    // 后序：双栈 nodes/flags；flag=1 取值，否则压 (n,1)+右(0)+左(0)
    fn postorder(self) {
        let out = [];
        if self.root == -1 { return out; }
        let nodes = [self.root];
        let flags = [0];
        let go = 1;
        while go == 1 {
            let ln = len(nodes);
            if ln == 0 {
                go = 0;
            } else {
                let n = nodes[ln - 1];
                let f = flags[ln - 1];
                let nn = [];
                let ff = [];
                for k in 0..(ln - 1) {
                    nn = nn + [nodes[k]];
                    ff = ff + [flags[k]];
                }
                nodes = nn;
                flags = ff;
                if f == 1 {
                    out = out + [self.vals[n]];
                } else {
                    nodes = nodes + [n];
                    flags = flags + [1];
                    let rc = self.right[n];
                    if rc != -1 { nodes = nodes + [rc]; flags = flags + [0]; }
                    let lc = self.left[n];
                    if lc != -1 { nodes = nodes + [lc]; flags = flags + [0]; }
                }
            }
        }
        out
    }

    // 层序：队列数组 + 头指针
    fn levelorder(self) {
        let out = [];
        if self.root == -1 { return out; }
        let q = [self.root];
        let head = 0;
        while head < len(q) {
            let n = q[head];
            head = head + 1;
            out = out + [self.vals[n]];
            let lc = self.left[n];
            let rc = self.right[n];
            if lc != -1 { q = q + [lc]; }
            if rc != -1 { q = q + [rc]; }
        }
        out
    }
}
