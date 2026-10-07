// graph_theory.x —— 图论（并入 std.math，import std.math 即可用）
// 节点 0..n-1；邻接表 adj[i]=[邻居...]；邻接矩阵 w[i][j]=权重（-1 无边）
// BFS / DFS / Dijkstra / 连通性判定

// BFS 最短路程：返回距离数组（不可达 -1）
fn bfs(adj, start) {
    let n = len(adj);
    let dist = [];
    for i in 0..n {
        dist = dist + [-1];
    }
    let q = [];
    for i in 0..n {
        q = q + [0];
    }
    let head = 0;
    let tail = 0;
    dist[start] = 0;
    q[tail] = start;
    tail = tail + 1;
    while head < tail {
        let v = q[head];
        head = head + 1;
        for u in adj[v] {
            if dist[u] == -1 {
                dist[u] = dist[v] + 1;
                q[tail] = u;
                tail = tail + 1;
            }
        }
    }
    return dist;
}

// DFS 遍历：返回访问顺序数组（递归闭包）
fn dfs(adj, start) {
    let n = len(adj);
    let visited = [];
    for i in 0..n {
        visited = visited + [0];
    }
    let order = [];
    let visit = fn(v) {
        visited[v] = 1;
        order = order + [v];
        for u in adj[v] {
            if visited[u] == 0 {
                visit(u);
            }
        }
    };
    visit(start);
    return order;
}

// Dijkstra 最短路程：w[i][j]=权重（-1 无边），返回 dist 数组
fn dijkstra(w, start) {
    let n = len(w);
    let INF = 1000000000000000000.0;
    let dist = [];
    for i in 0..n {
        dist = dist + [INF];
    }
    let visited = [];
    for i in 0..n {
        visited = visited + [0];
    }
    dist[start] = 0.0;
    let cnt = 0;
    while cnt < n {
        let u = -1;
        let best = INF;
        for v in 0..n {
            if visited[v] == 0 and dist[v] < best {
                best = dist[v];
                u = v;
            }
        }
        if u >= 0 {
            visited[u] = 1;
            for v in 0..n {
                if w[u][v] >= 0 and visited[v] == 0 and dist[u] + w[u][v] < dist[v] {
                    dist[v] = dist[u] + w[u][v];
                }
            }
        }
        cnt = cnt + 1;
    }
    return dist;
}

// 从 a 到 b 是否有路径：有返回 1，否则 0
fn has_path(adj, a, b) {
    let d = std.math.bfs(adj, a);
    if d[b] >= 0 {
        return 1;
    }
    return 0;
}
