// unionfind.x —— 并查集 class UnionFind（路径压缩 + 按秩合并）
// 平局规则固定：rank 相等时 parent[rb]=ra 且 rank[ra]++（保证确定性）

class UnionFind {
    let parent;
    let rank;
    let count;

    fn new(self, n) {
        let p = [];
        let r = [];
        for i in 0..n {
            p = p + [i];
            r = r + [0];
        }
        self.parent = p;
        self.rank = r;
        self.count = n;
    }

    // 找根并做路径压缩（两次遍历：先找根，再把路径节点直接挂根）
    fn find(self, x) {
        let r = x;
        while self.parent[r] != r { r = self.parent[r]; }
        let c = x;
        while self.parent[c] != c {
            let nx = self.parent[c];
            let t = self.parent;
            t[c] = r;
            self.parent = t;
            c = nx;
        }
        r
    }

    // 合并 a,b 所在集合；已连通返回 false，否则合并并返回 true
    fn union(self, a, b) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb { return false; }
        if self.rank[ra] < self.rank[rb] {
            let t = self.parent;
            t[ra] = rb;
            self.parent = t;
        } else {
            if self.rank[rb] < self.rank[ra] {
                let t = self.parent;
                t[rb] = ra;
                self.parent = t;
            } else {
                let t = self.parent;
                t[rb] = ra;
                self.parent = t;
                let tr = self.rank;
                tr[ra] = tr[ra] + 1;
                self.rank = tr;
            }
        }
        self.count = self.count - 1;
        true
    }

    fn connected(self, a, b) { self.find(a) == self.find(b) }

    fn component_count(self) { self.count }
}
