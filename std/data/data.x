// data.x —— 核心容器类（数组承载，字段修改用"读-改-写回"模式）
// 栈：后进先出；pop/peek 空时返回 -1
class Stack {
    let a;
    fn new(self) { self.a = []; }
    fn push(self, v) { self.a = self.a + [v]; }
    fn pop(self) {
        let n = len(self.a);
        if n == 0 { return -1; }
        let v = self.a[n - 1];
        let t = [];
        for i in 0..(n - 1) {
            t = t + [self.a[i]];
        }
        self.a = t;
        v
    }
    fn peek(self) {
        let n = len(self.a);
        if n == 0 { -1 } else { self.a[n - 1] }
    }
    fn is_empty(self) { len(self.a) == 0 }
}

// 队列：先进先出；dequeue/peek 空时返回 -1
class Queue {
    let a;
    fn new(self) { self.a = []; }
    fn enqueue(self, v) { self.a = self.a + [v]; }
    fn dequeue(self) {
        let n = len(self.a);
        if n == 0 { return -1; }
        let v = self.a[0];
        let t = [];
        for i in 1..n {
            t = t + [self.a[i]];
        }
        self.a = t;
        v
    }
    fn peek(self) {
        if len(self.a) == 0 { -1 } else { self.a[0] }
    }
    fn is_empty(self) { len(self.a) == 0 }
}

// 双端队列：两端均可进出；pop 空时返回 -1
class Deque {
    let a;
    fn new(self) { self.a = []; }
    fn push_front(self, v) { self.a = [v] + self.a; }
    fn push_back(self, v) { self.a = self.a + [v]; }
    fn pop_front(self) {
        let n = len(self.a);
        if n == 0 { return -1; }
        let v = self.a[0];
        let t = [];
        for i in 1..n {
            t = t + [self.a[i]];
        }
        self.a = t;
        v
    }
    fn pop_back(self) {
        let n = len(self.a);
        if n == 0 { return -1; }
        let v = self.a[n - 1];
        let t = [];
        for i in 0..(n - 1) {
            t = t + [self.a[i]];
        }
        self.a = t;
        v
    }
    fn len(self) { len(self.a) }
}

// 链表（数组承载）：append/prepend/remove(首个匹配)/find/len
class LinkedList {
    let a;
    fn new(self) { self.a = []; }
    fn append(self, v) { self.a = self.a + [v]; }
    fn prepend(self, v) { self.a = [v] + self.a; }
    fn find(self, v) {
        for i in 0..len(self.a) {
            if self.a[i] == v { return i; }
        }
        -1
    }
    fn remove(self, v) {
        let i = -1;
        for j in 0..len(self.a) {
            if self.a[j] == v and i < 0 { i = j; }
        }
        if i < 0 { return false; }
        let t = [];
        for j in 0..len(self.a) {
            if j != i { t = t + [self.a[j]]; }
        }
        self.a = t;
        true
    }
    fn len(self) { len(self.a) }
}
