// graph.x —— 图算法（邻接表 / 邻接矩阵，纯数组实现）
// topological_sort(Kahn) / kruskal_mst / prim_mst / scc(Kosaraju)

// 拓扑排序（Kahn）：adj 为 0..n-1 的邻接表；有环返回 []
fn topological_sort(adj) {
    let n = len(adj);
    let indeg = [];
    for i in 0..n { indeg = indeg + [0]; }
    for i in 0..n {
        let adji = adj[i];
        for j in 0..len(adji) {
            let v = adji[j];
            indeg[v] = indeg[v] + 1;
        }
    }
    let q = [];
    for i in 0..n {
        if indeg[i] == 0 { q = q + [i]; }
    }
    let head = 0;
    let out = [];
    while head < len(q) {
        let u = q[head];
        head = head + 1;
        out = out + [u];
        let adju = adj[u];
        for j in 0..len(adju) {
            let v = adju[j];
            indeg[v] = indeg[v] - 1;
            if indeg[v] == 0 { q = q + [v]; }
        }
    }
    if len(out) == n { out } else { [] }
}

// 最小生成树（Kruskal）：edges=[[u,v,w],...] 升序选边；空 edges 返回 0
fn kruskal_mst(edges) {
    let m = len(edges);
    if m == 0 { return 0; }
    let e = edges;
    let nn = 0;
    for i in 0..m {
        let u = e[i][0];
        let v = e[i][1];
        if u > nn { nn = u; }
        if v > nn { nn = v; }
    }
    let n = nn + 1;
    // 按 w 升序插入排序（e 为局部变量，可直接下标赋值）
    let i = 1;
    while i < m {
        let key = e[i];
        let j = i - 1;
        while j >= 0 and e[j][2] > key[2] {
            e[j + 1] = e[j];
            j = j - 1;
        }
        e[j + 1] = key;
        i = i + 1;
    }
    let uf = std.data.UnionFind(n);
    let total = 0.0;
    for k in 0..m {
        let u = e[k][0];
        let v = e[k][1];
        let w = e[k][2];
        if !uf.connected(u, v) {
            uf.union(u, v);
            total = total + w;
        }
    }
    total
}

// 最小生成树（Prim）：w 为 n×n 矩阵，-1 表示无边；返回总权
fn prim_mst(w, start) {
    let n = len(w);
    let INF = 1000000000000000000.0;
    let key = [];
    let mst = [];
    for i in 0..n {
        key = key + [INF];
        mst = mst + [0];
    }
    key[start] = 0.0;
    let total = 0.0;
    let iter = 0;
    while iter < n {
        let u = -1;
        let best = INF;
        for v in 0..n {
            if mst[v] == 0 and key[v] < best {
                best = key[v];
                u = v;
            }
        }
        if u == -1 {
            iter = n;
        } else {
            mst[u] = 1;
            total = total + key[u];
            let wu = w[u];
            for v in 0..n {
                let c = wu[v];
                if c >= 0 and mst[v] == 0 and c < key[v] {
                    key[v] = c;
                }
            }
            iter = iter + 1;
        }
    }
    total
}

// 强连通分量（Kosaraju）：返回分量数组的数组
fn scc(adj) {
    let n = len(adj);
    let visited = [];
    for i in 0..n { visited = visited + [0]; }
    let order = [];
    let dfs = fn(u) {
        visited[u] = 1;
        let adju = adj[u];
        for j in 0..len(adju) {
            let v = adju[j];
            if visited[v] == 0 {
                dfs(v);
            }
        }
        order = order + [u];
    };
    for i in 0..n {
        if visited[i] == 0 { dfs(i); }
    }
    // 转置图 tadj
    let tadj = [];
    for i in 0..n { tadj = tadj + [[]]; }
    for u in 0..n {
        let adju = adj[u];
        for j in 0..len(adju) {
            let v = adju[j];
            tadj[v] = tadj[v] + [u];
        }
    }
    let visited2 = [];
    for i in 0..n { visited2 = visited2 + [0]; }
    let comps = [];
    let dfs2 = fn(u, comp) {
        visited2[u] = 1;
        comp = comp + [u];
        let tu = tadj[u];
        for j in 0..len(tu) {
            let v = tu[j];
            if visited2[v] == 0 {
                comp = dfs2(v, comp);
            }
        }
        comp
    };
    let k = n - 1;
    while k >= 0 {
        let u = order[k];
        if visited2[u] == 0 {
            let comp = dfs2(u, []);
            comps = comps + [comp];
        }
        k = k - 1;
    }
    comps
}
