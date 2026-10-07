// lint 豁免统一放在两个包的 Cargo.toml [lints.rust]（本文件被根包与 gui 子包共用，
// 内部属性 #![allow] 无法被 include! 引入）
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::marker::PhantomData;
use serde::{Serialize, Deserialize, Serializer, Deserializer};
use std::str::from_utf8;
use std::sync::Arc;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use std::sync::OnceLock;
use std::panic;
use std::io::{self, Write, BufRead, Read};
use std::alloc::{self, Layout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use ariadne::{Color, Label, Report, ReportKind, Source};
use ahash::{HashMap as FastMap, HashMapExt};

// 自定义错误：携带 脚本文件名、行号、错误文本
#[derive(Debug)]
struct ScriptError {
    file: String,
    line: u32,
    col: u32,
    msg: String,
    /// 附加定位（根源/连带位置）：(line, col, msg)，渲染为次要黄色标签
    secondary: Vec<(u32, u32, String)>,
}

#[derive(Debug)]
enum LoopControl {
    None,
    Break,
    Continue,
    Return(Value),
}

// ==============================
// 全局 Arena 内存池 & 内存接口实现
// 提供：mem_global_init / mem_alloc / mem_rollback / mem_free / mem_global_destroy
// ==============================

const MEM_ALIGN: usize = 8;
const ARENA_BLOCK_SIZE: usize = 4096;

#[derive(Debug)]
struct ArenaBlock {
    base: *mut u8,
    cur: *mut u8,
    end: *mut u8,
    // 标记当前块是否已调用dealloc，防止双重释放
    deallocated: AtomicBool,
}

unsafe impl Send for ArenaBlock {}
unsafe impl Sync for ArenaBlock {}

impl ArenaBlock {
    fn new() -> Self {
        let layout = Layout::from_size_align(ARENA_BLOCK_SIZE, MEM_ALIGN).unwrap();
        let base = unsafe { alloc::alloc(layout) };
        if base.is_null() {
            panic!("Arena 块分配失败：系统内存不足");
        }
        let end = unsafe { base.add(ARENA_BLOCK_SIZE) };
        ArenaBlock {
            base,
            cur: base,
            end,
            deallocated: AtomicBool::new(false),
        }
    }

    fn can_alloc(&self, size: usize) -> bool {
        let total = (size + MEM_ALIGN - 1) & !(MEM_ALIGN - 1);
        unsafe { self.cur.add(total) <= self.end }
    }

    fn alloc(&mut self, size: usize) -> *mut u8 {
        let total = (size + MEM_ALIGN - 1) & !(MEM_ALIGN - 1);
        let ptr = self.cur;
        unsafe { self.cur = self.cur.add(total) };
        ptr
    }

    // 安全释放，仅首次调用执行dealloc，重复调用无操作
    fn safe_dealloc(&mut self) {
        if self.deallocated.swap(true, Ordering::Acquire) {
            return;
        }
        let layout = Layout::from_size_align(ARENA_BLOCK_SIZE, MEM_ALIGN).unwrap();
        unsafe { alloc::dealloc(self.base, layout) };
        self.base = std::ptr::null_mut();
        self.cur = std::ptr::null_mut();
        self.end = std::ptr::null_mut();
    }
}

// 全局Arena新增Drop实现，进程退出自动释放所有块，解决内存泄漏
#[derive(Debug, Default)]
struct GlobalArena {
    blocks: Vec<ArenaBlock>,
    rollback_points: Vec<usize>,
    _no_sync: PhantomData<*const ()>,
}

unsafe impl Send for GlobalArena {}
unsafe impl Sync for GlobalArena {}

impl Drop for GlobalArena {
    fn drop(&mut self) {
        self.destroy();
    }
}

impl GlobalArena {
    pub fn new() -> Self {
        Self {
            blocks: Vec::new(),
            rollback_points: Vec::new(),
            _no_sync: PhantomData,
        }
    }
    fn init(&mut self) {
        self.blocks.clear();
        self.rollback_points.clear();
        self.blocks.push(ArenaBlock::new());
    }

    fn alloc(&mut self, size: usize) -> *mut u8 {
        // 禁止分配0字节，直接panic，杜绝空指针流出
        if size == 0 {
            panic!("Arena mem_alloc 不允许分配0字节内存");
        }
        let last = self.blocks.last_mut().unwrap();
        if last.can_alloc(size) {
            return last.alloc(size);
        }
        let mut new_block = ArenaBlock::new();
        let ptr = new_block.alloc(size);
        self.blocks.push(new_block);
        ptr
    }

    fn save_rollback(&mut self) {
        let idx = self.blocks.len() - 1;
        let offset = unsafe { self.blocks[idx].cur.offset_from(self.blocks[idx].base) } as usize;
        self.rollback_points.push(offset);
    }

    fn rollback(&mut self) {
        let offset = match self.rollback_points.pop() {
            Some(o) => o,
            None => return,
        };
        let idx = self.blocks.len() - 1;
        let block = &mut self.blocks[idx];
        unsafe { block.cur = block.base.add(offset) };
        self.blocks.truncate(idx + 1);
    }

    fn destroy(&mut self) {
        for block in &mut self.blocks {
            block.safe_dealloc();
        }
        self.blocks.clear();
        self.rollback_points.clear();
    }
}

// 全局内存池容器，RwLock包裹HashMap，OnceLock延迟初始化
static mut ARENA_MAP: OnceLock<RwLock<HashMap<String, GlobalArena>>> = OnceLock::new();
const DEFAULT_ARENA_NAME: &str = "__global_default__";

/// 获取静态RwLock实例，内部unsafe操作包裹
unsafe fn get_arena_rwlock() -> &'static RwLock<HashMap<String, GlobalArena>> {
    unsafe {
        ARENA_MAP.get_or_init(|| RwLock::new(HashMap::new()))
    }
}

/// 获取读锁，内部包裹unsafe调用
fn map_read() -> RwLockReadGuard<'static, HashMap<String, GlobalArena>> {
    unsafe {
        get_arena_rwlock().read().unwrap()
    }
}

/// 获取写锁，内部包裹unsafe调用
fn map_write() -> RwLockWriteGuard<'static, HashMap<String, GlobalArena>> {
    unsafe {
        get_arena_rwlock().write().unwrap()
    }
}

// ========== 多内存池底层接口 ==========
/// 创建/初始化指定名称内存池
fn mem_create_arena(name: &str) {
    let mut map = map_write();
    let arena = map.entry(name.to_string()).or_insert_with(GlobalArena::new);
    arena.init();
}

/// 从指定内存池分配内存（修复E0596：分配需要&mut，改用写锁）
fn mem_alloc_named(name: &str, size: usize) -> *mut u8 {
    let mut map = map_write();
    let arena = map.get_mut(name).expect(&format!("内存池 {} 不存在", name));
    arena.alloc(size)
}

/// 给指定内存池保存回滚点
fn mem_rollback_named(name: &str) {
    let mut map = map_write();
    let arena = map.get_mut(name).expect(&format!("内存池 {} 不存在", name));
    arena.save_rollback();
}

/// 回滚指定内存池
fn mem_free_named(name: &str) {
    let mut map = map_write();
    let arena = map.get_mut(name).expect(&format!("内存池 {} 不存在", name));
    arena.rollback();
}

/// 销毁指定内存池
fn mem_destroy_named(name: &str) {
    let mut map = map_write();
    if let Some(mut arena) = map.remove(name) {
        arena.destroy();
    }
}

// -------- 兼容原有无参全局接口（默认池） --------
fn mem_global_init() {
    mem_create_arena(DEFAULT_ARENA_NAME);
}

fn mem_alloc(size: usize) -> *mut u8 {
    mem_alloc_named(DEFAULT_ARENA_NAME, size)
}

fn mem_rollback() {
    mem_rollback_named(DEFAULT_ARENA_NAME);
}

fn mem_free() {
    mem_free_named(DEFAULT_ARENA_NAME);
}

fn mem_global_destroy() {
    mem_destroy_named(DEFAULT_ARENA_NAME);
}

// 全局资源兜底释放函数
fn global_arena_cleanup() {
    let mut map = map_write();
    map.clear();
}

// ==============================
// 内存池字符串（完全基于自定义内存池，不使用标准 String 堆分配）
// ==============================
#[derive(Debug)]
struct RawBuf {
    data: *mut u8,
    len: usize,
    released: AtomicBool, // 改为原子布尔，多Arc并发安全
}

impl Drop for RawBuf {
    fn drop(&mut self) {
        // 原子交换标记，true代表已释放，直接返回
        if self.released.swap(true, Ordering::Acquire) {
            return;
        }
        if self.data.is_null() {
            return;
        }
        unsafe {
            // 还原Vec并释放堆内存
            let _ = Vec::from_raw_parts(self.data, self.len, self.len);
        }
        // 置空野指针
        self.data = std::ptr::null_mut();
        self.len = 0;
    }
}

// 对外 Pool —— 上层逻辑完全不变，无需修改任何调用处
#[derive(Debug, Clone)]
pub struct PoolStr {
    buf: Arc<RawBuf>,
}

impl PoolStr {
    pub fn new(s: &str) -> Self {
        let bytes = s.as_bytes();
        let mut buf = bytes.to_vec();
        let data = buf.as_mut_ptr();
        let len = buf.len();
        std::mem::forget(buf);

        Self {
            buf: Arc::new(RawBuf {
                data,
                len,
                released: AtomicBool::new(false), // 原子初始未释放
            })
        }
    }

    pub fn concat(a: &Self, b: &Self) -> Self {
        // 判空防护
        let a_bytes = if a.buf.data.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(a.buf.data, a.buf.len) }
        };
        let b_bytes = if b.buf.data.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(b.buf.data, b.buf.len) }
        };

        let mut buf = Vec::with_capacity(a_bytes.len() + b_bytes.len());
        buf.extend_from_slice(a_bytes);
        buf.extend_from_slice(b_bytes);

        let data = buf.as_mut_ptr();
        let len = buf.len();
        std::mem::forget(buf);

        Self {
            buf: Arc::new(RawBuf {
                data,
                len,
                released: AtomicBool::new(false),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        // 空指针/已释放 兜底返回空字符串
        if self.buf.data.is_null() || self.buf.released.load(Ordering::Relaxed) {
            return "";
        }
        unsafe {
            let slice = std::slice::from_raw_parts(self.buf.data, self.buf.len);
            from_utf8(slice).unwrap_or("")
        }
    }

    pub fn len(&self) -> usize {
        if self.buf.released.load(Ordering::Relaxed) { 0 } else { self.buf.len }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Serialize for PoolStr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_str().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PoolStr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Self::new(&s))
    }
}

// ==============================
// 词法分析 Token
// ==============================
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Let,
    Const,
    Class,
    Static,
    Hash,
    Fn,
    If,
    Else,
    Elif,
    While,
    For,
    From,
    To,
    In,
    Unsafe,
    Return,
    Yield,
    Break,
    Continue,
    Throw,
    Try,
    Catch,
    True,
    False,
    Import,
    Range,
    AndKey,
    OrKey,
    Match,
    Arrow,
    ArrowR, // -> 类型转换 / 函数返回类型标注
    Underscore,
    Ident(String),
    Number(i32),
    Int64(i64), // 超出 i32 范围的整数字面量
    Float(f64),
    String(String),
    RawString(String),
    Colon, // 单冒号：用于 class 继承 class Sub : Parent
    ColonColon,
    Dot,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Equal,
    Gt,
    Lt,
    Ge, // >=
    Le, // <=
    QuestionEqual, // ?=
    Neq, // != 不等于
    Not, // ! 逻辑非
    LParen,
    RParen,
    LBrace,
    RBrace,
    LSquare,
    RSquare,
    Semicolon,
    Comma,
    And,         // 取地址 &
    StarDeref,   // 解引用 * (前缀)
    Eof,
}

// ==============================
// 词法分析器 Tokenizer
// ==============================
struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
    pub last_col: u32, // 最近一次返回 token 的起始列（从1开始）
}

impl Tokenizer {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
            last_col: 1,
        }
    }

    /// 计算 given_pos 所在位置相对行首的列（从1开始）。
    /// 与 line_col_to_char_idx 保持一致：每个字符（含制表符）计 1 列，
    /// \r\n 折行时不在行内累加 \r。
    fn current_col(&self) -> u32 {
        self.col_at(self.pos)
    }

    /// 计算指定字符索引位置相对行首的列（从1开始）
    fn col_at(&self, at: usize) -> u32 {
        let mut line_start = 0usize;
        for (i, &c) in self.chars[..at].iter().enumerate() {
            if c == '\n' {
                line_start = i + 1;
            }
        }
        let mut col = 0u32;
        for &c in &self.chars[line_start..at] {
            if c == '\n' || c == '\r' {
                continue;
            }
            col += 1;
        }
        col + 1
    }

    // 统一处理换行
    fn consume_crlf(&mut self, line: &mut u32) {
        match self.chars[self.pos] {
            '\n' => {
                self.pos += 1;
                // Windows \r\n
                if self.pos < self.chars.len() && self.chars[self.pos] == '\r' {
                    self.pos += 1;
                }
            }
            '\r' => {
                self.pos += 1;
                // 单独 \r 或 \r\n
                if self.pos < self.chars.len() && self.chars[self.pos] == '\n' {
                    self.pos += 1;
                }
            }
            _ => return,
        }
        *line += 1;
    }

    fn next_token(&mut self, file: &str, line: &mut u32) -> Token {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            let c = self.chars[self.pos];
            if c == '\n' || c == '\r' {
                self.consume_crlf(line);
            } else {
                self.pos += 1;
            }
        }
        if self.pos >= self.chars.len() {
            self.last_col = self.current_col();
            return Token::Eof;
        }
        // 记录当前 token 的起始列，供错误定位使用
        self.last_col = self.current_col();

        match self.chars[self.pos] {
            '&' => self.consume(Token::And),
            '*' => {
                let next_ch = self.peek();
                // 向前看一个字符，判断左侧是不是操作数（数字/标识符/右括号/右方括号）
                let prev_char = if self.pos > 0 { self.chars[self.pos - 1] } else { ' ' };
                let left_is_operand = prev_char.is_ascii_digit() 
                    || prev_char.is_alphanumeric() 
                    || prev_char == '_' 
                    || prev_char == ')' 
                    || prev_char == ']';
                // 左侧是操作数 → 一定是中缀乘号，强制返回Star
                if left_is_operand {
                    self.consume(Token::Star)
                } else {
                    // 左侧不是操作数，才判断后面是不是标识符，区分解引用
                    if next_ch.is_alphanumeric() || next_ch == '_' || next_ch == '(' || next_ch == '&' || next_ch == '*' {
                        self.consume(Token::StarDeref)
                    } else {
                        self.consume(Token::Star)
                    }
                }
            }
            '.' => {
                if self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == '.' {
                    self.pos += 2;
                    return Token::Range;
                } else {
                    self.consume(Token::Dot)
                }
            }
            '?' => {
                if self.peek() == '=' {
                    self.pos += 2;
                    return Token::QuestionEqual;
                } else {
                    script_panic_at(file, *line, self.current_col(), "单独 ? 不支持，仅支持 ?= 运算符");
                }
            }
            'r' => {
                if self.peek() == '"' {
                    self.pos += 1;
                    return self.read_raw_string(file, line);
                } else {
                    self.read_identifier()
                }
            }
            '0'..='9' => self.read_number(file, line),
            'a'..='z' | 'A'..='Z' | '_' => self.read_identifier(),
            '"' => self.read_string(file, line),
            '[' => self.consume(Token::LSquare),
            ']' => self.consume(Token::RSquare),
            '!' => {
                if self.peek() == '=' {
                    self.pos += 2;
                    return Token::Neq;
                } else {
                    self.pos += 1;
                    return Token::Not;
                }
            }
            '=' => {
                if self.peek() == '=' {
                    self.pos += 1;
                    self.consume(Token::Equal)
                } else if self.peek() == '>' {
                    self.pos += 1;
                    self.consume(Token::Arrow)
                } else {
                    self.consume(Token::Eq)
                }
            }
            '+' => self.consume(Token::Plus),
            '-' => {
                let save = self.pos;
                let mut p = self.pos + 1;
                while p < self.chars.len() && self.chars[p].is_whitespace() { p += 1; }
                if p < self.chars.len() && self.chars[p] == '>' {
                    self.pos = p + 1;
                    Token::ArrowR
                } else {
                    self.pos = save;
                    self.consume(Token::Minus)
                }
            }
            '/' => {
                if self.peek() == '/' {
                    self.skip_line_comment(line);
                    return self.next_token(file, line);
                } else if self.peek() == '*' {
                    self.skip_block_comment(line);
                    return self.next_token(file, line);
                } else {
                    self.consume(Token::Slash)
                }
            }
            '%' => self.consume(Token::Percent),
            '>' => {
                if self.peek() == '=' {
                    self.pos += 2; // 同时跳过 > 和 = 两个字符
                    return Token::Ge;
                } else {
                    self.consume(Token::Gt)
                }
            }
            '<' => {
                if self.peek() == '=' {
                    self.pos += 2; // 同时跳过 < 和 = 两个字符
                    return Token::Le;
                } else {
                    self.consume(Token::Lt)
                }
            }
            '(' => self.consume(Token::LParen),
            ')' => self.consume(Token::RParen),
            '{' => self.consume(Token::LBrace),
            '}' => self.consume(Token::RBrace),
            '#' => self.consume(Token::Hash),
            ';' => self.consume(Token::Semicolon),
            ',' => self.consume(Token::Comma),
            ':' => {
                if self.peek() == ':' {
                    self.pos += 1;
                    self.consume(Token::ColonColon)
                } else {
                    self.consume(Token::Colon)
                }
            }
            ch => {
                let file_name = if file.is_empty() { "未知文件" } else { file };
                script_panic_at(file_name, *line, self.current_col(), &format!("词法错误：第{}行存在非法字符「{}」，无匹配语法规则", line, ch));
            }
        }
    }

    fn consume(&mut self, token: Token) -> Token {
        self.pos += 1;
        token
    }

    fn peek(&self) -> char {
        if self.pos + 1 < self.chars.len() {
            self.chars[self.pos + 1]
        } else {
            '\0'
        }
    }

    fn read_number(&mut self, file: &str, line: &mut u32) -> Token {
        let start = self.pos;
        let mut has_dot = false;

        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == '.' && self.pos + 1 < self.chars.len() && self.chars[self.pos + 1] == '.' {
                break;
            }
            if c.is_ascii_digit() {
                self.pos += 1;
            } else if c == '.' && !has_dot {
                has_dot = true;
                self.pos += 1;
            } else {
                break;
            }
        }

        let num_str: String = self.chars[start..self.pos].iter().collect();
        let start_col = self.col_at(start);
        if has_dot {
            let dot_idx = num_str.find('.').unwrap();
            if dot_idx == 0 || dot_idx == num_str.len() - 1 {
                script_panic_at(file, *line, start_col, &format!("非法浮点数 {}，请使用 0.5 / 123.45 格式", num_str));
            }
            match num_str.parse::<f64>() {
                Ok(f) => Token::Float(f),
                Err(_) => script_panic_at(file, *line, start_col, &format!("数字解析失败: {}", num_str)),
            }
        } else {
            match num_str.parse::<i32>() {
                Ok(n) => Token::Number(n),
                Err(_) => match num_str.parse::<i64>() {
                    Ok(n) => Token::Int64(n),
                    Err(_) => script_panic_at(file, *line, start_col, &format!("整数解析失败: {}", num_str)),
                },
            }
        }
    }

    fn read_string(&mut self, file: &str, line: &mut u32) -> Token {
        let start_line = *line; // 捕获字符串起始行，不再用内部换行后的line
        let start = self.pos;   // 起始引号位置
        let start_col = self.col_at(start);
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.chars.len() && self.chars[self.pos] != '"' {
            let c = self.chars[self.pos];
            if c == '\n' {
                self.consume_crlf(line);
                continue;
            }
            if c == '\\' {
                self.pos += 1;
                if self.pos >= self.chars.len() {
                    script_panic_at(file, *line, start_col, "字符串未闭合，转义符后缺少字符");
                }
                let esc = self.chars[self.pos];
                match esc {
                    'n' => s.push('\n'),
                    'r' => s.push('\r'),
                    't' => s.push('\t'),
                    '\\' => s.push('\\'),
                    '"' => s.push('"'),
                    '$' => s.push('$'),
                    other => script_panic_at(file, *line, start_col, &format!("不支持的转义字符 \\{}", other)),
                }
                self.pos += 1;
                continue;
            }
            s.push(c);
            self.pos += 1;
        }
        self.pos += 1;
        // 存入起始行，不是内部换行后的行
        Token::String(format!("{}||LINE||{}", s, start_line))
    }

    fn read_raw_string(&mut self, file: &str, line: &mut u32) -> Token {
        let start_line = *line; // 捕获原始字符串起始行
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.chars.len() && self.chars[self.pos] != '"' {
            if self.chars[self.pos] == '\n' {
                self.consume_crlf(line);
                continue;
            }
            s.push(self.chars[self.pos]);
            self.pos += 1;
        }
        self.pos += 1;
        // 存入起始行
        Token::RawString(format!("{}||LINE||{}", s, start_line))
    }

    fn read_identifier(&mut self) -> Token {
        let mut ident = String::new();
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
        {
            ident.push(self.chars[self.pos]);
            self.pos += 1;
        }

        if ident == "_" { return Token::Underscore; }
        match ident.as_str() {
            "let" => Token::Let,
            "fn" => Token::Fn,
            "if" => Token::If,
            "else" => Token::Else,
            "elif" => Token::Elif,
            "while" => Token::While,
            "for" => Token::For,
            "in" => Token::In,
            "from" => Token::From,
            "to" => Token::To,
            "unsafe" => Token::Unsafe,
            "yield" => Token::Yield,
            "return" => Token::Return,
            "break" => Token::Break,
            "continue" => Token::Continue,
            "throw" => Token::Throw,
            "try" => Token::Try,
            "catch" => Token::Catch,
            "true" => Token::True,
            "false" => Token::False,
            "import" => Token::Import,
            "const" => Token::Const,
            "class" => Token::Class,
            "static" => Token::Static,
            "and" => Token::AndKey, 
            "or" => Token::OrKey,
            "match" => Token::Match,
            _ => Token::Ident(ident),
        }
    }

    // 行注释：内部换行计数
    fn skip_line_comment(&mut self, line: &mut u32) {
        self.pos += 2;
        while self.pos < self.chars.len() {
            if self.chars[self.pos] == '\n' || self.chars[self.pos] == '\r' {
                self.consume_crlf(line);
                break;
            }
            self.pos += 1;
        }
    }

    // 块注释：多行换行全部计数
    fn skip_block_comment(&mut self, line: &mut u32) {
        self.pos += 2; // 跳过 /*
        let len = self.chars.len();

        while self.pos + 1 < len {
            // 匹配 */ 结束注释
            if self.chars[self.pos] == '*' && self.chars[self.pos + 1] == '/' {
                self.pos += 2;
                return;
            }

            // 统一处理 \n / \r\n / \r，自动推进pos、行号+1
            if self.chars[self.pos] == '\n' || self.chars[self.pos] == '\r' {
                self.consume_crlf(line);
                continue;
            }
            self.pos += 1;
        }

        // 读到文件尾无闭合注释
        script_panic("", *line, "未闭合的块注释 /*");
    }
}

// ==============================
// 抽象语法树 Expr / Op / Stmt
// ==============================
#[derive(Debug, Clone)]
struct Func {
    name: String,
    line: u32,
    params: Vec<String>,
    body: Vec<Stmt>,
    /// 返回类型标注（fn f() -> i64），None 表示未标注
    ret_ty: Option<String>,
}

#[derive(Debug, Clone)]
struct ClassDef {
    name: String,
    /// 父类名（单继承）
    superclass: Option<String>,
    fields: Vec<u32>,
    private_fields: Vec<u32>,
    constructor: Option<Func>,
    methods: HashMap<u32, Func>,
    statics: HashMap<u32, Func>,
}

#[derive(Debug, Clone)]
enum Expr {
    Number(i32, u32),
    Int64(i64, u32),
    Float(f64, u32),
    String(String, u32),
    RawString(String, u32),
    Bool(bool, u32),
    Ident(u32, u32),
    Array(Vec<Expr>, u32),
    Index(Box<Expr>, Box<Expr>, u32),
    BinOp(Box<Expr>, Op, Box<Expr>, u32),
    Call(String, Vec<Expr>, u32),
    Assign(u32, Box<Expr>, u32),
    IndexAssign(Box<Expr>, Box<Expr>, Box<Expr>, u32),
    AddrOf(Box<Expr>, u32),
    RawAddr(Box<Expr>, u32),
    Deref(Box<Expr>, u32),
    DerefAssign(Box<Expr>, Box<Expr>, u32),
    Neg(Box<Expr>, u32),
    Not(Box<Expr>, u32),
    Member(Box<Expr>, u32, u32), // expr . ident
    MethodCall(Box<Expr>, u32, Vec<Expr>, u32), // obj.method(args)
    MemberAssign(Box<Expr>, u32, Box<Expr>, u32),
    // 匿名函数字面量（闭包）：fn(params) { body }
    Lambda(Func, u32),
    // 字典字面量：{ "k": expr, ... }
    Dict(Vec<(String, Expr)>, u32),
    // 私有字段访问：obj.#name
    PrivateMember(Box<Expr>, u32, u32),
    // 私有字段赋值：obj.#name = value
    PrivateMemberAssign(Box<Expr>, u32, Box<Expr>, u32), // obj.field = value
    // match 表达式：subject + 模式分支
    Match(Box<Expr>, Vec<(MatchPat, Expr)>, u32),
    // 区间表达式：lo..hi（数值区间值）
    Range(Box<Expr>, Box<Expr>, u32),
    // 类型转换：expr -> i32/i64/float/str/bool
    Cast(Box<Expr>, String, u32),
}

#[derive(Debug, Clone)]
enum MatchPat {
    Lit(Value),
    Wildcard,
    Range(f64, f64),
}

#[derive(Debug, Clone)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Gt,
    Lt,
    Le,
    Ge,
    Equal,
    And, 
    Or,
    RelCmp, // ?= 关系比较
    Neq, // !=
}

// 表达式静态类型
#[derive(Debug, Clone, PartialEq)]
enum Type {
    Int,
    Float,
    Bool,
    String,
    Array(Box<Type>),
    ArenaPtr, // 指针类型
    RawPtr,
    ArrayElemPtr,
}

// 类型环境：记录每个变量的静态类型
#[derive(Default, Clone)]
struct TypeEnv {
    vars: FastMap<String, Type>,
    outer: Option<Box<TypeEnv>>,
}

impl TypeEnv {
    fn new() -> Self {
        Self {
            vars: FastMap::new(),
            outer: None,
        }
    }

    // 嵌套作用域
    fn nested(outer: TypeEnv) -> Self {
        Self {
            vars: FastMap::new(),
            outer: Some(Box::new(outer)),
        }
    }

    // 定义变量类型
    fn define(&mut self, name: String, ty: Type) {
        self.vars.insert(name, ty);
    }

    // ID 版本（反查符号表）
    fn define_id(&mut self, id: u32, ty: Type) { self.define(interner().lookup(id), ty); }
    fn get_id(&self, id: u32) -> Option<Type> { self.get(&interner().lookup(id)) }

    // 查询变量类型
    fn get(&self, name: &str) -> Option<Type> {
        if let Some(ty) = self.vars.get(name) {
            return Some(ty.clone());
        }
        if let Some(outer) = &self.outer {
            outer.get(name)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
enum Stmt {
    Let(u32, Expr, u32),
    Const(u32, Expr, u32),
    FnDef(String, Vec<String>, Vec<Stmt>, Option<String>, u32),
    If(Expr, Vec<Stmt>, Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>, u32),
    While(Expr, Vec<Stmt>, u32),
    For(u32, Expr, Expr, Vec<Stmt>, u32),
    Print(Vec<Expr>, u32),
    Expr(Expr, u32),
    Return(Option<Expr>, u32),
    Yield(Expr, u32),
    Break(u32),
    Continue(u32),
    /// throw 表达式：抛出异常值
    Throw(Box<Expr>, u32),
    /// try { } catch (e) { }
    Try { body: Vec<Stmt>, catch_var: Option<String>, handler: Vec<Stmt>, line: u32 },
    ImportItem {
        lib_path: String,
        parts: Vec<String>,
        alias: Option<String>,
        import_all: bool,
        names: Option<Vec<String>>,
        line: u32,
    },
    UnsafeBlock(Vec<Stmt>, u32),
    Class(ClassDef, u32),
}

// ==============================
// 语法解析器 Parser
// ==============================
struct Parser {
    tokenizer: Tokenizer,
    current: Token,
    const_names: HashSet<u32>,
    pub line: u32,       // 新增：当前脚本行号
    pub file: String,    // 新增：当前脚本文件名
    // 新增：全局类型检查环境，贯穿整个文件解析
    type_env: TypeEnv,
    in_unsafe_block: bool,
    /// 已知模块命名空间（如 "std"、"std::math"、"math"），点链命中时按命名空间访问
    known_modules: HashSet<String>,
}

impl Parser {
    fn new(mut tokenizer: Tokenizer) -> Self {
        let mut line = 1;
        // 初始化读取首个 token，同步行号
        let current = tokenizer.next_token("", &mut line);
        let mut p = Self {
            tokenizer,
            current,
            const_names: HashSet::new(),
            line,
            file: String::new(),
            type_env: TypeEnv::new(), // 初始化全局类型环境
            in_unsafe_block: false,
            known_modules: HashSet::new(),
        };
        p.known_modules.insert("std".to_string());
        p.known_modules.insert("lib".to_string());
        p
    }

    fn parse_program(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        while self.current != Token::Eof {
            stmts.push(self.parse_stmt());
        }
        stmts
    }

    fn parse_stmt(&mut self) -> Stmt {
        let current_line = self.line;
        let stmt = match &self.current {
            Token::Break => self.parse_break(),
            Token::Continue => self.parse_continue(),
            Token::Import => self.parse_import(),
            Token::Let => self.parse_let(),
            Token::Const => self.parse_const(),
            Token::Fn => self.parse_fn_def(),
            Token::Class => self.parse_class(),
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::For => self.parse_for(),
            Token::Unsafe => self.parse_unsafe_block(),
            Token::Return => self.parse_return(),
            Token::Yield => self.parse_yield(),
            Token::Try => self.parse_try(),
            Token::Throw => self.parse_throw(),
            Token::Ident(name) if name == "print" => self.parse_print(),
            _ => self.parse_expr_stmt(),
        };

        // 使用 Parser 全局唯一的类型环境，变量作用域正常传递
        // check_stmt(&stmt, &mut self.type_env, &self.file, current_line);
        stmt
    }

    fn parse_for(&mut self) -> Stmt {
        self.consume(Token::For);
        let var_name = match &self.current {
            Token::Ident(s) => s.clone(),
            _ => self.panic_here( "for 关键字后必须跟变量名")
        };
        self.consume(Token::Ident(var_name.clone()));

        let mut start_expr: Expr;
        let mut end_expr: Expr;

        // 直接匹配专属关键字 Token，不再判断 Ident
        match self.current {
            Token::In => {
                self.consume(Token::In);
                start_expr = self.parse_expr();
                if self.current == Token::Range {
                    self.consume(Token::Range);
                    end_expr = self.parse_expr();
                } else if let Expr::Range(l, r, _ln) = start_expr {
                    start_expr = *l;
                    end_expr = *r;
                } else {
                    // 单值源：for x in gen() / for x in arr，end 用哨兵 0，求值按 start 类型分派
                    end_expr = Expr::Number(0, self.line);
                }
            }
            Token::From => {
                self.consume(Token::From);
                start_expr = self.parse_expr();
                if self.current != Token::To {
                    self.panic_here( "from 语法缺少 to 关键字");
                }
                self.consume(Token::To);
                end_expr = self.parse_expr();
            }
            _ => {
                self.panic_here( "for 语法错误！支持两种格式：\n1. for 变量 in 起始..结束 {}\n2. for 变量 from 起始 to 结束 {}");
            }
        }

        if self.current != Token::LBrace {
            self.panic_here( "循环体必须用大括号包裹");
        }
        self.consume(Token::LBrace);
        let mut body = Vec::new();
        while self.current != Token::RBrace && self.current != Token::Eof {
            body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);
        Stmt::For(interner().get(&var_name), start_expr, end_expr, body, self.line)
    }

    /// 解析 unsafe 代码块：所有裸指针操作必须在此内部
    fn parse_unsafe_block(&mut self) -> Stmt {
        self.consume(Token::Unsafe);
        if self.current != Token::LBrace {
            self.panic_here( "unsafe 后必须跟大括号 {}");
        }
        self.consume(Token::LBrace);

        let old_unsafe_state = self.in_unsafe_block;
        self.in_unsafe_block = true; // 进入unsafe块

        let mut body = Vec::new();
        while self.current != Token::RBrace && self.current != Token::Eof {
            body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);

        self.in_unsafe_block = old_unsafe_state; // 退出，恢复旧状态
        Stmt::UnsafeBlock(body, self.line)
    }

    // 解析 return; / return expr;
    fn parse_try(&mut self) -> Stmt {
        let start_line = self.line;
        self.consume(Token::Try);
        if self.current != Token::LBrace { self.panic_here("try 后必须跟 { } 块"); }
        self.consume(Token::LBrace);
        let mut body = Vec::new();
        while self.current != Token::RBrace && self.current != Token::Eof {
            body.push(self.parse_stmt());
        }
        if self.current == Token::Eof { self.panic_here("try 块缺少右大括号 }"); }
        self.consume(Token::RBrace);
        // catch 必须跟随 try
        if self.current != Token::Catch {
            self.panic_here("try 块后必须有 catch 块");
        }
        self.consume(Token::Catch);
        // 可选 catch (e)
        let mut catch_var: Option<String> = None;
        if self.current == Token::LParen {
            self.consume(Token::LParen);
            let vname = match &self.current {
                Token::Ident(s) => s.clone(),
                _ => self.panic_here("catch ( 后必须是异常变量名"),
            };
            self.consume(Token::Ident(vname.clone()));
            if self.current != Token::RParen { self.panic_here("catch 变量缺少右括号 )"); }
            self.consume(Token::RParen);
            catch_var = Some(vname);
        }
        if self.current != Token::LBrace { self.panic_here("catch 后必须跟 { } 块"); }
        self.consume(Token::LBrace);
        let mut handler = Vec::new();
        while self.current != Token::RBrace && self.current != Token::Eof {
            handler.push(self.parse_stmt());
        }
        if self.current == Token::Eof { self.panic_here("catch 块缺少右大括号 }"); }
        self.consume(Token::RBrace);
        Stmt::Try { body, catch_var, handler, line: start_line }
    }

    fn parse_throw(&mut self) -> Stmt {
        let ln = self.line;
        self.consume(Token::Throw);
        let expr = self.parse_expr();
        if self.current != Token::Semicolon {
            self.panic_here("throw 语句必须以分号结尾");
        }
        self.consume(Token::Semicolon);
        Stmt::Throw(Box::new(expr), ln)
    }

    fn parse_return(&mut self) -> Stmt {
        self.consume(Token::Return);
        // 判断后面是否有表达式（分号/}直接无返回值）
        let has_expr = !matches!(self.current, Token::Semicolon | Token::RBrace | Token::Eof);
        let ret_expr = if has_expr {
            Some(self.parse_expr())
        } else {
            None
        };
        self.consume(Token::Semicolon);
        Stmt::Return(ret_expr, self.line)
    }

    fn parse_yield(&mut self) -> Stmt {
        self.consume(Token::Yield);
        let ln = self.line; // yield 关键字所在行（后续 consume 分号会推进行号）
        let expr = self.parse_expr();
        self.consume(Token::Semicolon);
        Stmt::Yield(expr, ln)
    }

    fn parse_break(&mut self) -> Stmt {
        self.consume(Token::Break);
        self.consume(Token::Semicolon);
        Stmt::Break(self.line)
    }

    fn parse_continue(&mut self) -> Stmt {
        self.consume(Token::Continue);
        self.consume(Token::Semicolon);
        Stmt::Continue(self.line)
    }

    fn parse_const(&mut self) -> Stmt {
        self.consume(Token::Const);
        let name = match &self.current {
            Token::Ident(n) => n.clone(),
            _ => self.panic_here( "语法错误：期望常量名")
        };
        self.const_names.insert(interner().get(&name));
        self.consume(Token::Ident(name.clone()));
        self.consume(Token::Eq);
        let expr = self.parse_expr();
        self.consume(Token::Semicolon);
        Stmt::Const(interner().get(&name), expr, self.line)
    }

    fn parse_import(&mut self) -> Stmt {
        self.consume(Token::Import);
        let mut path_str = String::new();
        // 新语法：import std.math.x; / import std.math.*; / import std.*;（. 或 :: 分隔，可含 * 通配）
        // 兼容旧语法：import "math" / import "std::math"（字符串，仍支持）
        if let Token::String(s) = &self.current {
            path_str = s.split("||LINE||").next().unwrap_or("").to_string();
            self.consume(Token::String(s.clone()));
        } else {
            loop {
                match &self.current {
                    Token::Ident(n) => { path_str.push_str(n); self.consume(Token::Ident(n.clone())); }
                    Token::Star => { path_str.push('*'); self.consume(Token::Star); }
                    _ => break,
                }
                if matches!(self.current, Token::Dot) {
                    path_str.push('.'); self.consume(Token::Dot);
                } else if matches!(self.current, Token::ColonColon) {
                    path_str.push_str("::"); self.consume(Token::ColonColon);
                } else {
                    break;
                }
            }
            if path_str.is_empty() {
                self.panic_here("import 后必须跟模块路径（如 std.math / std.math.*）");
            }
        }

        let mut alias: Option<String> = None;
        if let Token::Ident(word) = &self.current {
            if word == "as" {
                self.consume(Token::Ident("as".to_string()));
                let a = match &self.current {
                    Token::Ident(s) => s.clone(),
                    _ => self.panic_here( "as 后必须是别名标识符")
                };
                self.consume(Token::Ident(a.clone()));
                alias = Some(a);
            }
        }
        // 命名导入：import mod.{func1, constA};（花括号符号列表，不支持 as）
        let mut names: Option<Vec<String>> = None;
        if self.current == Token::LBrace {
            if alias.is_some() { self.panic_here("命名导入（{ }）不支持与 as 同时使用"); }
            self.consume(Token::LBrace);
            let mut list = Vec::new();
            loop {
                let n = match &self.current {
                    Token::Ident(s) => s.clone(),
                    _ => self.panic_here("import { } 内必须是标识符"),
                };
                self.consume(Token::Ident(n.clone()));
                list.push(n);
                match self.current {
                    Token::Comma => { self.consume(Token::Comma); }
                    Token::RBrace => { self.consume(Token::RBrace); break; }
                    _ => self.panic_here("import { } 内需用逗号分隔，并以 } 结束"),
                }
            }
            if list.is_empty() { self.panic_here("import { } 内不能为空"); }
            if let Token::Ident(w) = &self.current { if w == "as" { self.panic_here("命名导入（{ }）不支持 as"); } }
            names = Some(list);
        }
        self.consume(Token::Semicolon);
        // 归一化：:: → .，再按 . 拆分（过滤空），保留 * 通配标记
        let normalized = path_str.replace("::", ".").replace("||LINE||", "");
        let parts: Vec<String> = normalized.split('.')
            .filter(|seg| !seg.is_empty())
            .map(|seg| seg.to_string())
            .collect();
        let import_all = parts.iter().any(|seg| seg == "*");
        // 记住模块路径各级前缀，使点链命名空间访问（std.math.PI）可解析
        let mut acc = Vec::new();
        for seg in &parts {
            if seg == "*" { break; }
            acc.push(seg.clone());
            self.known_modules.insert(acc.join("::"));
        }
        // std 前缀模块同时记住无前缀别名（std::math -> math），使 math.PI 也可用
        if parts.first().map(|x| x.as_str()) == Some("std") {
            let mut acc2 = Vec::new();
            for seg in &parts[1..] {
                if seg == "*" { break; }
                acc2.push(seg.clone());
                self.known_modules.insert(acc2.join("::"));
            }
        }
        Stmt::ImportItem {
            lib_path: path_str,
            parts,
            alias,
            import_all,
            names,
            line: self.line,
        }
    }

    fn parse_let(&mut self) -> Stmt {
        self.consume(Token::Let);
        let name = match &self.current {
            Token::Ident(n) => n.clone(),
            _ => self.panic_here( "语法错误：期望参数名")
        };
        self.consume(Token::Ident(name.clone()));
        // 类型标注：let a: i32 = expr
        let mut ty: Option<String> = None;
        if matches!(self.current, Token::Colon) {
            self.consume(Token::Colon);
            ty = Some(match &self.current {
                Token::Ident(t) => t.clone(),
                _ => self.panic_here( "语法错误：类型标注需要类型名"),
            });
            self.consume(Token::Ident(ty.clone().unwrap()));
        }
        self.consume(Token::Eq);
        let expr = self.parse_expr();
        self.consume(Token::Semicolon);
        let expr = match ty {
            Some(t) => Expr::Cast(Box::new(expr), t, self.line),
            None => expr,
        };
        Stmt::Let(interner().get(&name), expr, self.line)
    }

    fn parse_fn_def(&mut self) -> Stmt {
        self.consume(Token::Fn);
        // fn 后紧跟 ( 说明是匿名函数表达式被当作语句（如函数体内隐式返回 fn(x){...}）
        if matches!(self.current, Token::LParen) {
            let line = self.line;
            let lambda = self.parse_lambda_body();
            if matches!(self.current, Token::Semicolon) {
                self.consume(Token::Semicolon);
            } else if !matches!(self.current, Token::RBrace | Token::Eof) {
                script_panic_at(&self.file, line, 1, "表达式语句必须以分号结尾");
            }
            return Stmt::Expr(lambda, line);
        }
        let fn_start_line = self.line; // fn 关键字所在行（后续 consume 会推进到函数结束行）
        let name = match &self.current {
            Token::Ident(n) => n.clone(),
            _ => self.panic_here( "语法错误：期望函数名")
        };
        self.consume(Token::Ident(name.clone()));
        self.type_env.define(name.clone(), Type::Int);
        self.consume(Token::LParen);

        let mut params = Vec::new();
        // 保存外层全局类型环境
        let outer_env = self.type_env.clone();
        // 创建函数局部嵌套环境
        self.type_env = TypeEnv::nested(outer_env.clone());

        if !matches!(self.current, Token::RParen) {
            loop {
                let p = match &self.current {
                    Token::Ident(s) => s.clone(),
                    _ => self.panic_here( "语法错误：期望参数名")
                };
                params.push(p.clone());
                // 局部环境注册形参n
                self.type_env.define(p, Type::Int);
                self.consume(Token::Ident(params.last().unwrap().clone()));
                if matches!(self.current, Token::Comma) {
                    self.consume(Token::Comma);
                } else {
                    break;
                }
            }
        }
        self.consume(Token::RParen);
        // 返回类型标注：fn f() -> i64
        let ret_ty: Option<String> = if matches!(self.current, Token::ArrowR) {
            self.consume(Token::ArrowR);
            let t = match &self.current {
                Token::Ident(s) => s.clone(),
                _ => self.panic_here( "语法错误：-> 后需要返回类型"),
            };
            self.consume(Token::Ident(t.clone()));
            Some(t)
        } else { None };
        self.consume(Token::LBrace);

        let mut body = Vec::new();
        while !matches!(self.current, Token::RBrace) {
            body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);

        // 函数解析完毕，恢复外层全局类型环境
        self.type_env = outer_env;
        Stmt::FnDef(name, params, body, ret_ty, fn_start_line)
    }

    /// 匿名函数字面量（闭包）：fn(params) { body }
    fn parse_lambda(&mut self) -> Expr {
        self.consume(Token::Fn);
        self.parse_lambda_body()
    }

    /// 解析匿名函数体（Fn 已消费，从 ( 开始）
    fn parse_lambda_body(&mut self) -> Expr {
        let line = self.line;
        self.consume(Token::LParen);
        let mut params = Vec::new();
        let outer_env = self.type_env.clone();
        self.type_env = TypeEnv::nested(outer_env.clone());
        if !matches!(self.current, Token::RParen) {
            loop {
                let p = match &self.current {
                    Token::Ident(s) => s.clone(),
                    _ => self.panic_here("语法错误：期望参数名"),
                };
                params.push(p.clone());
                self.type_env.define(p, Type::Int);
                self.consume(Token::Ident(params.last().unwrap().clone()));
                if matches!(self.current, Token::Comma) {
                    self.consume(Token::Comma);
                } else {
                    break;
                }
            }
        }
        self.consume(Token::RParen);
        self.consume(Token::LBrace);
        let mut body = Vec::new();
        while !matches!(self.current, Token::RBrace) && !matches!(self.current, Token::Eof) {
            body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);
        self.type_env = outer_env;
        Expr::Lambda(Func { name: "<lambda>".into(), line, params, body, ret_ty: None }, line)
    }

    /// 字典字面量：{ "k": expr, "k2": expr, ... }，键为字符串或标识符
    fn parse_dict(&mut self) -> Expr {
        self.consume(Token::LBrace);
        let line = self.line;
        let mut items = Vec::new();
        if matches!(self.current, Token::RBrace) {
            self.consume(Token::RBrace);
            return Expr::Dict(items, line);
        }
        loop {
            let key = match &self.current {
                Token::String(full) => {
                    let parts: Vec<&str> = full.split("||LINE||").collect();
                    let k = parts[0].to_string();
                    self.consume(Token::String(full.clone()));
                    k
                }
                Token::Ident(id) => {
                    let k = id.clone();
                    self.consume(Token::Ident(k.clone()));
                    k
                }
                _ => self.panic_here("字典键必须是字符串或标识符"),
            };
            self.consume(Token::Colon);
            let val = self.parse_expr();
            items.push((key, val));
            if matches!(self.current, Token::Comma) {
                self.consume(Token::Comma);
                if matches!(self.current, Token::RBrace) { break; }
            } else {
                break;
            }
        }
        self.consume(Token::RBrace);
        Expr::Dict(items, line)
    }

    /// 解析 class 类定义
    fn parse_class(&mut self) -> Stmt {
        self.consume(Token::Class);
        let name = match &self.current {
            Token::Ident(n) => n.clone(),
            _ => self.panic_here("语法错误：期望类名"),
        };
        self.consume(Token::Ident(name.clone()));
        // 可选单继承：class Student : Person {
        let mut superclass: Option<String> = None;
        if matches!(self.current, Token::Colon) {
            self.consume(Token::Colon);
            superclass = match &self.current {
                Token::Ident(p) => Some(p.clone()),
                _ => self.panic_here("继承语法错误：冒号后必须是父类名"),
            };
            self.consume(Token::Ident(superclass.clone().unwrap()));
        }
        if self.current != Token::LBrace {
            self.panic_here("类体必须用大括号包裹");
        }
        self.consume(Token::LBrace);
        let mut fields = Vec::new();
        let mut private_fields = Vec::new();
        let mut constructor: Option<Func> = None;
        let mut methods: HashMap<u32, Func> = HashMap::new();
        let mut statics: HashMap<u32, Func> = HashMap::new();
        while self.current != Token::RBrace && self.current != Token::Eof {
            match &self.current {
                Token::Let => {
                    // 字段声明：let name;（公开）或 let #name;（私有，推荐）
                    self.consume(Token::Let);
                    let is_private = matches!(self.current, Token::Hash);
                    if is_private {
                        self.consume(Token::Hash);
                    }
                    let fname = match &self.current {
                        Token::Ident(s) => s.clone(),
                        _ => self.panic_here("语法错误：期望字段名"),
                    };
                    self.consume(Token::Ident(fname.clone()));
                    // 可选字段类型标注：let name: str;
                    if matches!(self.current, Token::Colon) {
                        self.consume(Token::Colon);
                        let _t = match &self.current {
                            Token::Ident(s) => s.clone(),
                            _ => self.panic_here("语法错误：字段类型标注需要类型名"),
                        };
                        self.consume(Token::Ident(_t.clone()));
                    }
                    self.consume(Token::Semicolon);
                    if is_private {
                        private_fields.push(interner().get(&fname));
                    } else {
                        fields.push(interner().get(&fname));
                    }
                }
                Token::Hash => {
                    // 旧私有字段语法兼容：#let password;
                    self.consume(Token::Hash);
                    if !matches!(self.current, Token::Let) {
                        self.panic_here("# 后必须跟 let 声明私有字段");
                    }
                    self.consume(Token::Let);
                    let fname = match &self.current {
                        Token::Ident(s) => s.clone(),
                        _ => self.panic_here("语法错误：期望字段名"),
                    };
                    self.consume(Token::Ident(fname.clone()));
                    // 可选字段类型标注：#let name: str;
                    if matches!(self.current, Token::Colon) {
                        self.consume(Token::Colon);
                        let _t = match &self.current {
                            Token::Ident(s) => s.clone(),
                            _ => self.panic_here("语法错误：字段类型标注需要类型名"),
                        };
                        self.consume(Token::Ident(_t.clone()));
                    }
                    self.consume(Token::Semicolon);
                    private_fields.push(interner().get(&fname));
                }
                Token::Static => {
                    // 静态方法：static fn hello() {...}
                    self.consume(Token::Static);
                    if !matches!(self.current, Token::Fn) {
                        self.panic_here("static 后必须跟 fn 方法定义");
                    }
                    let (mname, params, body, ret_ty) = self.parse_class_method();
                    statics.insert(interner().get(&mname), Func { name: mname.clone(), line: self.line, params, body, ret_ty });
                }
                Token::Fn => {
                    let (mname, params, body, ret_ty) = self.parse_class_method();
                    if mname == "new" || mname == "init" {
                        constructor = Some(Func { name: "new".into(), line: self.line, params, body, ret_ty });
                    } else {
                        methods.insert(interner().get(&mname), Func { name: mname.clone(), line: self.line, params, body, ret_ty });
                    }
                }
                _ => self.panic_here("类体内仅支持字段声明 let x; / fn 方法 / static fn 静态方法"),
            }
        }
        self.consume(Token::RBrace);
        Stmt::Class(ClassDef { name, superclass, fields, private_fields, constructor, methods, statics }, self.line)
    }

    /// 解析类内方法定义：fn 名(参数...) { 体 }，返回 (方法名, 参数, 函数体)
    fn parse_class_method(&mut self) -> (String, Vec<String>, Vec<Stmt>, Option<String>) {
        self.consume(Token::Fn);
        let name = match &self.current {
            Token::Ident(n) => n.clone(),
            _ => self.panic_here("语法错误：期望方法名"),
        };
        self.consume(Token::Ident(name.clone()));
        self.consume(Token::LParen);
        let mut params = Vec::new();
        if !matches!(self.current, Token::RParen) {
            loop {
                let p = match &self.current {
                    Token::Ident(s) => s.clone(),
                    _ => self.panic_here("语法错误：期望参数名"),
                };
                params.push(p.clone());
                self.consume(Token::Ident(p));
                if matches!(self.current, Token::Comma) {
                    self.consume(Token::Comma);
                } else {
                    break;
                }
            }
        }
        self.consume(Token::RParen);
        // 返回类型标注：fn m() -> type
        let ret_ty: Option<String> = if matches!(self.current, Token::ArrowR) {
            self.consume(Token::ArrowR);
            let t = match &self.current {
                Token::Ident(s) => s.clone(),
                _ => self.panic_here("语法错误：-> 后需要返回类型"),
            };
            self.consume(Token::Ident(t.clone()));
            Some(t)
        } else { None };
        self.consume(Token::LBrace);
        let mut body = Vec::new();
        while !matches!(self.current, Token::RBrace) {
            body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);
        (name, params, body, ret_ty)
    }

    fn parse_if(&mut self) -> Stmt {
        self.consume(Token::If);
        let cond = self.parse_expr();
        self.consume(Token::LBrace);

        let mut then_body = Vec::new();
        while !matches!(self.current, Token::RBrace) {
            then_body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);

        let mut elif_branches = Vec::new();
        while matches!(self.current, Token::Elif) {
            self.consume(Token::Elif);
            let elif_cond = self.parse_expr();
            self.consume(Token::LBrace);
            let mut elif_body = Vec::new();
            while !matches!(self.current, Token::RBrace) {
                elif_body.push(self.parse_stmt());
            }
            self.consume(Token::RBrace);
            elif_branches.push((elif_cond, elif_body));
        }

        let mut else_body = Vec::new();
        if matches!(self.current, Token::Else) {
            self.consume(Token::Else);
            self.consume(Token::LBrace);
            while !matches!(self.current, Token::RBrace) {
                else_body.push(self.parse_stmt());
            }
            self.consume(Token::RBrace);
        }
        Stmt::If(cond, then_body, elif_branches, else_body, self.line)
    }

    fn parse_while(&mut self) -> Stmt {
        self.consume(Token::While);
        let cond = self.parse_expr();
        self.consume(Token::LBrace);
        let mut body = Vec::new();
        while !matches!(self.current, Token::RBrace) {
            body.push(self.parse_stmt());
        }
        self.consume(Token::RBrace);
        Stmt::While(cond, body, self.line)
    }

    fn parse_print(&mut self) -> Stmt {
        self.consume(Token::Ident("print".into()));
        self.consume(Token::LParen);
        let mut args = Vec::new();
        if !matches!(self.current, Token::RParen) {
            loop {
                args.push(self.parse_expr());
                if matches!(self.current, Token::Comma) {
                    self.consume(Token::Comma);
                } else {
                    break;
                }
            }
        }
        self.consume(Token::RParen);
        self.consume(Token::Semicolon);
        Stmt::Print(args, self.line)
    }

    fn parse_expr_stmt(&mut self) -> Stmt {
        // 记录语句起始行（tokenizer 已跳过空白/换行定位到表达式首 token）
        let stmt_line = self.line;
        let expr = self.parse_expr();
        // 分号规则：一般位置必须有分号；仅块末尾（右大括号/EOF 前）允许省略，作为隐式返回值
        if matches!(self.current, Token::Semicolon) {
            self.consume(Token::Semicolon);
        } else if !matches!(self.current, Token::RBrace | Token::Eof) {
            script_panic_at(&self.file, stmt_line, 1, "表达式语句必须以分号结尾");
        }
        Stmt::Expr(expr, stmt_line)
    }

    // 逻辑或 or 优先级低于 and
    fn parse_or(&mut self) -> Expr {
        let mut expr = self.parse_logic();
        while matches!(self.current, Token::OrKey) {
            self.consume(Token::OrKey);
            let rhs = self.parse_logic();
            expr = Expr::BinOp(Box::new(expr), Op::Or, Box::new(rhs), self.line);
        }
        expr
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_assignment()
    }

    fn parse_assignment(&mut self) -> Expr {
        let expr = self.parse_or(); 
        if matches!(self.current, Token::Eq) {
            self.consume(Token::Eq);
            let value = self.parse_assignment();
            match expr {
                Expr::Ident(name, _) => {
                    if self.const_names.contains(&name) {
                        self.panic_here("静态检查失败：常量不允许赋值");
                    }
                    Expr::Assign(name, Box::new(value), self.line)
                }
                Expr::Index(arr, idx, _) => {
                    if let Expr::Ident(arr_name, _) = &*arr {
                        if self.const_names.contains(arr_name) {
                            self.panic_here( &format!("静态检查失败：常量数组 {} 不允许修改", arr_name));
                        }
                    }
                    Expr::IndexAssign(arr, idx, Box::new(value), self.line)
                }
                // 解析 *ptr = val 生成专属指针赋值节点
                Expr::Deref(inner_ptr, _) => {
                    Expr::DerefAssign(inner_ptr, Box::new(value), self.line)
                }
                // 解析 obj.field = val 实例字段赋值
                Expr::Member(obj, member, _) => {
                    Expr::MemberAssign(obj, member, Box::new(value), self.line)
                }
                // 解析 obj.#field = val 私有字段赋值
                Expr::PrivateMember(obj, member, _) => {
                    Expr::PrivateMemberAssign(obj, member, Box::new(value), self.line)
                }
                _ => self.panic_here( "左侧不支持赋值")
            }
        } else {
            expr
        }
    }

    // 逻辑与：and 优先级低于比较运算
    fn parse_logic(&mut self) -> Expr {
        let mut expr = self.parse_cmp();
        while matches!(self.current, Token::AndKey) {
            self.consume(Token::AndKey);
            let rhs = self.parse_cmp();
            expr = Expr::BinOp(Box::new(expr), Op::And, Box::new(rhs), self.line);
        }
        expr
    }

    fn parse_cmp(&mut self) -> Expr {
        let mut expr = self.parse_term();
        while matches!(self.current, Token::Gt | Token::Lt | Token::Ge | Token::Le | Token::Equal | Token::Neq | Token::QuestionEqual) {
            let op = match self.current {
                Token::Gt => Op::Gt,
                Token::Lt => Op::Lt,
                Token::Ge => Op::Ge,
                Token::Le => Op::Le,
                Token::Equal => Op::Equal,
                Token::Neq => Op::Neq,
                Token::QuestionEqual => Op::RelCmp,
                _ => unreachable!(),
            };
            self.consume(op_token(&op));
            let right = self.parse_term();
            expr = Expr::BinOp(Box::new(expr), op, Box::new(right), self.line);
        }
        expr
    }

    fn parse_term(&mut self) -> Expr {
        let mut expr = self.parse_factor();

        // 二元 + - 减法，不再捕获前缀负号
        while matches!(self.current, Token::Plus | Token::Minus) {
            let op = match self.current {
                Token::Plus => Op::Add,
                Token::Minus => Op::Sub,
                _ => unreachable!(),
            };
            self.consume(op_token(&op));
            let right = self.parse_factor();
            expr = Expr::BinOp(Box::new(expr), op, Box::new(right), self.line);
        }
        expr
    }

    fn parse_factor(&mut self) -> Expr {
        let mut ops: Vec<u8> = Vec::new(); // 0 = 负号, 1 = 逻辑非
        loop {
            if matches!(self.current, Token::Minus) { self.consume(Token::Minus); ops.push(0); }
            else if matches!(self.current, Token::Not) { self.consume(Token::Not); ops.push(1); }
            else { break; }
        }

        let mut expr = self.parse_primary();

        // 后缀类型转换：expr -> i32/i64/float/str/bool
        while matches!(self.current, Token::ArrowR) {
            self.consume(Token::ArrowR);
            let ty = match &self.current {
                Token::Ident(s) => s.clone(),
                _ => self.panic_here( "语法错误：-> 后需要类型名"),
            };
            self.consume(Token::Ident(ty.clone()));
            expr = Expr::Cast(Box::new(expr), ty, self.line);
        }

        // 修复核心：右侧改用 parse_primary，消除递归死循环
        while matches!(self.current, Token::Star | Token::Slash | Token::Percent) {
            let op = match self.current {
                Token::Star => Op::Mul,
                Token::Slash => Op::Div,
                Token::Percent => Op::Mod,
                _ => unreachable!(),
            };
            self.consume(op_token(&op));
            let right = self.parse_primary();
            expr = Expr::BinOp(Box::new(expr), op, Box::new(right), self.line);
        }

        // 按原始顺序逆序应用前缀：-x / !x / -!x / !-x 均保持正确语义
        for &op in ops.iter().rev() {
            if op == 0 {
                if self.has_ptr_expr(&expr) {
                    self.panic_here( "解析错误：指针不支持取负运算");
                }
                expr = match expr {
                    // 区间取负：负号作用于左端点（右端点的负号已由内部 parse_expr 处理）
                    Expr::Range(l, r, ln) => Expr::Range(Box::new(Expr::Neg(l, ln)), r, ln),
                    other => Expr::Neg(Box::new(other), self.line),
                };
            } else {
                expr = Expr::Not(Box::new(expr), self.line);
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        // 处理两类取地址：普通 & / unsafe &
        match self.current {
            Token::And => {
                // 普通 & 取地址：允许任意位置（原有规则不变）
                self.consume(Token::And);
                let inner = self.parse_primary();

                if matches!(inner, Expr::String(..) | Expr::RawString(..)) {
                    self.panic_here( "解析错误：禁止对字符串字面量使用取地址 &");
                }
                if matches!(inner, Expr::AddrOf(..) | Expr::RawAddr(..)) {
                    self.panic_here( "解析错误：禁止连续嵌套取地址");
                }

                return Expr::AddrOf(Box::new(inner),self.line);
            }
            // ========= unsafe & 裸指针：纯解析期强制判断 in_unsafe_block =========
            Token::Unsafe => {
                // 【关键】不管有没有类型检查，这里直接拦截
                if !self.in_unsafe_block {
                    script_panic(
                        &self.file,
                        self.line,
                        "解析错误：unsafe & 裸指针语法必须放在 unsafe {} 代码块内"
                    );
                }

                self.consume(Token::Unsafe);
                if self.current != Token::And {
                    self.panic_here( "unsafe 后必须紧跟 & 进行裸指针取地址");
                }
                self.consume(Token::And);
                let inner = self.parse_primary();

                if matches!(inner, Expr::String(..) | Expr::RawString(..)) {
                    self.panic_here( "解析错误：禁止对字符串字面量使用裸指针取地址");
                }
                if matches!(inner, Expr::AddrOf(..) | Expr::RawAddr(..)) {
                    self.panic_here( "解析错误：禁止连续嵌套取地址");
                }

                return Expr::RawAddr(Box::new(inner), self.line);
            }
            // 解引用 *
            Token::StarDeref => {
                self.consume(Token::StarDeref);
                if matches!(self.current, Token::RParen | Token::Semicolon | Token::RBrace | Token::Comma) {
                    self.panic_here( "解析错误：解引用 * 缺少操作数");
                }
                let inner = self.parse_primary();
                return Expr::Deref(Box::new(inner), self.line);
            }
            _ => {}
        }

        let expr = match &self.current {
            Token::Match => self.parse_match(),
            Token::Fn => self.parse_lambda(),
            Token::LBrace => self.parse_dict(),
            Token::Number(n) => {
                let mut val = *n;
                self.consume(Token::Number(val));
                Expr::Number(val, self.line)
            }
            Token::Int64(n) => {
                let val = *n;
                self.consume(Token::Int64(val));
                Expr::Int64(val, self.line)
            }
            Token::Float(n) => {
                let mut val = *n;
                self.consume(Token::Float(val));
                Expr::Float(val, self.line)
            }
            Token::String(full) => {
                let parts: Vec<&str> = full.split("||LINE||").collect();
                let s = parts[0].to_string();
                let line: u32 = parts[1].parse().unwrap_or(self.line);
                self.consume(Token::String(full.clone()));
                Expr::String(s, line)
            }
            Token::RawString(full) => {
                let parts: Vec<&str> = full.split("||LINE||").collect();
                let s = parts[0].to_string();
                let line: u32 = parts[1].parse().unwrap_or(self.line);
                self.consume(Token::RawString(full.clone()));
                Expr::RawString(s, line)
            }
            Token::True => {
                self.consume(Token::True);
                Expr::Bool(true, self.line)
            }
            Token::False => {
                self.consume(Token::False);
                Expr::Bool(false, self.line)
            }
            Token::Ident(name) => {
                let name = name.clone();
                let ident_line = self.line; // 解析标识符前立刻捕获行号
                self.consume(Token::Ident(name.clone()));
                let mut full_name;
                // 首标识符是已知模块根（std/lib/已导入模块）时，`.` 与 `::` 均作命名空间分隔符
                let is_mod_root = self.known_modules.contains(&name)
                    || self.known_modules.iter().any(|m| m.starts_with(&format!("{}::", name)));
                if is_mod_root {
                    full_name = name;
                    loop {
                        if self.current == Token::ColonColon {
                            self.consume(Token::ColonColon);
                            let next = match &self.current {
                                Token::Ident(s) => s.clone(),
                                _ => self.panic_here( ":: 后必须是标识符"),
                            };
                            self.consume(Token::Ident(next.clone()));
                            full_name = format!("{}::{}", full_name, next);
                        } else if self.current == Token::Dot {
                            self.consume(Token::Dot);
                            let next = match &self.current {
                                Token::Ident(s) => s.clone(),
                                _ => self.panic_here( ". 后必须是标识符"),
                            };
                            self.consume(Token::Ident(next.clone()));
                            full_name = format!("{}::{}", full_name, next);
                        } else {
                            break;
                        }
                    }
                } else {
                    full_name = name;
                    while self.current == Token::ColonColon {
                        self.consume(Token::ColonColon);
                        let next = match &self.current {
                            Token::Ident(s) => s.clone(),
                            _ => self.panic_here( ":: 后必须是标识符"),
                        };
                        self.consume(Token::Ident(next.clone()));
                        full_name = format!("{}::{}", full_name, next);
                    }
                }
                if matches!(self.current, Token::LParen) {
                    self.consume(Token::LParen);
                    let mut args = Vec::new();
                    if !matches!(self.current, Token::RParen) {
                        loop {
                            args.push(self.parse_expr());
                            if matches!(self.current, Token::Comma) {
                                self.consume(Token::Comma);
                            } else {
                                break;
                            }
                        }
                    }
                    self.consume(Token::RParen);
                    Expr::Call(full_name, args, ident_line) // 使用捕获的ident_line
                } else {
                    Expr::Ident(interner().get(&full_name), ident_line) // 存入未偏移的原始行号
                }
            }
            Token::LParen => {
                self.consume(Token::LParen);
                let e = self.parse_expr();
                self.consume(Token::RParen);
                e
            }
            Token::LSquare => {
                self.consume(Token::LSquare);
                let mut items = Vec::new();
                if !matches!(self.current, Token::RSquare) {
                    loop {
                        items.push(self.parse_expr());
                        if matches!(self.current, Token::Comma) {
                            self.consume(Token::Comma);
                        } else {
                            break;
                        }
                    }
                }
                self.consume(Token::RSquare);
                Expr::Array(items, self.line)
            }
            _ => self.panic_here( &format!("语法错误：意外 token {:?}", self.current))
        };

        let mut expr = expr;
        loop {
            match self.current {
                Token::Range => {
                    self.consume(Token::Range);
                    let r = self.parse_expr();
                    expr = Expr::Range(Box::new(expr), Box::new(r), self.line);
                }
                Token::LSquare => {
                    self.consume(Token::LSquare);
                    let idx = self.parse_expr();
                    self.consume(Token::RSquare);
                    expr = Expr::Index(Box::new(expr), Box::new(idx), self.line);
                }
                Token::Dot => {
                    self.consume(Token::Dot);
                    if matches!(self.current, Token::Hash) {
                        // 私有成员访问：obj.#name
                        self.consume(Token::Hash);
                        let pname = match &self.current {
                            Token::Ident(s) => s.clone(),
                            _ => self.panic_here("# 后必须是字段名"),
                        };
                        self.consume(Token::Ident(pname.clone()));
                        expr = Expr::PrivateMember(Box::new(expr), interner().get(&pname), self.line);
                    } else {
                    let name = match &self.current {
                        Token::Ident(s) => s.clone(),
                        _ => self.panic_here(". 后面必须是标识符"),
                    };
                    self.consume(Token::Ident(name.clone()));
                    if matches!(self.current, Token::LParen) {
                        // 方法调用 obj.method(args)
                        self.consume(Token::LParen);
                        let mut args = Vec::new();
                        if !matches!(self.current, Token::RParen) {
                            loop {
                                args.push(self.parse_expr());
                                if matches!(self.current, Token::Comma) {
                                    self.consume(Token::Comma);
                                } else {
                                    break;
                                }
                            }
                        }
                        self.consume(Token::RParen);
                        expr = Expr::MethodCall(Box::new(expr), interner().get(&name), args, self.line);
                    } else {
                        expr = Expr::Member(Box::new(expr), interner().get(&name), self.line);
                    }
                }
                }
                _ => break,
            }
        }
        expr
    }

    fn consume(&mut self, expected: Token) {
        if std::mem::discriminant(&self.current) == std::mem::discriminant(&expected) {
            self.current = self.tokenizer.next_token(&self.file, &mut self.line);
        } else {
            self.panic_here(&format!("语法解析失败：期望 Token {:?}，实际读到 {:?}", expected, self.current));
        }
    }

    /// 以当前 token 的（行,列）位置抛出脚本错误
    fn panic_here(&self, msg: &str) -> ! {
        script_panic_at(&self.file, self.line, self.tokenizer.last_col, msg)
    }

    /// 递归检查表达式是否包含指针（结合静态类型环境）
    fn parse_match(&mut self) -> Expr {
        let ln = self.line;
        self.consume(Token::Match);
        let subject = self.parse_expr();
        if self.current != Token::LBrace {
            self.panic_here("解析错误：match 后必须跟 {");
        }
        self.consume(Token::LBrace);
        let mut arms = Vec::new();
        while self.current != Token::RBrace {
            let pat = self.parse_match_pat();
            if self.current != Token::Arrow {
                self.panic_here("解析错误：match 分支必须使用 => 连接");
            }
            self.consume(Token::Arrow);
            let body = self.parse_expr();
            arms.push((pat, body));
            if self.current == Token::Comma {
                self.consume(Token::Comma);
            } else if self.current != Token::RBrace {
                self.panic_here("解析错误：match 分支之间需要用逗号分隔");
            }
        }
        self.consume(Token::RBrace);
        Expr::Match(Box::new(subject), arms, ln)
    }

    fn parse_match_pat(&mut self) -> MatchPat {
        // 每个分支显式推进并设置 self.current，支持多 token 的区间模式
        let cur = std::mem::replace(&mut self.current, Token::Eof);
        let result = match cur {
            Token::Underscore => {
                self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                MatchPat::Wildcard
            }
            Token::Number(n) => {
                let nxt = self.tokenizer.next_token(&self.file, &mut self.line);
                if nxt == Token::Range {
                    let end = self.tokenizer.next_token(&self.file, &mut self.line);
                    let r = match end {
                        Token::Number(m) => MatchPat::Range(n as f64, m as f64),
                        Token::Float(m) => MatchPat::Range(n as f64, m),
                        Token::Minus => {
                            let neg = self.tokenizer.next_token(&self.file, &mut self.line);
                            match neg {
                                Token::Number(m) => MatchPat::Range(n as f64, -(m as f64)),
                                Token::Float(m) => MatchPat::Range(n as f64, -m),
                                _ => self.panic_here("解析错误：区间右端点必须是数字"),
                            }
                        }
                        _ => self.panic_here("解析错误：区间右端点必须是数字"),
                    };
                    self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                    r
                } else {
                    self.current = nxt;
                    MatchPat::Lit(Value::Int(n))
                }
            }
            Token::Float(f) => {
                let nxt = self.tokenizer.next_token(&self.file, &mut self.line);
                if nxt == Token::Range {
                    let end = self.tokenizer.next_token(&self.file, &mut self.line);
                    let r = match end {
                        Token::Number(m) => MatchPat::Range(f, m as f64),
                        Token::Float(m) => MatchPat::Range(f, m),
                        Token::Minus => {
                            let neg = self.tokenizer.next_token(&self.file, &mut self.line);
                            match neg {
                                Token::Number(m) => MatchPat::Range(f, -(m as f64)),
                                Token::Float(m) => MatchPat::Range(f, -m),
                                _ => self.panic_here("解析错误：区间右端点必须是数字"),
                            }
                        }
                        _ => self.panic_here("解析错误：区间右端点必须是数字"),
                    };
                    self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                    r
                } else {
                    self.current = nxt;
                    MatchPat::Lit(Value::Float(f))
                }
            }
            Token::String(full) => {
                let parts: Vec<&str> = full.split("||LINE||").collect();
                self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                MatchPat::Lit(Value::String(PoolStr::new(parts[0])))
            }
            Token::True => {
                self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                MatchPat::Lit(Value::Bool(true))
            }
            Token::False => {
                self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                MatchPat::Lit(Value::Bool(false))
            }
            Token::Minus => {
                let cur2 = self.tokenizer.next_token(&self.file, &mut self.line);
                match cur2 {
                    Token::Number(n) => {
                        let nxt = self.tokenizer.next_token(&self.file, &mut self.line);
                        if nxt == Token::Range {
                            let end = self.tokenizer.next_token(&self.file, &mut self.line);
                            let r = match end {
                                Token::Number(m) => MatchPat::Range(-(n as f64), m as f64),
                                Token::Float(m) => MatchPat::Range(-(n as f64), m),
                                Token::Minus => {
                                    let neg = self.tokenizer.next_token(&self.file, &mut self.line);
                                    match neg {
                                        Token::Number(m) => MatchPat::Range(-(n as f64), -(m as f64)),
                                        Token::Float(m) => MatchPat::Range(-(n as f64), -m),
                                        _ => self.panic_here("解析错误：区间右端点必须是数字"),
                                    }
                                }
                                _ => self.panic_here("解析错误：区间右端点必须是数字"),
                            };
                            self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                            r
                        } else {
                            self.current = nxt;
                            MatchPat::Lit(Value::Int(-n))
                        }
                    }
                    Token::Float(f) => {
                        let nxt = self.tokenizer.next_token(&self.file, &mut self.line);
                        if nxt == Token::Range {
                            let end = self.tokenizer.next_token(&self.file, &mut self.line);
                            let r = match end {
                                Token::Number(m) => MatchPat::Range(-f, m as f64),
                                Token::Float(m) => MatchPat::Range(-f, m),
                                Token::Minus => {
                                    let neg = self.tokenizer.next_token(&self.file, &mut self.line);
                                    match neg {
                                        Token::Number(m) => MatchPat::Range(-f, -(m as f64)),
                                        Token::Float(m) => MatchPat::Range(-f, -m),
                                        _ => self.panic_here("解析错误：区间右端点必须是数字"),
                                    }
                                }
                                _ => self.panic_here("解析错误：区间右端点必须是数字"),
                            };
                            self.current = self.tokenizer.next_token(&self.file, &mut self.line);
                            r
                        } else {
                            self.current = nxt;
                            MatchPat::Lit(Value::Float(-f))
                        }
                    }
                    _ => self.panic_here("解析错误：match 负数模式后必须是数字"),
                }
            }
            _ => self.panic_here("解析错误：match 分支必须以字面量、区间或 _ 开头"),
        };
        result
    }

    fn has_ptr_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Member(e,_,_) => self.has_ptr_expr(e),
            Expr::AddrOf(_, _) | Expr::RawAddr(_, _) | Expr::Deref(_,_) => true,
            Expr::DerefAssign(_, _, _) => true,
            Expr::Neg(e, _) => self.has_ptr_expr(e),
            Expr::Not(e, _) => self.has_ptr_expr(e),
            Expr::MethodCall(obj, _, args, _) => self.has_ptr_expr(obj) || args.iter().any(|a| self.has_ptr_expr(a)),
            Expr::MemberAssign(obj, _, rhs, _) => self.has_ptr_expr(obj) || self.has_ptr_expr(rhs),
            Expr::PrivateMember(e, _, _) => self.has_ptr_expr(e),
            Expr::PrivateMemberAssign(obj, _, rhs, _) => self.has_ptr_expr(obj) || self.has_ptr_expr(rhs),

            Expr::Ident(name, _) => {
                // 从静态类型环境查询该变量类型是否为指针
                match self.type_env.get_id(*name) {
                    Some(Type::ArenaPtr) | Some(Type::RawPtr) => true,
                    _ => false
                }
            }

            Expr::BinOp(l, _, r, _) => self.has_ptr_expr(l) || self.has_ptr_expr(r),
            Expr::Index(e1, e2, _) | Expr::IndexAssign(e1, e2, _, _) => {
                self.has_ptr_expr(e1) || self.has_ptr_expr(e2)
            }
            Expr::Call(_, args,_) => args.iter().any(|a| self.has_ptr_expr(a)),
            Expr::Array(items,_) => items.iter().any(|i| self.has_ptr_expr(i)),
            Expr::Assign(_, rhs, _) => self.has_ptr_expr(rhs),
            Expr::Number(..) | Expr::Float(..) | Expr::String(..)
            | Expr::RawString(..) | Expr::Bool(..) | Expr::Lambda(_, _) => false,
            Expr::Dict(items, _) => items.iter().any(|(_, v)| self.has_ptr_expr(v)),
            Expr::Match(s, arms, _) => {
                if self.has_ptr_expr(s) { return true; }
                arms.iter().any(|(_, b)| self.has_ptr_expr(b))
            }
            Expr::Range(l, r, _) => self.has_ptr_expr(l) || self.has_ptr_expr(r),
            Expr::Int64(_, _) => false,
            Expr::Cast(e, _, _) => self.has_ptr_expr(e),
        }
    }
}

fn op_token(op: &Op) -> Token {
    match op {
        Op::Add => Token::Plus,
        Op::Sub => Token::Minus,
        Op::Mul => Token::Star,
        Op::Div => Token::Slash,
        Op::Mod => Token::Percent,
        Op::Gt => Token::Gt,
        Op::Lt => Token::Lt,
        Op::Equal => Token::Equal,
        Op::Neq => Token::Neq,
        Op::Ge => Token::Ge,
        Op::Le => Token::Le, 
        Op::And => Token::AndKey, 
        Op::Or => Token::OrKey,
        Op::RelCmp => Token::QuestionEqual,
    }
}

// 生成器：惰性 yield 执行（首版仅支持平铺 yield，控制流内 yield 暂不支持）
struct GeneratorData {
    body: Vec<Stmt>, // 剩余待执行语句（每次 yield 后截取）
    env: Env,        // 生成器局部环境（含形参绑定，outer=定义处父环境）
    done: bool,
}

fn stmt_has_yield(s: &Stmt) -> bool {
    match s {
        Stmt::Yield(..) => true,
        Stmt::If(_, t, e, el, _) => {
            if body_has_yield(t) || body_has_yield(el) { return true; }
            e.iter().any(|(_, b)| body_has_yield(b))
        }
        Stmt::While(_, b, _) | Stmt::UnsafeBlock(b, _) => body_has_yield(b),
        Stmt::For(_, _, _, b, _) => body_has_yield(b),
        Stmt::Try { body, handler, .. } => body_has_yield(body) || body_has_yield(handler),
        _ => false,
    }
}
fn body_has_yield(stmts: &[Stmt]) -> bool {
    stmts.iter().any(stmt_has_yield)
}

impl std::fmt::Debug for GeneratorData {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Generator(done={})", self.done)
    }
}
fn body_has_nested_yield(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match s { Stmt::Yield(..) => false, _ => stmt_has_yield(s) })
}

/// 在函数体内查找控制流（if/while/for）内的首个 yield 位置 (line, col)
fn find_nested_yield_pos(stmts: &[Stmt]) -> Option<(u32, u32)> {
    for s in stmts {
        if let Some(p) = nested_yield_in_blocks(s) { return Some(p); }
    }
    None
}
fn nested_yield_in_blocks(s: &Stmt) -> Option<(u32, u32)> {
    let subs: Vec<&[Stmt]> = match s {
        Stmt::If(_, t, elifs, e2, _) => {
            let mut v = vec![t.as_slice()];
            for (_, b) in elifs { v.push(b.as_slice()); }
            v.push(e2.as_slice());
            v
        }
        Stmt::While(_, b, _) => vec![b.as_slice()],
        Stmt::For(_, _, _, b, _) => vec![b.as_slice()],
        _ => return None,
    };
    for blk in subs {
        for inner in blk {
            if let Stmt::Yield(_, ln) = inner { return Some((*ln, 1)); }
            if let Some(pp) = nested_yield_in_blocks(inner) { return Some(pp); }
        }
    }
    None
}

// ==============================
// 运行时值 Value
// ==============================
#[derive(Debug, Clone)]
enum Value {
    Int(i32),
    Int64(i64),
    Float(f64),
    String(PoolStr),
    Bool(bool),
    Array(Rc<Vec<Value>>),
    // Arena 内存指针：仅指向全局/命名Arena分配的内存，安全托管
    ArenaPtr(*mut u8),
    // Raw 裸指针（C风格，外部/堆内存）
    RawPtr(*mut u8),
    // 安全数组元素指针：数组变量名 + 下标，无裸地址
    ArrayElementPtr(String, usize),
    // 类对象：共享类定义（只读）
    Class(Rc<ClassDef>),
    // 实例对象：共享类定义 + 可变字段表
    Instance(Rc<RefCell<InstanceData>>),
    // 闭包：函数体 + 捕获的外部变量共享单元
    Closure(Rc<ClosureData>),
    // 字典/映射：字符串键 -> 值
    Dict(Rc<FastMap<String, Value>>),
    // 区间值：lo..hi（含端点），match 区间模式与区间表达式共享
    Range(f64, f64),
    // 生成器对象：惰性 yield，next() 推进
    Generator(Rc<RefCell<GeneratorData>>),
}

// 用户 throw 的异常标记（panic payload）。实际异常值存于 Interpreter.pending_throw，避免 Rc 非 Send 问题
// 数组/字典写时复制（COW）辅助：读共享 Rc（clone 只复制引用头）、写走 Rc::make_mut，
// 仅当底层被多处共享时才深拷贝，保持值语义的同时让“读变量/传参/返回”从深拷贝降为 O(1)。
fn value_array_mut(v: &mut Value) -> &mut Vec<Value> {
    if let Value::Array(rc) = v {
        Rc::make_mut(rc)
    } else {
        panic!("value_array_mut: 非数组值");
    }
}
fn value_dict_mut(v: &mut Value) -> &mut FastMap<String, Value> {
    if let Value::Dict(rc) = v {
        Rc::make_mut(rc)
    } else {
        panic!("value_dict_mut: 非字典值");
    }
}

// match 模式匹配的值相等比较（与 == 同类型语义一致）
fn value_matches(pat: &Value, subj: &Value) -> bool {
    match (pat, subj) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Int64(a), Value::Int64(b)) => a == b,
        (Value::Int64(a), Value::Int(b)) => *a == *b as i64,
        (Value::Int(a), Value::Int64(b)) => (*a as i64) == *b,
        (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
        (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
        (Value::Int64(a), Value::Float(b)) => (*a as f64) == *b,
        (Value::Float(a), Value::Int64(b)) => *a == (*b as f64),
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::String(a), Value::String(b)) => a.as_str() == b.as_str(),
        _ => false,
    }
}

struct ThrowMarker;

/// 将 Value 格式化为展示字符串（用于未捕获异常渲染）
fn fmt_value(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Int64(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.as_str().to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(fmt_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::ArenaPtr(_) => "(ArenaPtr)".to_string(),
        Value::RawPtr(_) => "(RawPtr)".to_string(),
        Value::ArrayElementPtr(name, idx) => format!("(ArrayElementPtr) &{name}[{idx}]"),
        Value::Class(c) => format!("(class {})", c.name),
        Value::Instance(i) => format!("(instance of {})", i.borrow().class.name),
        Value::Closure(_) => "(closure)".to_string(),
        Value::Generator(_) => "(generator)".to_string(),
        Value::Dict(map) => {
            let items: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", k, fmt_value(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
        Value::Range(lo, hi) => format!("{}..{}", fmt_range_num(*lo), fmt_range_num(*hi)),
    }
}

// 区间端点数字格式化：整数省略小数点
fn fmt_range_num(f: f64) -> String {
    if f.fract() == 0.0 {
        format!("{}", f as i64)
    } else {
        format!("{}", f)
    }
}

// 隐藏类字段布局：字段名 id -> 槽位索引（沿继承链合并，祖先字段在前）
#[derive(Debug)]
struct FieldLayout {
    index: FastMap<u32, usize>,
    total: usize,
}

// 实例数据：所属类 + 字段槽（按 FieldLayout 索引，字段访问 O(1)）
#[derive(Debug)]
struct InstanceData {
    class: Rc<ClassDef>,
    fields: Vec<Value>,
}

// ---------- GUI（egui/eframe，feature "gui" 可选） ----------
// 纯解释器包（xlang.exe）不含 GUI；GUI 版是独立子包 gui/（xlang_gui.exe），
// include! 本文件并常开 feature "gui"。
// 由 gui_window 的 builder 闭包收集的控件声明，在 eframe 事件循环中渲染
#[cfg(feature = "gui")]
#[derive(Clone)]
enum CanvasCmd {
    Line { x1: f32, y1: f32, x2: f32, y2: f32, color: [u8; 4], width: f32 },
    Rect { x: f32, y: f32, w: f32, h: f32, color: [u8; 4] },
    Circle { cx: f32, cy: f32, r: f32, color: [u8; 4] },
}

#[cfg(feature = "gui")]
#[derive(Clone)]
enum GuiControl {
    Button { label: String, onclick: Value, size: f32 },
    Text { text: String },
    Heading { text: String },
    Separator,
    Spacer,
    Input { label: String, value: String, onchange: Value },
    InputVar { label: String, unit: Rc<RefCell<Value>> },
    TextArea { label: String, unit: Rc<RefCell<Value>> },
    Output { unit: Rc<RefCell<Value>>, last: Option<String> },
    Terminal { unit: Rc<RefCell<Value>>, onsubmit: Value },  // 可编辑输出区：可自由输入，Ctrl+Enter 提交
    Checkbox { label: String, value: bool, onchange: Value },
    Slider { label: String, min: f64, max: f64, value: f64, onchange: Value },
    Row(Vec<GuiControl>),
    Col(Vec<GuiControl>),
    TopBar { children: Vec<GuiControl>, horizontal: bool },  // 顶部工具条：true=横着平铺(一行) / false=竖着平铺(多行)
    SideBar(Vec<GuiControl>),  // 左侧面板（文件列表等）
    BottomBar(Vec<GuiControl>),  // 底部面板（输出区等，固定可见不被编辑区挤走）
    ColorEdit { label: String, color: [u8; 4], onchange: Value },
    Combo { label: String, options: Vec<String>, selected: usize, onchange: Value },
    Radio { label: String, options: Vec<String>, selected: usize, onchange: Value },
    Table { headers: Vec<String>, rows: Vec<Vec<Value>>, onrow: Option<Value> },
    Multiselect { label: String, options: Vec<String>, selected: Vec<usize>, onchange: Value },
    Tabs { names: Vec<String>, selected: usize, onchange: Value },
    Canvas { width: f32, height: f32, cmds: Vec<CanvasCmd> },
}

// 加载中文字体（egui 默认字体不含 CJK，中文会显示为方块）；找不到系统字体则回退默认
#[cfg(feature = "gui")]
fn setup_cjk_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhbd.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("cjk".to_owned(), std::sync::Arc::new(egui::FontData::from_owned(data)));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push("cjk".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

// eframe 应用：渲染控件，点击时通过裸指针执行 XLang 回调闭包
#[cfg(feature = "gui")]
mod logo_icon {
    pub const W: u32 = 128;
    pub const H: u32 = 128;
    pub const RGBA: &[u8] = include_bytes!("logo_icon.rgba");
}

#[cfg(feature = "gui")]
struct GuiApp {
    controls: Vec<GuiControl>,
    interp: *mut Interpreter,
    /// 面板分类缓存：控件拓扑在脚本收集阶段固定、运行期不变，
    /// 仅首次（或 dirty）重建分类，之后每帧复用，避免反复遍历 self.controls
    classify_dirty: bool,
    top: Option<(usize, bool)>,
    side: Option<usize>,
    bottom: Option<usize>,
    central: Vec<usize>,
}
#[cfg(feature = "gui")]
impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 记录当前 egui 上下文，供 quit 在回调里关闭窗口
        let ctx_ptr = ctx as *const egui::Context as *mut egui::Context;
        unsafe { (*self.interp).gui_ctx = Some(ctx_ptr); }
        // 多面板布局分类（TopBar->顶部、SideBar->左、BottomBar->底部、其余->中央）。
        // 控件拓扑在脚本收集阶段固定、运行期不变，故用 dirty 标记只重建一次，避免每帧遍历。
        // 渲染仍按索引原地操作 self.controls（不克隆，保留输入框/复选框/滑杆等控件内状态）
        if self.classify_dirty {
            self.top = None;
            self.side = None;
            self.bottom = None;
            self.central.clear();
            for (i, c) in self.controls.iter().enumerate() {
                match c {
                    GuiControl::TopBar { horizontal, .. } => self.top = Some((i, *horizontal)),
                    GuiControl::SideBar(_) => self.side = Some(i),
                    GuiControl::BottomBar(_) => self.bottom = Some(i),
                    _ => self.central.push(i),
                }
            }
            self.classify_dirty = false;
        }
        let mut pending_top: Vec<(Value, Vec<Value>)> = Vec::new();
        if let Some((i, horizontal)) = self.top {
            egui::TopBottomPanel::top("ide_top").show(ctx, |ui| {
                egui::ScrollArea::horizontal().show(ui, |ui| {
                    if horizontal {
                        // 横着平铺：所有子控件一行横向排列
                        ui.horizontal(|ui| render_panel_children(ui, &mut self.controls, i, &mut pending_top, self.interp));
                    } else {
                        // 竖着平铺：每个子控件各占一行，多行堆叠
                        ui.vertical(|ui| render_panel_children(ui, &mut self.controls, i, &mut pending_top, self.interp));
                    }
                });
            });
        }
        let mut pending_side: Vec<(Value, Vec<Value>)> = Vec::new();
        if let Some(i) = self.side {
            egui::SidePanel::left("ide_side").default_width(230.0).resizable(false).show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| render_panel_children(ui, &mut self.controls, i, &mut pending_side, self.interp));
            });
        }
        let mut pending_bottom: Vec<(Value, Vec<Value>)> = Vec::new();
        if let Some(i) = self.bottom {
            egui::TopBottomPanel::bottom("ide_bottom").show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| render_panel_children(ui, &mut self.controls, i, &mut pending_bottom, self.interp));
            });
        }
        let mut pending_central: Vec<(Value, Vec<Value>)> = Vec::new();
        {
            // 借出 self.controls（可变）与 self.central（只读）为独立字段借用，避免闭包内自借用冲突
            let controls = &mut self.controls;
            let central = &self.central;
            let interp = self.interp;
            egui::CentralPanel::default().show(ctx, |ui| {
                // 顶层滚动区：内容超出窗口高度时可滚动（IDE 表/编辑器/输出均可达）
                egui::ScrollArea::vertical().show(ui, |ui| {
                    render_controls_idx(ui, controls, central, &mut pending_central, interp);
                });
            });
        }
        for (cb, args) in pending_top.into_iter().chain(pending_side).chain(pending_bottom).chain(pending_central) {
            if let Value::Closure(c) = cb {
                // 单线程事件循环内执行回调闭包
                unsafe { (*self.interp).call_closure(&c, args, 0); }
            }
        }
        // 轮询 gui_run 异步结果：写回共享变量（terminal/output 实时刷新）并请求重绘
        unsafe {
            let i = &mut *self.interp;
            let mut to_remove = Vec::new();
            for (idx, rx) in i.gui_async_rx.iter_mut().enumerate() {
                let mut got = false;
                while let Ok((vn, out)) = rx.try_recv() {
                    let id = interner().get(&vn);
                    i.env.set_id(id, Value::String(PoolStr::new(&out)));
                    ctx.request_repaint();
                    got = true;
                }
                if got { to_remove.push(idx); }
            }
            for &j in to_remove.iter().rev() { i.gui_async_rx.remove(j); }
        }
    }
}

// 渲染面板容器控件（TopBar/SideBar/BottomBar）的子控件（不克隆，原地渲染保住控件状态）
#[cfg(feature = "gui")]
fn render_panel_children(ui: &mut egui::Ui, controls: &mut Vec<GuiControl>, idx: usize, pending: &mut Vec<(Value, Vec<Value>)>, interp: *mut Interpreter) {
    let children: &mut Vec<GuiControl> = match &mut controls[idx] {
        GuiControl::TopBar { children, .. } | GuiControl::SideBar(children) | GuiControl::BottomBar(children) => children,
        _ => return,
    };
    render_controls(ui, children, pending, interp);
}

// 按索引渲染 self.controls 的子集（中央面板）
#[cfg(feature = "gui")]
fn render_controls_idx(ui: &mut egui::Ui, controls: &mut Vec<GuiControl>, idxs: &[usize], pending: &mut Vec<(Value, Vec<Value>)>, interp: *mut Interpreter) {
    for &i in idxs {
        render_one(ui, &mut controls[i], pending, interp);
    }
}

// 递归渲染控件树（支持 Row/Col 嵌套布局），回调收集到 pending 由帧末统一执行
#[cfg(feature = "gui")]
fn render_controls(ui: &mut egui::Ui, controls: &mut Vec<GuiControl>, pending: &mut Vec<(Value, Vec<Value>)>, interp: *mut Interpreter) {
    for c in controls {
        render_one(ui, c, pending, interp);
    }
}

#[cfg(feature = "gui")]
fn render_one(ui: &mut egui::Ui, c: &mut GuiControl, pending: &mut Vec<(Value, Vec<Value>)>, interp: *mut Interpreter) {
    match c {
            GuiControl::Button { label, onclick, size } => {
                let mut btn = egui::Button::new(label.as_str());
                if *size > 0.0 { btn = btn.min_size(egui::vec2(64.0, *size)); }
                if ui.add(btn).clicked() { pending.push((onclick.clone(), Vec::new())); }
            }
            GuiControl::Text { text } => { ui.label(text.as_str()); }
            GuiControl::Heading { text } => { ui.heading(text.as_str()); }
            GuiControl::Separator => { ui.separator(); }
            GuiControl::Spacer => { ui.add_space(8.0); }
            GuiControl::Input { label, value, onchange } => {
                if ui.add(egui::TextEdit::singleline(value).hint_text(label.as_str())).changed() {
                    pending.push((onchange.clone(), vec![Value::String(PoolStr::new(value))]));
                }
            }
            GuiControl::InputVar { label, unit } => {
                // 绑共享变量：编辑写回变量；外部清空/改变量→输入框实时刷新
                let mut s = match &*unit.borrow() {
                    Value::String(x) => x.as_str().to_string(),
                    _ => String::new(),
                };
                if ui.add(egui::TextEdit::singleline(&mut s).hint_text(label.as_str())).changed() {
                    *unit.borrow_mut() = Value::String(PoolStr::new(&s));
                }
            }
            GuiControl::Checkbox { label, value, onchange } => {
                if ui.checkbox(value, label.as_str()).changed() {
                    pending.push((onchange.clone(), vec![Value::Bool(*value)]));
                }
            }
            GuiControl::TextArea { label, unit } => {
                // 实时读共享变量显示；编辑后写回共享变量（IDE 联动：外部改变量→编辑器更新）
                let mut s = match &*unit.borrow() {
                    Value::String(x) => x.as_str().to_string(),
                    _ => String::new(),
                };
                if ui.add(egui::TextEdit::multiline(&mut s).hint_text(label.as_str()).desired_rows(20)).changed() {
                    *unit.borrow_mut() = Value::String(PoolStr::new(&s));
                }
            }
            GuiControl::Output { unit, last } => {
                // 只读输出：缓存上次字符串，共享值未变则不重新 to_string（大输出避免每帧复制）
                // borrow guard 需在显式作用域内用完即释放，避免 as_str 切片悬垂
                {
                    let guard = unit.borrow();
                    let cur = match &*guard {
                        Value::String(x) => Some(x.as_str()),
                        _ => None,
                    };
                    let changed = match (cur, last.as_deref()) {
                        (Some(c), Some(l)) => c != l,
                        (None, Some(_)) => true,
                        (Some(_), None) => true,
                        (None, None) => false,
                    };
                    if changed {
                        if let Some(c) = cur { *last = Some(c.to_string()); }
                        else { *last = None; }
                    }
                }
                if let Some(t) = last.as_deref() {
                    ui.label(t);
                }
            }
            GuiControl::Terminal { unit, onsubmit } => {
                // 可编辑终端：显示输出 + 可自由输入；Ctrl+Enter 提交输入给回调并清空
                let mut s = match &*unit.borrow() {
                    Value::String(x) => x.as_str().to_string(),
                    _ => String::new(),
                };
                let resp = egui::TextEdit::multiline(&mut s)
                    .desired_rows(18).hint_text("输出区；在此自由输入，Ctrl+Enter 运行").show(ui);
                if resp.response.changed() {
                    *unit.borrow_mut() = Value::String(PoolStr::new(&s));
                }
                if resp.response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.ctrl) {
                    // Ctrl+Enter 提交：焦点未丢失，不能用 lost_focus 判断；吃掉 TextEdit 刚插入的换行
                    let inp = s.trim_end_matches('\n').to_string();
                    *unit.borrow_mut() = Value::String(PoolStr::new(""));
                    pending.push((onsubmit.clone(), vec![Value::String(PoolStr::new(&inp))]));
                }
            }
            GuiControl::Slider { label, min, max, value, onchange } => {
                if ui.add(egui::Slider::new(value, *min..=*max).text(label.as_str())).changed() {
                    pending.push((onchange.clone(), vec![Value::Float(*value)]));
                }
            }
            GuiControl::TopBar { children, .. } | GuiControl::SideBar(children) | GuiControl::BottomBar(children) => {
                // 面板被嵌套在普通容器中时退化为内联渲染
                render_controls(ui, children, pending, interp);
            }
            GuiControl::Row(children) => {
                ui.horizontal(|ui| render_controls(ui, children, pending, interp));
            }
            GuiControl::Col(children) => {
                ui.vertical(|ui| render_controls(ui, children, pending, interp));
            }
            GuiControl::ColorEdit { label, color, onchange } => {
                let mut c32 = egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                if ui.color_edit_button_srgba(&mut c32).changed() {
                    *color = [c32.r(), c32.g(), c32.b(), c32.a()];
                    let arr = Value::Array(Rc::new(color.iter().map(|v| Value::Int(*v as i32)).collect()));
                    pending.push((onchange.clone(), vec![arr]));
                }
            }
            GuiControl::Combo { label, options, selected, onchange } => {
                let mut changed = false;
                let sel_text = options.get(*selected).cloned().unwrap_or_default();
                egui::ComboBox::from_label(label.as_str()).selected_text(sel_text).show_ui(ui, |ui| {
                    for (i, opt) in options.iter().enumerate() {
                        if ui.selectable_label(*selected == i, opt.as_str()).clicked() {
                            *selected = i;
                            changed = true;
                        }
                    }
                });
                if changed {
                    if let Some(opt) = options.get(*selected) {
                        pending.push((onchange.clone(), vec![Value::String(PoolStr::new(opt))]));
                    }
                }
            }
            GuiControl::Radio { label, options, selected, onchange } => {
                ui.label(label.as_str());
                let mut changed = false;
                for (i, opt) in options.iter().enumerate() {
                    if ui.radio_value(&mut *selected, i, opt.as_str()).changed() {
                        changed = true;
                    }
                }
                if changed {
                    if let Some(opt) = options.get(*selected) {
                        pending.push((onchange.clone(), vec![Value::String(PoolStr::new(opt))]));
                    }
                }
            }
            GuiControl::Table { headers, rows, onrow } => {
                let headers_ref: &Vec<String> = headers;
                let rows_ref: &Vec<Vec<Value>> = rows;
                // 稳定 id：用表头内容+行数（跨帧克隆时 as_ptr 会变，导致 Grid 每帧重建闪烁）
                let tid = egui::Id::new(("xlang_table", headers_ref.clone(), rows_ref.len()));
                egui::Grid::new(tid).striped(true).show(ui, |ui| {
                    for h in headers_ref.iter() { ui.strong(h.as_str()); }
                    ui.end_row();
                    for (ri, row) in rows_ref.iter().enumerate() {
                        // 首列可点击，触发 onrow 回调（传行索引）
                        if let Some(first) = row.first() {
                            if let Some(onrow) = onrow.as_ref() {
                                if ui.selectable_label(false, value_to_str_for_join(first)).clicked() {
                                    pending.push((onrow.clone(), vec![Value::Int(ri as i32)]));
                                }
                                for cell in row.iter().skip(1) { ui.label(value_to_str_for_join(cell)); }
                            } else {
                                for cell in row.iter() { ui.label(value_to_str_for_join(cell)); }
                            }
                        }
                        ui.end_row();
                    }
                });
            }
            GuiControl::Multiselect { label, options, selected, onchange } => {
                let mut changed = false;
                let sel_text = format!("{} 已选", selected.len());
                egui::ComboBox::from_label(label.as_str()).selected_text(sel_text).show_ui(ui, |ui| {
                    for (i, opt) in options.iter().enumerate() {
                        let is_sel = selected.contains(&i);
                        let text = if is_sel { format!("\u{2713} {}", opt) } else { opt.clone() };
                        if ui.selectable_label(is_sel, text).clicked() {
                            if is_sel { selected.retain(|&x| x != i); } else { selected.push(i); }
                            changed = true;
                        }
                    }
                });
                if changed {
                    let arr = Value::Array(Rc::new(selected.iter().map(|&i| Value::Int(i as i32)).collect()));
                    pending.push((onchange.clone(), vec![arr]));
                }
            }
            GuiControl::Tabs { names, selected, onchange } => {
                let mut changed = false;
                ui.horizontal(|ui| {
                    for (i, name) in names.iter().enumerate() {
                        if ui.selectable_label(*selected == i, name.as_str()).clicked() {
                            *selected = i;
                            changed = true;
                        }
                    }
                });
                if changed {
                    pending.push((onchange.clone(), vec![Value::Int(*selected as i32)]));
                }
            }
            GuiControl::Canvas { width, height, cmds } => {
                let (response, painter) = ui.allocate_painter(egui::vec2(*width, *height), egui::Sense::hover());
                let base = response.rect.min;
                for cmd in cmds.iter() {
                    match cmd {
                        CanvasCmd::Line { x1, y1, x2, y2, color, width: lw } => {
                            let c = egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                            painter.line_segment([egui::pos2(base.x + x1, base.y + y1), egui::pos2(base.x + x2, base.y + y2)], egui::Stroke::new(*lw, c));
                        }
                        CanvasCmd::Rect { x, y, w, h, color } => {
                            let c = egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                            painter.rect_filled(egui::Rect::from_min_size(egui::pos2(base.x + x, base.y + y), egui::vec2(*w, *h)), 0.0, c);
                        }
                        CanvasCmd::Circle { cx, cy, r, color } => {
                            let c = egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                            painter.circle_filled(egui::pos2(base.x + cx, base.y + cy), *r, c);
                        }
                    }
            }
        }
    }
}

// 闭包数据：函数体 + 捕获的外部变量共享单元（引用捕获）
#[derive(Debug, Clone)]
struct ClosureData {
    func: Func,
    /// 捕获的外部变量：名字 -> 共享单元（与定义处外层变量共享存储）
    captured: FastMap<u32, Rc<RefCell<Value>>>,
}

// ---------- 闭包自由变量收集 ----------
fn collect_expr_refs(e: &Expr, refs: &mut Vec<u32>) {
    match e {
        Expr::Ident(name, _) => refs.push(*name),
        Expr::Number(..) | Expr::Float(..) | Expr::String(..) | Expr::RawString(..) | Expr::Bool(..) => {}
        Expr::Array(items, _) => for it in items { collect_expr_refs(it, refs); },
        Expr::Index(a, i, _) => { collect_expr_refs(a, refs); collect_expr_refs(i, refs); }
        Expr::BinOp(l, _, r, _) => { collect_expr_refs(l, refs); collect_expr_refs(r, refs); }
        Expr::Call(n, args, _) => { refs.push(interner().get(&n)); for a in args { collect_expr_refs(a, refs); } }
        Expr::Assign(n, v, _) => { refs.push(*n); collect_expr_refs(v, refs); }
        Expr::IndexAssign(a, i, v, _) => { collect_expr_refs(a, refs); collect_expr_refs(i, refs); collect_expr_refs(v, refs); }
        Expr::AddrOf(i, _) | Expr::RawAddr(i, _) | Expr::Deref(i, _) | Expr::Neg(i, _) | Expr::Not(i, _) => collect_expr_refs(i, refs),
        Expr::DerefAssign(p, v, _) => { collect_expr_refs(p, refs); collect_expr_refs(v, refs); }
        Expr::Member(o, _, _) => collect_expr_refs(o, refs),
        Expr::MethodCall(o, _, args, _) => { collect_expr_refs(o, refs); for a in args { collect_expr_refs(a, refs); } }
        Expr::MemberAssign(o, _, v, _) => { collect_expr_refs(o, refs); collect_expr_refs(v, refs); }
        Expr::PrivateMember(o, _, _) => collect_expr_refs(o, refs),
        Expr::PrivateMemberAssign(o, _, v, _) => { collect_expr_refs(o, refs); collect_expr_refs(v, refs); }
        Expr::Lambda(_, _) => {} // 嵌套闭包自行捕获，不深入
        Expr::Dict(items, _) => for (_, v) in items { collect_expr_refs(v, refs); },
        Expr::Match(s, arms, _) => {
            collect_expr_refs(s, refs);
            for (_, b) in arms { collect_expr_refs(b, refs); }
        }
        Expr::Range(l, r, _) => { collect_expr_refs(l, refs); collect_expr_refs(r, refs); }
        Expr::Int64(_, _) => {}
        Expr::Cast(e, _, _) => collect_expr_refs(e, refs),
    }
}

fn collect_stmt_refs(s: &Stmt, refs: &mut Vec<u32>, locals: &mut Vec<u32>) {
    match s {
        Stmt::Let(n, e, _) => { locals.push(*n); collect_expr_refs(e, refs); }
        Stmt::Const(n, e, _) => { locals.push(*n); collect_expr_refs(e, refs); }
        Stmt::FnDef(_, params, body, _, _) => { for p in params { locals.push(interner().get(p)); } for b in body { collect_stmt_refs(b, refs, locals); } }
        Stmt::If(c, t, elifs, e2, _) => {
            collect_expr_refs(c, refs);
            for x in t { collect_stmt_refs(x, refs, locals); }
            for (ec, b) in elifs { collect_expr_refs(ec, refs); for x in b { collect_stmt_refs(x, refs, locals); } }
            for x in e2 { collect_stmt_refs(x, refs, locals); }
        }
        Stmt::While(c, b, _) => { collect_expr_refs(c, refs); for x in b { collect_stmt_refs(x, refs, locals); } }
        Stmt::For(v, s2, e, b, _) => { locals.push(*v); collect_expr_refs(s2, refs); collect_expr_refs(e, refs); for x in b { collect_stmt_refs(x, refs, locals); } }
        Stmt::Print(es, _) => for e in es { collect_expr_refs(e, refs); },
        Stmt::Expr(e, _) => collect_expr_refs(e, refs),
        Stmt::Return(Some(e), _) => collect_expr_refs(e, refs),
        Stmt::Return(None, _) | Stmt::Break(_) | Stmt::Continue(_) => {}
        Stmt::Yield(e, _) => collect_expr_refs(e, refs),
        Stmt::Throw(e, _) => collect_expr_refs(e, refs),
        Stmt::Try { body, catch_var, handler, .. } => {
            for x in body { collect_stmt_refs(x, refs, locals); }
            if let Some(cv) = catch_var { locals.push(interner().get(cv)); }
            for x in handler { collect_stmt_refs(x, refs, locals); }
        }
        Stmt::ImportItem { .. } => {}
        Stmt::UnsafeBlock(b, _) => for x in b { collect_stmt_refs(x, refs, locals); },
        Stmt::Class(..) => {}
    }
}

/// 收集闭包体中的自由变量（排除参数、内部 let 绑定、self）
fn collect_closure_free(func: &Func) -> Vec<u32> {
    let mut refs: Vec<u32> = Vec::new();
    let mut locals: Vec<u32> = func.params.iter().map(|p| interner().get(p)).collect();
    for s in &func.body { collect_stmt_refs(s, &mut refs, &mut locals); }
    let mut out = Vec::new();
    for r in refs {
        if r == interner().get("self") { continue; }
        if locals.contains(&r) { continue; }
        if !out.contains(&r) { out.push(r); }
    }
    out
}

impl Value {
    fn is_truthy(&self) -> bool {
        match self {
            Value::Int(n) => *n != 0,
            Value::Int64(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Bool(b) => *b,
            Value::Array(arr) => !arr.is_empty(),
            Value::ArenaPtr(ptr) => !ptr.is_null(),
            // 新增 RawPtr
            Value::RawPtr(ptr) => !ptr.is_null(),
            Value::ArrayElementPtr(_, _) => true,
            Value::Range(_, _) => true,
            Value::Class(_) => true,
            Value::Instance(_) => true,
            Value::Closure(_) => true,
            Value::Dict(map) => !map.is_empty(),
            Value::Generator(_) => true,
        }
    }

    // 判断是否为Arena安全指针
    fn is_arena_ptr(&self) -> bool {
        matches!(self, Value::ArenaPtr(_))
    }

    // 新增：判断是否为Raw裸指针
    fn is_raw_ptr(&self) -> bool {
        matches!(self, Value::RawPtr(_))
    }
}

// ==============================
// 变量环境 Env
// ==============================
#[derive(Debug, Clone)]
enum MemKind {
    Arena,   // Arena内存池（安全指针可用）
    Heap,    // 标准堆（字符串，禁止取地址）
    RawHeap, // 裸指针指向的外部/堆内存（仅Raw指针可用）
}

#[derive(Debug, Clone)]
enum VarStorage {
    /// 普通直存：热路径，无 RefCell 开销
    Plain(Value),
    /// 闭包捕获的共享单元：引用捕获双向同步
    Shared(Rc<RefCell<Value>>),
}

impl VarEntry {
    /// 提升为共享单元（若已是 Shared 直接返回其 Rc）
    fn promote(&mut self) -> Rc<RefCell<Value>> {
        match &mut self.value {
            VarStorage::Shared(u) => u.clone(),
            VarStorage::Plain(v) => {
                let u = Rc::new(RefCell::new(std::mem::replace(v, Value::Int(0))));
                self.value = VarStorage::Shared(u.clone());
                u
            }
        }
    }
}

/// 符号表：把变量名字符串唯一映射为 u32 整数 ID。
/// 采用两级结构：string->id 查重，id->string 反查（错误提示用）。
struct Interner {
    map: RwLock<FastMap<String, u32>>,
    vec: RwLock<Vec<String>>,
}

impl Interner {
    fn get(&self, s: &str) -> u32 {
        let mut map = self.map.write().unwrap();
        if let Some(&id) = map.get(s) {
            return id;
        }
        let mut vec = self.vec.write().unwrap();
        let id = vec.len() as u32;
        vec.push(s.to_string());
        map.insert(s.to_string(), id);
        id
    }
    fn lookup(&self, id: u32) -> String {
        let vec = self.vec.read().unwrap();
        vec[id as usize].clone()
    }
}

static GLOBAL_INTERNER: OnceLock<Interner> = OnceLock::new();
fn interner() -> &'static Interner {
    GLOBAL_INTERNER.get_or_init(|| {
        let i = Interner {
            map: RwLock::new(FastMap::new()),
            vec: RwLock::new(Vec::new()),
        };
        // 预插入常见内置名，稳定其 ID
        for n in ["print", "self", "super", "len", "args", "str_to_num", "true", "false"] {
            i.get(n);
        }
        i
    })
}

#[derive(Clone)]
struct VarEntry {
    value: VarStorage,
    is_const: bool,
    mem_kind: MemKind,
}

#[derive(Default, Clone)]
struct Env {
    vars: FastMap<u32, Box<VarEntry>>,
    outer: Option<Box<Env>>,
}

impl Env {
    fn new() -> Self {
        Self {
            vars: FastMap::new(),
            outer: None,
        }
    }

    fn nested(outer: Env) -> Self {
        Self {
            vars: FastMap::new(),
            outer: Some(Box::new(outer)),
        }
    }

    // ================= 整数 ID 版本（变量查找热路径，免字符串哈希） =================

    fn get_id(&self, id: u32) -> Value {
        if let Some(entry) = self.vars.get(&id) {
            match &entry.value {
                VarStorage::Plain(v) => v.clone(),
                VarStorage::Shared(u) => u.borrow().clone(),
            }
        } else if let Some(outer) = &self.outer {
            outer.get_id(id)
        } else {
            panic!("变量未定义: {}", interner().lookup(id));
        }
    }

    /// 热路径借引用读取：普通 Plain 直存变量一次查表拿到 &Value（不克隆、不二次查表）。
    /// Shared(RefCell) 与跨层变量返回 None，由调用方回退到 get_id。
    fn get_ref_id(&self, id: u32) -> Option<&Value> {
        if let Some(entry) = self.vars.get(&id) {
            return match &entry.value {
                VarStorage::Plain(v) => Some(v),
                VarStorage::Shared(_) => None,
            };
        }
        if let Some(outer) = &self.outer {
            outer.get_ref_id(id)
        } else {
            None
        }
    }

    fn get_mut_id(&mut self, id: u32) -> Option<&mut VarEntry> {
        if let Some(entry) = self.vars.get_mut(&id) {
            return Some(&mut **entry);
        }
        if let Some(outer) = &mut self.outer {
            outer.get_mut_id(id)
        } else {
            None
        }
    }

    fn is_const_id(&self, id: u32) -> bool {
        if let Some(entry) = self.vars.get(&id) {
            return entry.is_const;
        }
        if let Some(outer) = &self.outer {
            outer.is_const_id(id)
        } else {
            false
        }
    }

    fn set_id(&mut self, id: u32, val: Value) {
        if let Some(entry) = self.vars.get_mut(&id) {
            if entry.is_const {
                panic!("错误：常量 {} 不可被修改", interner().lookup(id));
            }
            entry.mem_kind = match &val {
                Value::String(_) => MemKind::Heap,
                _ => MemKind::Arena,
            };
            match &mut entry.value {
                VarStorage::Plain(v) => { *v = val; }
                VarStorage::Shared(u) => { *u.borrow_mut() = val; }
            }
            return;
        }
        if let Some(outer) = &mut self.outer {
            outer.set_id(id, val);
        } else {
            panic!("变量未定义: {}", interner().lookup(id));
        }
    }

    fn contains_id(&self, id: u32) -> bool {
        if self.vars.contains_key(&id) {
            return true;
        }
        if let Some(outer) = &self.outer {
            outer.contains_id(id)
        } else {
            false
        }
    }

    fn get_mem_kind_id(&self, id: u32) -> MemKind {
        if let Some(entry) = self.vars.get(&id) {
            return entry.mem_kind.clone();
        }
        if let Some(outer) = &self.outer {
            outer.get_mem_kind_id(id)
        } else {
            panic!("变量未定义: {}", interner().lookup(id));
        }
    }

    fn define_var_id(&mut self, id: u32, val: Value) {
        let mem_kind = match &val {
            Value::String(_) => MemKind::Heap,
            _ => MemKind::Arena,
        };
        self.vars.insert(id, Box::new(VarEntry {
            value: VarStorage::Plain(val),
            is_const: false,
            mem_kind,
        }));
    }

    fn define_const_id(&mut self, id: u32, val: Value) {
        let mem_kind = match &val {
            Value::String(_) => MemKind::Heap,
            _ => MemKind::Arena,
        };
        self.vars.insert(id, Box::new(VarEntry {
            value: VarStorage::Plain(val),
            is_const: true,
            mem_kind,
        }));
    }

    fn remove_id(&mut self, id: u32) {
        self.vars.remove(&id);
    }

    fn define_unit_id(&mut self, id: u32, unit: Rc<RefCell<Value>>) {
        let mem_kind = match &*unit.borrow() {
            Value::String(_) => MemKind::Heap,
            _ => MemKind::Arena,
        };
        self.vars.insert(id, Box::new(VarEntry {
            value: VarStorage::Shared(unit),
            is_const: false,
            mem_kind,
        }));
    }

    fn get_unit_id(&self, id: u32) -> Option<Rc<RefCell<Value>>> {
        if let Some(entry) = self.vars.get(&id) {
            return match &entry.value {
                VarStorage::Shared(u) => Some(u.clone()),
                VarStorage::Plain(_) => None,
            };
        }
        if let Some(outer) = &self.outer {
            outer.get_unit_id(id)
        } else {
            None
        }
    }

    fn promote_to_shared_id(&mut self, id: u32) -> Option<Rc<RefCell<Value>>> {
        if let Some(entry) = self.vars.get_mut(&id) {
            return Some(entry.promote());
        }
        if let Some(outer) = &mut self.outer {
            outer.promote_to_shared_id(id)
        } else {
            None
        }
    }

    // ================= 字符串版本（兼容，内部转 ID） =================

    fn get(&self, name: &str) -> Value { self.get_id(interner().get(name)) }
    fn get_mut(&mut self, name: &str) -> Option<&mut VarEntry> { self.get_mut_id(interner().get(name)) }
    fn is_const(&self, name: &str) -> bool { self.is_const_id(interner().get(name)) }
    fn set(&mut self, name: &str, val: Value) { self.set_id(interner().get(name), val) }
    fn contains(&self, name: &str) -> bool { self.contains_id(interner().get(name)) }
    fn get_mem_kind(&self, name: &str) -> MemKind { self.get_mem_kind_id(interner().get(name)) }
    fn define_var(&mut self, name: String, val: Value) { self.define_var_id(interner().get(&name), val) }
    fn define_const(&mut self, name: String, val: Value) { self.define_const_id(interner().get(&name), val) }
    fn remove(&mut self, name: &str) { self.remove_id(interner().get(name)) }
    fn define_unit(&mut self, name: String, unit: Rc<RefCell<Value>>) { self.define_unit_id(interner().get(&name), unit) }
    fn get_unit(&self, name: &str) -> Option<Rc<RefCell<Value>>> { self.get_unit_id(interner().get(name)) }
    fn promote_to_shared(&mut self, name: &str) -> Option<Rc<RefCell<Value>>> { self.promote_to_shared_id(interner().get(name)) }
}

// ==============================
// 函数 & 解释器主体
// ==============================
// 保存一个模块导出项：顶层let / const 的AST节点
#[derive(Debug, Clone)]
enum ModuleExportItem {
    Let(String, Expr, u32),
    Const(String, Expr, u32),
    Fn(String, Vec<String>, Vec<Stmt>, u32),
    /// 模块内定义的类（class）：名字, 定义, 行号
    Class(String, ClassDef, u32),
}

/// key: 完整模块路径字符串，例如 "lib::math"
#[derive(Default, Clone)]
struct ModuleCacheEntry {
    exports: Vec<ModuleExportItem>,
    // 是否已完成初始化执行（防止重复执行顶层代码）
    initialized: bool,
}

type ModuleCache = HashMap<String, ModuleCacheEntry>;

type ModuleNativeFuncs = HashMap<String, HashMap<String, fn(Vec<Value>) -> Value>>;

/// 取语句的源码行号（所有 Stmt 变体最后一个 u32 字段即行号）
fn stmt_line(s: &Stmt) -> u32 {
    match s {
        Stmt::Let(_, _, l) | Stmt::Const(_, _, l) | Stmt::FnDef(_, _, _, _, l)
        | Stmt::Print(_, l) | Stmt::Expr(_, l) | Stmt::Return(_, l)
        | Stmt::Break(l) | Stmt::Continue(l) | Stmt::UnsafeBlock(_, l) | Stmt::Yield(_, l) => *l,
        Stmt::If(_, _, _, _, l) | Stmt::While(_, _, l) | Stmt::For(_, _, _, _, l) => *l,
        Stmt::ImportItem { line, .. } => *line,
        Stmt::Class(_, l) => *l,
        Stmt::Throw(_, l) => *l,
        Stmt::Try { line, .. } => *line,
    }
}

struct Interpreter {
    env: Env,
    funcs: HashMap<String, Func>,
    classes: HashMap<String, ClassDef>,
    /// 隐藏类字段布局缓存：类指针 -> FieldLayout
    field_cache: HashMap<usize, Rc<FieldLayout>>,
    /// GUI：当前窗口收集的控件（仅 feature "gui" 编译）
    #[cfg(feature = "gui")]
    gui_controls: Vec<GuiControl>,
    /// GUI：是否正在窗口事件循环中（仅 feature "gui" 编译）
    #[cfg(feature = "gui")]
    gui_active: bool,
    /// GUI：布局容器栈（gui_row/gui_col 嵌套收集时压栈）
    #[cfg(feature = "gui")]
    gui_stack: Vec<*mut Vec<GuiControl>>,
    /// GUI：canvas 绘制命令栈（gui_canvas 嵌套收集时压栈）
    #[cfg(feature = "gui")]
    gui_canvas_stack: Vec<*mut Vec<CanvasCmd>>,
    /// GUI：当前 eframe 上下文（quit 在回调里关闭窗口用）
    #[cfg(feature = "gui")]
    gui_ctx: Option<*mut egui::Context>,
    /// GUI：gui_run 异步执行的 mpsc 接收端队列（主线程轮询写回共享变量）
    #[cfg(feature = "gui")]
    gui_async_rx: Vec<std::sync::mpsc::Receiver<(String, String)>>,
    /// GUI：窗口图标文件路径（gui_icon(path) 设置；未设置时默认用内嵌 logo.ico）
    #[cfg(feature = "gui")]
    gui_icon: Option<String>,
    lib_funcs: HashMap<String, fn(Vec<Value>) -> Value>,
    module_native: ModuleNativeFuncs,
    mod_alias: HashMap<String, String>,
    module_cache: ModuleCache,
    /// 当前正在执行的方法所属类（用于私有字段访问权限判断）
    current_class: Option<String>,
    /// 当前正在执行的方法/构造器的 self 实例（供 super() 调用父类构造）
    current_self: Option<Value>,
    /// 函数/方法调用栈（名称, 定义行）：错误时附加根源标签
    ctx_stack: Vec<(String, u32)>,
    pub file: String,
    pub line: u32,
    /// 命令行传给脚本的参数（xlang run script.x arg1 arg2 ...）
    script_args: Vec<String>,
    /// 最近一次 throw 的值（配合 ThrowMarker panic 跨栈传播）
    pending_throw: Option<Value>,
}

impl Interpreter {
    /// 统一错误出口：若正处于函数/方法调用栈内，附加函数定义处作为根源标签
    fn emit_script_error(&self, line: u32, msg: &str) -> ! {
        if let Some((cname, cline)) = self.ctx_stack.last() {
            script_panic_at_multi(
                &self.file, line, 1, msg,
                vec![(*cline, 1, format!("位于 {cname}（定义于此处）"))],
            );
        } else {
            script_panic_at(&self.file, line, 1, msg);
        }
    }

    /// 以解释器当前(文件,行)抛出脚本错误，列号回退到1（解释期不明列号）
    fn panic_here(&self, msg: &str) -> ! {
        self.emit_script_error(self.line, msg)
    }

    /// 调用内置函数：捕获 panic，把参数/执行错误转成带当前语句行号的 ariadne 错误
    fn call_builtin(&self, lib_fn: fn(Vec<Value>) -> Value, args: Vec<Value>) -> Value {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lib_fn(args))) {
            Ok(v) => v,
            Err(payload) => {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "内置函数内部错误".to_string()
                };
                self.emit_script_error(self.line, &msg)
            }
        }
    }

    /// 以解释器当前文件 + 指定行号抛出脚本错误（列号回退1）
    fn panic_at(&self, line: u32, _col: u32, msg: &str) -> ! {
        self.emit_script_error(line, msg)
    }

    fn new() -> Self {
        let mut interp = Self {
            env: Env::new(),
            funcs: HashMap::new(),
            classes: HashMap::new(),
            field_cache: HashMap::new(),
            #[cfg(feature = "gui")]
            gui_controls: Vec::new(),
            #[cfg(feature = "gui")]
            gui_active: false,
            #[cfg(feature = "gui")]
            gui_stack: Vec::new(),
            #[cfg(feature = "gui")]
            gui_canvas_stack: Vec::new(),
            #[cfg(feature = "gui")]
            gui_ctx: None,
            #[cfg(feature = "gui")]
            gui_async_rx: Vec::new(),
            #[cfg(feature = "gui")]
            gui_icon: None,
            lib_funcs: HashMap::new(),
            module_native: HashMap::new(),
            mod_alias: HashMap::new(),
            module_cache: ModuleCache::default(),
            current_class: None,
            current_self: None,
            ctx_stack: Vec::new(),
            file: String::new(),
            line: 0,
            script_args: Vec::new(),
            pending_throw: None,
        };

        // substr(s, start, len) 字符串截取
        interp.lib_funcs.insert("print".to_string(), |args| {
            let parts: Vec<String> = args.iter().map(fmt_value).collect();
            println!("{}", parts.join(" "));
            Value::Int(0)
        });
        // keys(dict) 返回 dict 全部键（字符串数组），用于遍历/序列化
        interp.lib_funcs.insert("keys".to_string(), |args| {
            if args.len() != 1 { panic!("keys(dict) 需要一个参数"); }
            match &args[0] {
                Value::Dict(map) => {
                    let mut ks = Vec::new();
                    for k in map.keys() { ks.push(Value::String(PoolStr::new(k.as_str()))); }
                    Value::Array(Rc::new(ks))
                }
                _ => panic!("keys 参数必须是 dict"),
            }
        });
        // type(v) 返回值的类型名
        interp.lib_funcs.insert("type".to_string(), |args| {
            if args.len() != 1 { panic!("type(v) 需要一个参数"); }
            let s = match &args[0] {
                Value::Int(_) => "int",
                Value::Float(_) => "float",
                Value::String(_) => "str",
                Value::Bool(_) => "bool",
                Value::Array(_) => "array",
                Value::Dict(_) => "dict",
                Value::Closure(_) | Value::Class(_) => "func",
                Value::Instance(_) => "instance",
                Value::Generator(_) => "generator",
                Value::Range(_, _) => "range",
                _ => "ptr",
            };
            Value::String(PoolStr::new(s))
        });
        interp.lib_funcs.insert("substr".to_string(), |args| {
            if args.len() != 3 {
                panic!("substr(s, start, len) 需要3个参数：字符串，起始下标(0开始)，截取长度");
            }
            let s = match &args[0] {
                Value::String(ps) => ps.as_str(),
                _ => panic!("substr第一个参数必须是字符串"),
            };
            let start = match args[1] {
                Value::Int(i) => i as usize,
                _ => panic!("substr第二个参数start必须为整数"),
            };
            let len_arg = match args[2] {
                Value::Int(i) => i,
                _ => panic!("substr第三个参数len必须为整数"),
            };
            if len_arg <= 0 {
                return Value::String(PoolStr::new(""));
            }
            let len = len_arg as usize;
            let chars: Vec<char> = s.chars().collect();
            let st = start.min(chars.len());
            let end = st.saturating_add(len).min(chars.len());
            let slice: String = chars[st..end].iter().collect();
            Value::String(PoolStr::new(&slice))
        });

        // index_of(s, needle) 查找子串，找不到返回‑1
        interp.lib_funcs.insert("index_of".to_string(), |args| {
            if args.len() != 2 {
                panic!("index_of(s, needle) 需要2个参数：源字符串，待查找子串/字符");
            }
            let s = match &args[0] {
                Value::String(ps) => ps.as_str(),
                _ => panic!("index_of第一个参数必须是字符串"),
            };
            let needle = match &args[1] {
                Value::String(ps) => ps.as_str(),
                Value::Int(_) | Value::Float(_) => panic!("index_of第二个参数必须字符串(字符/子串)"),
                _ => panic!("index_of第二个参数必须字符串"),
            };
            let pos = s.find(needle);
            match pos {
                Some(p) => Value::Int(p as i32),
                None => Value::Int(-1),
            }
        });

        // split_str(s, sep): 按分隔符拆分为字符串数组（字符安全，支持中文）
        interp.lib_funcs.insert("split_str".to_string(), |args| {
            if args.len() != 2 {
                panic!("split_str(s, sep) 需要2个参数：源字符串，分隔符");
            }
            let s = match &args[0] {
                Value::String(ps) => ps.as_str().to_string(),
                _ => panic!("split_str第一个参数必须是字符串"),
            };
            let sep = match &args[1] {
                Value::String(ps) => ps.as_str().to_string(),
                _ => panic!("split_str第二个参数必须是字符串(分隔符)"),
            };
            let parts: Vec<&str> = s.split(sep.as_str()).collect();
            let mut arr = Vec::new();
            for p in parts {
                arr.push(Value::String(PoolStr::new(p)));
            }
            Value::Array(Rc::new(arr))
        });

        // trim_str(s): 去除字符串首尾空白
        interp.lib_funcs.insert("trim_str".to_string(), |args| {
            if args.len() != 1 { panic!("trim_str 需要1个参数：字符串"); }
            let s = match &args[0] {
                Value::String(ps) => ps.as_str().trim(),
                _ => panic!("trim_str参数必须为字符串"),
            };
            Value::String(PoolStr::new(s))
        });

        // starts_with(s, prefix): 前缀判断
        interp.lib_funcs.insert("starts_with".to_string(), |args| {
            if args.len() != 2 { panic!("starts_with(s, prefix) 需要2个参数"); }
            let s = match &args[0] { Value::String(ps) => ps.as_str(), _ => panic!("starts_with第一个参数必须字符串") };
            let p = match &args[1] { Value::String(ps) => ps.as_str(), _ => panic!("starts_with第二个参数必须字符串") };
            Value::Bool(s.starts_with(p))
        });

        // ends_with(s, suffix): 后缀判断
        interp.lib_funcs.insert("ends_with".to_string(), |args| {
            if args.len() != 2 { panic!("ends_with(s, suffix) 需要2个参数"); }
            let s = match &args[0] { Value::String(ps) => ps.as_str(), _ => panic!("ends_with第一个参数必须字符串") };
            let p = match &args[1] { Value::String(ps) => ps.as_str(), _ => panic!("ends_with第二个参数必须字符串") };
            Value::Bool(s.ends_with(p))
        });

        // http_server(host:str, port:int) 阻塞启动静态http服务
        interp.lib_funcs.insert("http_server".to_string(), |args| {
            if args.len() != 2 {
                panic!("http_server(host, port) 需要2个参数：host字符串，端口整数");
            }
            let host = match &args[0] {
                Value::String(s) => s.as_str().to_string(),
                _ => panic!("http_server第一个参数必须是host字符串"),
            };
            let port = match args[1] {
                Value::Int(p) => p as u16,
                _ => panic!("http_server第二个参数必须是端口整数"),
            };

            let listener = match TcpListener::bind((host.as_str(), port)) {
                Ok(l) => l,
                Err(e) => panic!("HTTP服务绑定 {}:{} 失败: {}", host, port, e),
            };
            println!("XLang HTTP server running on http://{}:{}", host, port);
            println!("Serving directory: {}", std::env::current_dir().unwrap().display());
            println!("Press Ctrl+C to stop.");

            for stream in listener.incoming() {
                match stream {
                    Ok(mut stream) => {
                        // 只读取请求头：读到空行 \r\n\r\n（或上限8KB）即返回，
                        // 不再等客户端断开连接，否则 keep-alive 浏览体会卡住。
                        let mut buf = Vec::with_capacity(2048);
                        let mut byte = [0u8; 1];
                        loop {
                            match stream.read(&mut byte) {
                                Ok(0) => break,
                                Ok(_) => {
                                    buf.push(byte[0]);
                                    if buf.ends_with(b"\r\n\r\n") || buf.len() > 8192 {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let req = String::from_utf8_lossy(&buf);
                        let lines: Vec<&str> = req.lines().collect();
                        if lines.is_empty() { continue; }
                        let req_line = lines[0];
                        let parts: Vec<&str> = req_line.split_whitespace().collect();
                        if parts.len() < 2 { continue; }
                        let method = parts[0];
                        let raw_path = parts[1];

                        // url路径转本地Windows文件路径
                        let url_path = percent_encoding::percent_decode_str(raw_path).decode_utf8_lossy();
                        let mut local_path = PathBuf::from(".");
                        for seg in url_path.split('/') {
                            if seg.is_empty() || seg == "." { continue; }
                            if seg == ".." {
                                local_path.pop();
                                continue;
                            }
                            local_path.push(seg);
                        }

                        let resp: String;
                        let mut content_type;
                        if local_path.is_dir() {
                            // 目录：优先返回 index.html（类似 python -m http.server）
                            let index_file = ["index.html", "index.htm", "index.HTML", "index.HTM"]
                                .iter()
                                .map(|n| local_path.join(n))
                                .find(|p| p.is_file());
                            if let Some(ifile) = index_file {
                                match std::fs::read(&ifile) {
                                    Ok(data) => {
                                        let mime = mime_from_path(&ifile);
                                        let header = format!(
                                            "HTTP/1.1 200 OK\r\nContent-Type:{}\r\nContent-Length:{}\r\n\r\n",
                                            mime,
                                            data.len()
                                        );
                                        let _ = stream.write_all(header.as_bytes());
                                        let _ = stream.write_all(&data);
                                        continue;
                                    }
                                    Err(_) => {
                                        resp = "HTTP/1.1 500 Internal Server Error\r\nContent-Type:text/plain;charset=utf-8\r\nContent-Length:21\r\n\r\n500 Internal Server Error".to_string();
                                        let _ = stream.write_all(resp.as_bytes());
                                        continue;
                                    }
                                }
                            }
                            // 目录里没有 index 文件，回退为生成html文件列表
                            let entries = match std::fs::read_dir(&local_path) {
                                Ok(e) => e,
                                Err(_) => {
                                    resp = "HTTP/1.1 403 Forbidden\r\nContent-Type:text/plain;charset=utf-8\r\nContent-Length:13\r\n\r\n403 Forbidden".to_string();
                                    let _ = stream.write_all(resp.as_bytes());
                                    continue;
                                }
                            };
                            let mut html = String::new();
                            html.push_str("HTTP/1.1 200 OK\r\nContent-Type:text/html;charset=utf-8\r\n");
                            html.push_str("\r\n<!DOCTYPE html><html><body>");
                            html.push_str("<h1>Directory listing</h1><ul>");
                            if raw_path != "/" {
                                html.push_str(r#"<li><a href="../">../</a></li>"#);
                            }
                            for entry in entries.flatten() {
                                let name_os = entry.file_name();
                                let name = name_os.to_string_lossy();
                                let href = format!("{}/{}", raw_path.trim_end_matches('/'), name);
                                let slash = if entry.file_type().unwrap().is_dir() { "/" } else { "" };
                                html.push_str(&format!(r#"<li><a href="{}{}">{}{}</a></li>"#, href, slash, name, slash));
                            }
                            html.push_str("</ul></body></html>");
                            resp = html;
                        } else if local_path.is_file() {
                            // 文件：读取文件 + MIME
                            content_type = mime_from_path(&local_path);
                            match std::fs::read(&local_path) {
                                Ok(data) => {
                                    let header = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type:{}\r\nContent-Length:{}\r\n\r\n",
                                        content_type,
                                        data.len()
                                    );
                                    let _ = stream.write_all(header.as_bytes());
                                    let _ = stream.write_all(&data);
                                    continue;
                                }
                                Err(_) => {
                                    resp = "HTTP/1.1 404 Not Found\r\nContent-Type:text/plain;charset=utf-8\r\nContent-Length:12\r\n\r\n404 Not Found".to_string();
                                }
                            }
                        } else {
                            resp = "HTTP/1.1 404 Not Found\r\nContent-Type:text/plain;charset=utf-8\r\nContent-Length:12\r\n\r\n404 Not Found".to_string();
                        }
                        let _ = stream.write_all(resp.as_bytes());
                    }
                    Err(e) => eprintln!("accept error:{}", e),
                }
            }
            Value::Int(0)
        });

        interp.lib_funcs.insert("version".to_string(), |args| {
            if !args.is_empty() {
                panic!("version() 不接收任何参数");
            }
            Value::String(PoolStr::new("0.3.0"))
        });

        interp.lib_funcs.insert("to_str".to_string(), |args| {
            if args.len() != 1 {
                panic!("to_str() 仅接收1个参数");
            }
            let s = match &args[0] {
                Value::Int(n) => n.to_string(),
                Value::Int64(n) => n.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::String(st) => st.as_str().to_string(),
                Value::Array(a) => {
                    let parts: Vec<String> = a.iter().map(value_to_str_for_join).collect();
                    format!("[{}]", parts.join(", "))
                }
                Value::ArenaPtr(_) => "(ArenaPtr)".to_string(),
                Value::RawPtr(_) => "(RawPtr)".to_string(),
                Value::ArrayElementPtr(n, i) => format!("&{}[{}]", n, i),
                Value::Range(lo, hi) => format!("{}..{}", fmt_range_num(*lo), fmt_range_num(*hi)),
                Value::Class(c) => format!("(class {})", c.name),
                Value::Instance(i) => format!("(instance of {})", i.borrow().class.name),
        Value::Closure(_) => "(closure)".to_string(),
        Value::Generator(_) => "(generator)".to_string(),
        Value::Dict(_) => fmt_value(&args[0]),
            };
            Value::String(PoolStr::new(&s))
        });
        // ========== 计算器/数值工具 内置函数注册 ==========
        // str_to_num(s)：将数字字符串解析为 Int（无小数点）或 Float（含小数点/科学计数），解析失败报错
        interp.lib_funcs.insert("str_to_num".to_string(), |args| {
            if args.len() != 1 {
                panic!("str_to_num() 仅接收1个参数");
            }
            let s = match &args[0] {
                Value::String(st) => st.as_str(),
                _ => panic!("str_to_num 参数必须为字符串"),
            };
            let t = s.trim();
            if t.contains('.') || t.contains('e') || t.contains('E') {
                match t.parse::<f64>() {
                    Ok(f) => Value::Float(f),
                    Err(_) => panic!("str_to_num 无法解析数字: {}", s),
                }
            } else {
                match t.parse::<i32>() {
                    Ok(n) => Value::Int(n),
                    Err(_) => match t.parse::<f64>() {
                        Ok(f) => Value::Float(f),
                        Err(_) => panic!("str_to_num 无法解析数字: {}", s),
                    },
                }
            }
        });

        // to_float(x)：Int/Float/数字字符串 → Float
        interp.lib_funcs.insert("to_float".to_string(), |args| {
            if args.len() != 1 {
                panic!("to_float() 仅接收1个参数");
            }
            match &args[0] {
                Value::Int(n) => Value::Float(*n as f64),
                Value::Int64(n) => Value::Float(*n as f64),
                Value::Float(f) => Value::Float(*f),
                Value::String(st) => {
                    let t = st.as_str().trim();
                    match t.parse::<f64>() {
                        Ok(f) => Value::Float(f),
                        Err(_) => panic!("to_float 无法解析数字: {}", st.as_str()),
                    }
                }
                _ => panic!("to_float 参数必须是数字或数字字符串"),
            }
        });

        // to_int(x)：Int/Float/数字字符串 → Int（小数直接截断）
        interp.lib_funcs.insert("to_int".to_string(), |args| {
            if args.len() != 1 {
                panic!("to_int() 仅接收1个参数");
            }
            match &args[0] {
                Value::Int(n) => Value::Int(*n),
                Value::Int64(n) => Value::Int(*n as i32),
                Value::Float(f) => Value::Int(*f as i32),
                Value::String(st) => {
                    let t = st.as_str().trim();
                    match t.parse::<f64>() {
                        Ok(f) => Value::Int(f as i32),
                        Err(_) => panic!("to_int 无法解析数字: {}", st.as_str()),
                    }
                }
                _ => panic!("to_int 参数必须是数字或数字字符串"),
            }
        });

        // ========== 多内存池 内置函数注册 ==========
        // 全局默认池：无参
        interp.lib_funcs.insert("mem_global_init".to_string(), |_args| {
            mem_global_init();
            Value::Int(0)
        });
        interp.lib_funcs.insert("mem_rollback".to_string(), |_args| {
            mem_rollback();
            Value::Int(0)
        });
        interp.lib_funcs.insert("mem_free".to_string(), |_args| {
            mem_free();
            Value::Int(0)
        });
        interp.lib_funcs.insert("mem_global_destroy".to_string(), |_args| {
            mem_global_destroy();
            Value::Int(0)
        });

        // mem_alloc(池名)：创建并初始化命名内存池
        interp.lib_funcs.insert("mem_alloc".to_string(), |args| {
            if args.len() != 1 {
                panic!("mem_alloc 必须传入内存池名称（字符串/标识符）");
            }
            let name = match &args[0] {
                Value::String(s) => s.as_str().to_string(),
                Value::Int(_) | Value::Float(_) => panic!("内存池名称不能为数字"),
                _ => panic!("mem_alloc 参数必须是字符串"),
            };
            mem_create_arena(&name);
            Value::Int(0)
        });

        // mem_rollback(池名)：给指定池打回滚点
        interp.lib_funcs.insert("mem_rollback".to_string(), |args| {
            match args.len() {
                0 => {
                    mem_rollback();
                }
                1 => {
                    let name = match &args[0] {
                        Value::String(s) => s.as_str(),
                        _ => panic!("mem_rollback 参数必须是内存池名字符串"),
                    };
                    mem_rollback_named(name);
                }
                _ => panic!("mem_rollback 最多接收 1 个名称参数"),
            }
            Value::Int(0)
        });

        // mem_free(池名)：回滚指定池
        interp.lib_funcs.insert("mem_free".to_string(), |args| {
            match args.len() {
                0 => {
                    mem_free();
                }
                1 => {
                    let name = match &args[0] {
                        Value::String(s) => s.as_str(),
                        _ => panic!("mem_free 参数必须是内存池名字符串"),
                    };
                    mem_free_named(name);
                }
                _ => panic!("mem_free 最多接收 1 个名称参数"),
            }
            Value::Int(0)
        });

        // mem_global_destroy(池名)：销毁指定池
        interp.lib_funcs.insert("mem_global_destroy".to_string(), |args| {
            match args.len() {
                0 => {
                    mem_global_destroy();
                }
                1 => {
                    let name = match &args[0] {
                        Value::String(s) => s.as_str(),
                        _ => panic!("mem_global_destroy 参数必须是内存池名字符串"),
                    };
                    mem_destroy_named(name);
                }
                _ => panic!("mem_global_destroy 最多接收 1 个名称参数"),
            }
            Value::Int(0)
        });

        // ========= 文件操作原生函数注册 =========
        // read_file(path): 读取整个文件，返回完整字符串
        interp.lib_funcs.insert("read_file".to_string(), |args| {
            if args.len() != 1 {
                panic!("read_file 仅接收文件路径1个参数");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("read_file 参数必须为字符串路径"),
            };
            let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
                panic!("读取文件失败 {}: {}", path, e)
            });
            Value::String(PoolStr::new(&content))
        });

        // write_file(path, text): 覆盖重写整个文件
        interp.lib_funcs.insert("write_file".to_string(), |args| {
            if args.len() != 2 {
                panic!("write_file 需要两个参数：路径，写入文本");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("第一个参数必须是文件路径字符串"),
            };
            // 统一全部分支返回 String，消除类型冲突
            let text: String = match &args[1] {
                Value::String(s) => s.as_str().to_string(),
                val => {
                    let st = match val {
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Array(arr) => {
                            let mut buf = String::from("[");
                            for (i, v) in arr.iter().enumerate() {
                                if i > 0 { buf.push_str(", "); }
                                match v {
                                    Value::Int(x) => buf.push_str(&x.to_string()),
                                    Value::Float(x) => buf.push_str(&x.to_string()),
                                    Value::String(s) => buf.push_str(s.as_str()),
                                    Value::Bool(x) => buf.push_str(&x.to_string()),
                                    _ => buf.push_str("(ptr)"),
                                }
                            }
                            buf.push(']');
                            buf
                        }
                        _ => String::from(""),
                    };
                    st
                }
            };
            std::fs::write(path, text).unwrap_or_else(|e| {
                panic!("写入文件失败 {}: {}", path, e)
            });
            Value::Int(0)
        });

        // append_file(path, text): 追加写入文件末尾（插入内容）
        interp.lib_funcs.insert("append_file".to_string(), |args| {
            if args.len() != 2 {
                panic!("append_file 需要两个参数：路径，追加文本");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("第一个参数必须是文件路径字符串"),
            };
            let text = match &args[1] {
                Value::String(s) => s.as_str().to_string(),
                val => {
                    let st = match val {
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => String::new(),
                    };
                    st
                }
            };
            use std::fs::OpenOptions;
            let mut f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap_or_else(|e| panic!("追加文件失败 {}: {}", path, e));
            std::io::Write::write_all(&mut f, text.as_bytes()).unwrap();
            Value::Int(0)
        });

        // read_lines(path): 返回数组，每一项为文件一行文本（自动去除换行符）
        interp.lib_funcs.insert("read_lines".to_string(), |args| {
            if args.len() != 1 {
                panic!("read_lines 仅接收文件路径1个参数");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("参数必须为字符串路径"),
            };
            let content = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("读取失败: {}", e));
            let mut arr = Vec::new();
            for line in content.lines() {
                arr.push(Value::String(PoolStr::new(line)));
            }
            Value::Array(Rc::new(arr))
        });

        // file_line_count(path): 计算文件总行数，返回整数
        interp.lib_funcs.insert("file_line_count".to_string(), |args| {
            if args.len() != 1 {
                panic!("file_line_count 仅接收文件路径");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("参数必须为字符串路径"),
            };
            let content = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("读取失败: {}", e));
            let cnt = content.lines().count() as i32;
            Value::Int(cnt)
        });

        // read_line_at(path, line_num): 读取指定行号文本，行号从1开始
        interp.lib_funcs.insert("read_line_at".to_string(), |args| {
            if args.len() != 2 {
                panic!("read_line_at(路径, 行号)");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("第一个参数为文件路径字符串"),
            };
            let line_idx = match args[1] {
                Value::Int(n) => {
                    if n < 1 {
                        panic!("行号必须 >= 1");
                    }
                    (n - 1) as usize
                }
                _ => panic!("第二个参数为整数行号"),
            };
            let content = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("读取失败: {}", e));
            let mut lines = content.lines();
            let target = lines.nth(line_idx).unwrap_or_else(|| panic!("行号超出文件总行数"));
            Value::String(PoolStr::new(target))
        });

        // file_truncate(path, size): 截断文件到指定字节长度
        interp.lib_funcs.insert("file_truncate".to_string(), |args| {
            if args.len() != 2 {
                panic!("file_truncate(路径, 字节大小)");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("路径为字符串"),
            };
            let size = match args[1] {
                Value::Int(n) => n as u64,
                Value::Float(f) => f as u64,
                _ => panic!("第二个参数为数字字节长度"),
            };
            let f = std::fs::OpenOptions::new()
                .write(true)
                .create(true) // 不存在则新建空文件
                .open(path)
                .unwrap_or_else(|e| panic!("打开文件截断失败: {}", e));
            f.set_len(size).unwrap();
            Value::Int(0)
        });

        // 覆写指定行 edit_line(文件路径, 行号, 新文本)
        interp.lib_funcs.insert("edit_line".to_string(), |args| {
            if args.len() != 3 {
                panic!("edit_line 需要3个参数：路径，行号，替换文本");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("第一个参数必须是文件路径字符串"),
            };
            let line_idx = match args[1] {
                Value::Int(n) => {
                    if n < 1 {
                        panic!("edit_line 行号必须 >= 1");
                    }
                    (n - 1) as usize
                }
                _ => panic!("第二个参数为整数行号"),
            };
            let new_txt = match &args[2] {
                Value::String(s) => s.as_str().to_string(),
                val => {
                    let st = match val {
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => String::new(),
                    };
                    st
                }
            };

            let mut lines: Vec<String> = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("读取文件失败 {}: {}", path, e))
                .lines()
                .map(|s| s.to_string())
                .collect();

            if line_idx >= lines.len() {
                panic!("edit_line：行号超出文件总行数");
            }
            lines[line_idx] = new_txt;

            // 写回文件
            let content = lines.join("\n");
            std::fs::write(path, content)
                .unwrap_or_else(|e| panic!("写入文件失败 {}: {}", path, e));
            Value::Int(0)
        });

        // 在指定行后插入一行 append_line(文件路径, 行号, 插入文本)
        // line_num = 0 → 文件最开头插入
        interp.lib_funcs.insert("append_line".to_string(), |args| {
            if args.len() != 3 {
                panic!("append_line 需要3个参数：路径，行号，插入文本");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("第一个参数必须是文件路径字符串"),
            };
            let target_line = match args[1] {
                Value::Int(n) => n as usize,
                _ => panic!("第二个参数为整数行号"),
            };
            let insert_txt = match &args[2] {
                Value::String(s) => s.as_str().to_string(),
                val => {
                    let st = match val {
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => String::new(),
                    };
                    st
                }
            };

            let mut lines: Vec<String> = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("读取文件失败 {}: {}", path, e))
                .lines()
                .map(|s| s.to_string())
                .collect();

            if target_line == 0 {
                // 头部插入
                lines.insert(0, insert_txt);
            } else {
                let pos = target_line;
                if pos > lines.len() {
                    panic!("append_line：插入行位置超出文件总行数");
                }
                lines.insert(pos, insert_txt);
            }

            let content = lines.join("\n");
            std::fs::write(path, content)
                .unwrap_or_else(|e| panic!("写入文件失败 {}: {}", path, e));
            Value::Int(0)
        });

        // append_at_line(文件路径, 行号, 行末尾追加文本)
        interp.lib_funcs.insert("append_at_line".to_string(), |args| {
            if args.len() != 3 {
                panic!("append_at_line 需要3个参数：路径，行号，行末尾追加内容");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("第一个参数必须是文件路径字符串"),
            };
            let line_idx = match args[1] {
                Value::Int(n) => {
                    if n < 1 {
                        panic!("append_at_line 行号必须 >= 1");
                    }
                    (n - 1) as usize
                }
                _ => panic!("第二个参数为整数行号"),
            };
            let suffix_txt = match &args[2] {
                Value::String(s) => s.as_str().to_string(),
                val => {
                    let st = match val {
                        Value::Int(n) => n.to_string(),
                        Value::Float(f) => f.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => String::new(),
                    };
                    st
                }
            };

            // 读取全部行
            let mut lines: Vec<String> = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("读取文件失败 {}: {}", path, e))
                .lines()
                .map(|s| s.to_string())
                .collect();

            if line_idx >= lines.len() {
                panic!("append_at_line：行号超出文件总行数");
            }
            // 在原有行末尾拼接内容
            lines[line_idx] = format!("{}{}", lines[line_idx], suffix_txt);

            // 写回文件
            let content = lines.join("\n");
            std::fs::write(path, content)
                .unwrap_or_else(|e| panic!("写入文件失败 {}: {}", path, e));
            Value::Int(0)
        });

        // file_exists(path): 判断文件是否存在，返回true/false
        interp.lib_funcs.insert("file_exists".to_string(), |args| {
            if args.len() != 1 {
                panic!("file_exists(文件路径)");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("参数必须字符串路径"),
            };
            let exist = std::path::Path::new(path).exists();
            Value::Bool(exist)
        });

        // delete_file(path): 删除文件
        interp.lib_funcs.insert("delete_file".to_string(), |args| {
            if args.len() != 1 {
                panic!("delete_file(文件路径)");
            }
            let path = match &args[0] {
                Value::String(s) => s.as_str(),
                _ => panic!("路径为字符串"),
            };
            std::fs::remove_file(path).unwrap_or_else(|e| panic!("删除文件失败: {}", e));
            Value::Int(0)
        });

        register_os_funcs(&mut interp);
        interp
    }

    fn set_script_args(&mut self, args: Vec<String>) {
        self.script_args = args;
    }

    fn run(&mut self, program: &[Stmt]) {
        // 注入命令行参数为顶层 args 数组（xlang run script.x a b c）；始终注入（可为空数组）
        let arr: Vec<Value> = self.script_args.iter().map(|s| Value::String(PoolStr::new(s.as_str()))).collect();
        self.env.define_var("args".to_string(), Value::Array(Rc::new(arr)));
        for stmt in program {
            let _ = self.exec_stmt(stmt);
        }
    }

    /// 通用函数调用：切换局部环境、绑定形参、执行函数体、恢复父环境
    fn call_func(&mut self, func: Func, evaluated_args: Vec<Value>) -> Value {
        // 生成器函数：不立即执行，返回可迭代的生成器对象（惰性）
        if body_has_yield(&func.body) {
            return self.make_generator(func, evaluated_args);
        }
        self.ctx_stack.push((func.name.clone(), func.line));
        // 取出当前环境作为父环境（不克隆，直接所有权转移）
        let parent_env = std::mem::replace(&mut self.env, Env::new());
        // 创建嵌套局部环境，父环境直接作为外层作用域
        let mut local_env = Env::nested(parent_env);
        // 绑定形参到局部环境
        for (param, arg) in func.params.iter().zip(evaluated_args) {
            local_env.define_var(param.clone(), arg);
        }
        // 切换到局部环境执行
        self.env = local_env;

        let (result, _ctrl) = self.exec_stmts_return_last(&func.body);
        let result = match &func.ret_ty {
            Some(rt) => self.cast_value(result, rt, func.line),
            None => result,
        };

        // 执行完毕，取回父环境（子函数对外部变量的修改已保留在父环境中）
        let local_env = std::mem::replace(&mut self.env, Env::new());
        self.env = *local_env.outer.unwrap();
        self.ctx_stack.pop();

        result
    }
    /// 生成器调用：不执行函数体，构造含参数环境与剩余语句的生成器对象
    fn make_generator(&mut self, func: Func, evaluated_args: Vec<Value>) -> Value {
        // 首版仅支持平铺 yield（控制流内 yield 暂不支持）
        if body_has_nested_yield(&func.body) {
            if let Some((ln, cl)) = find_nested_yield_pos(&func.body) {
                script_panic_at_multi(
                    &self.file, self.line, 1,
                    "暂不支持在 if/while/for 内使用 yield（当前仅支持平铺 yield）",
                    vec![(ln, cl, "此处的 yield 位于控制流（if/while/for）内".to_string())],
                );
            }
            self.panic_here("暂不支持在 if/while/for 内使用 yield（当前仅支持平铺 yield）");
        }
        let caller_env = std::mem::replace(&mut self.env, Env::new());
        let mut local_env = Env::nested(caller_env.clone());
        for (param, arg) in func.params.iter().zip(evaluated_args) {
            local_env.define_var(param.clone(), arg);
        }
        let gd = GeneratorData { body: func.body, env: local_env, done: false };
        self.env = caller_env;
        Value::Generator(Rc::new(RefCell::new(gd)))
    }
    /// 推进生成器到下一个 yield（None = 生成器结束）
    fn generator_next_val(&mut self, g: Rc<RefCell<GeneratorData>>) -> Option<Value> {
        let caller_env = std::mem::replace(&mut self.env, Env::new());
        let mut g = g.borrow_mut();
        if g.done {
            self.env = caller_env;
            return None;
        }
        self.env = std::mem::replace(&mut g.env, Env::new());
        let mut produced = Value::Int(0);
        let mut rest = Vec::new();
        let has = self.exec_stmts_until_yield(&g.body, &mut produced, &mut rest);
        if !has { g.done = true; }
        g.body = rest;
        g.env = std::mem::replace(&mut self.env, Env::new());
        self.env = caller_env;
        if has { Some(produced) } else { None }
    }
    /// 顺序执行语句，遇第一个 yield 产出值并保存剩余语句（平铺）
    fn exec_stmts_until_yield(&mut self, stmts: &[Stmt], produced: &mut Value, rest: &mut Vec<Stmt>) -> bool {
        for (i, s) in stmts.iter().enumerate() {
            if let Stmt::Yield(expr, _) = s {
                *produced = self.eval_expr(expr);
                rest.clear();
                rest.extend_from_slice(&stmts[i + 1..]);
                return true;
            }
            self.line = stmt_line(s);
            let _ = self.exec_stmt(s);
        }
        false
    }
    /// 闭包调用：注入捕获的共享单元 + 绑定参数，执行闭包体，恢复环境
    fn call_closure(&mut self, closure: &Rc<ClosureData>, args: Vec<Value>, call_line: u32) -> Value {
        if closure.func.params.len() != args.len() {
            self.panic_at(call_line, 1, &format!("闭包参数数量不匹配：期望 {} 个，实际 {} 个", closure.func.params.len(), args.len()));
        }
        let parent_env = std::mem::replace(&mut self.env, Env::new());
        let mut local_env = Env::nested(parent_env);
        // 注入捕获变量（共享单元，引用捕获：闭包与外层变量共享同一存储）
        for (name, unit) in &closure.captured {
            local_env.define_unit_id(*name, unit.clone());
        }
        // 绑定形参
        for (param, arg) in closure.func.params.iter().zip(args) {
            local_env.define_var(param.clone(), arg);
        }
        self.env = local_env;
        let (result, _ctrl) = self.exec_stmts_return_last(&closure.func.body);
        let result = match &closure.func.ret_ty {
            Some(rt) => self.cast_value(result, rt, closure.func.line),
            None => result,
        };
        let local_env = std::mem::replace(&mut self.env, Env::new());
        self.env = *local_env.outer.unwrap();
        result
    }

    /// 数组高阶函数：map / filter / reduce / join / sort（需闭包回调，走解释器方法）
    fn call_array_hof(&mut self, name: &str, args: Vec<Value>, ln: u32) -> Value {
        match name {
            "map" => {
                if args.len() != 2 { self.panic_at(ln, 1, "map(arr, fn) 需要2个参数：数组、函数"); }
                let arr = match &args[0] { Value::Array(a) => a.clone(), _ => self.panic_at(ln, 1, "map 第一个参数必须是数组") };
                let cb = match &args[1] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "map 第二个参数必须是函数") };
                let mut out = Vec::with_capacity(arr.len());
                for el in arr.iter() {
                    let r = self.call_closure(&cb, vec![el.clone()], ln);
                    out.push(r);
                }
                Value::Array(Rc::new(out))
            }
            "filter" => {
                if args.len() != 2 { self.panic_at(ln, 1, "filter(arr, fn) 需要2个参数：数组、函数"); }
                let arr = match &args[0] { Value::Array(a) => a.clone(), _ => self.panic_at(ln, 1, "filter 第一个参数必须是数组") };
                let cb = match &args[1] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "filter 第二个参数必须是函数") };
                let mut out = Vec::new();
                for el in arr.iter() {
                    let r = self.call_closure(&cb, vec![el.clone()], ln);
                    if r.is_truthy() { out.push(el.clone()); }
                }
                Value::Array(Rc::new(out))
            }
            "reduce" => {
                if args.len() != 3 { self.panic_at(ln, 1, "reduce(arr, fn, init) 需要3个参数：数组、函数、初始值"); }
                let arr = match &args[0] { Value::Array(a) => a.clone(), _ => self.panic_at(ln, 1, "reduce 第一个参数必须是数组") };
                let cb = match &args[1] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "reduce 第二个参数必须是函数") };
                let mut acc = args[2].clone();
                for el in arr.iter() {
                    acc = self.call_closure(&cb, vec![acc, el.clone()], ln);
                }
                acc
            }
            "join" => {
                if args.len() != 2 { self.panic_at(ln, 1, "join(arr, sep) 需要2个参数：数组、分隔符"); }
                let arr = match &args[0] { Value::Array(a) => a.clone(), _ => self.panic_at(ln, 1, "join 第一个参数必须是数组") };
                let sep = match &args[1] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "join 第二个参数必须是字符串分隔符") };
                let mut parts = Vec::with_capacity(arr.len());
                for el in arr.iter() {
                    parts.push(value_to_str_for_join(el));
                }
                Value::String(PoolStr::new(&parts.join(&sep)))
            }
            "sort" => {
                if args.len() != 1 { self.panic_at(ln, 1, "sort(arr) 需要1个参数：数组"); }
                let mut arr = match &args[0] { Value::Array(a) => a.clone(), _ => self.panic_at(ln, 1, "sort 参数必须是数组") };
                Rc::make_mut(&mut arr).sort_by(|a, b| value_cmp(a, b));
                Value::Array(arr)
            }
            _ => self.panic_at(ln, 1, "未知数组高阶函数"),
        }
    }

    /// GUI 分派：gui_window / gui_button / gui_text（仅 feature "gui" 编译）
    #[cfg(feature = "gui")]
    fn call_gui(&mut self, name: &str, args: Vec<Value>, ln: u32) -> Value {
        match name {
            "gui_window" => self.gui_window(args, ln),
            "gui_icon" => self.gui_icon(args, ln),
            "gui_button" => self.gui_button(args, ln),
            "gui_text" => self.gui_text(args, ln),
            "gui_heading" => self.gui_heading(args, ln),
            "gui_separator" => self.gui_separator(args, ln),
            "gui_spacer" => self.gui_spacer(args, ln),
            "gui_input" => self.gui_input(args, ln),
            "gui_input_var" => self.gui_input_var(args, ln),
            "gui_textarea" => self.gui_textarea(args, ln),
            "gui_output" => self.gui_output(args, ln),
            "gui_terminal" => self.gui_terminal(args, ln),
            "gui_checkbox" => self.gui_checkbox(args, ln),
            "gui_slider" => self.gui_slider(args, ln),
            "gui_row" => self.gui_row(args, ln),
            "gui_col" => self.gui_col(args, ln),
            "gui_topbar" => self.gui_topbar(args, ln),
            "gui_topbar_v" => self.gui_topbar_v(args, ln),
            "gui_sidebar" => self.gui_sidebar(args, ln),
            "gui_bottombar" => self.gui_bottombar(args, ln),
            "gui_run" => self.gui_run(args, ln),
            "gui_color_edit" => self.gui_color_edit(args, ln),
            "gui_combo" => self.gui_combo(args, ln),
            "gui_radio" => self.gui_radio(args, ln),
            "gui_table" => self.gui_table(args, ln),
            "gui_multiselect" => self.gui_multiselect(args, ln),
            "gui_tabs" => self.gui_tabs(args, ln),
            "gui_canvas" => self.gui_canvas(args, ln),
            "canvas_line" => self.canvas_line(args, ln),
            "canvas_rect" => self.canvas_rect(args, ln),
            "canvas_circle" => self.canvas_circle(args, ln),
            _ => self.panic_at(ln, 1, "未知 GUI 函数"),
        }
    }

    /// gui_window(title, builder)：执行 builder 收集控件，然后进入 eframe 事件循环直到窗口关闭
    #[cfg(feature = "gui")]
    /// 解码 ico 文件为 egui 图标数据（支持 PNG 压缩条目，如 PIL 生成的 ico）
    #[cfg(feature = "gui")]
    fn decode_ico(&self, bytes: &[u8]) -> Option<egui::IconData> {
        if bytes.len() < 6 { return None; }
        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        // 第一遍：选尺寸最大的 PNG 条目（目录宽高为 0 表示 256）
        let mut best: Option<(usize, usize, usize)> = None; // (wh, off, size)
        for i in 0..count {
            let e = 6 + i * 16;
            if e + 16 > bytes.len() { break; }
            let wd = if bytes[e] == 0 { 256 } else { bytes[e] as usize };
            let ht = if bytes[e+1] == 0 { 256 } else { bytes[e+1] as usize };
            let wh = wd.max(ht);
            let size = u32::from_le_bytes([bytes[e+8], bytes[e+9], bytes[e+10], bytes[e+11]]) as usize;
            let off = u32::from_le_bytes([bytes[e+12], bytes[e+13], bytes[e+14], bytes[e+15]]) as usize;
            if off + size <= bytes.len() && size > 8 && bytes[off] == 0x89 && bytes[off+1] == 0x50 {
                if best.is_none() || wh > best.unwrap().0 { best = Some((wh, off, size)); }
            }
        }
        let (_, off, size) = best?;
        let data = &bytes[off..off+size];
        let decoder = png::Decoder::new(std::io::Cursor::new(data));
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).ok()?;
        let (w, h) = (info.width as usize, info.height as usize);
        let ch = info.color_type.samples() as usize;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for px in 0..(w * h) {
            let i = px * ch;
            let (r, g, b) = match info.color_type {
                png::ColorType::Rgba => (buf[i], buf[i+1], buf[i+2]),
                png::ColorType::Rgb => (buf[i], buf[i+1], buf[i+2]),
                png::ColorType::Grayscale => (buf[i], buf[i], buf[i]),
                _ => continue,
            };
            let a = if info.color_type == png::ColorType::Rgba { buf[i+3] } else { 255 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        if w * h * 4 != rgba.len() { return None; }
        Some(egui::IconData { rgba, width: w as u32, height: h as u32 })
    }

    /// gui_icon(path)：设置窗口图标为指定 .ico 文件（覆盖默认 logo.ico）
    #[cfg(feature = "gui")]
    fn gui_icon(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_icon(path) 需要1个参数"); }
        let path = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_icon 参数必须是图标文件路径字符串") };
        self.gui_icon = Some(path);
        Value::Int(0)
    }

    #[cfg(feature = "gui")]
    fn gui_window(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if self.gui_active {
            self.panic_at(ln, 1, "不支持嵌套窗口（gui_window 内不能再开窗口）");
        }
        if args.len() != 2 && args.len() != 4 { self.panic_at(ln, 1, "gui_window(title, builder) 或 gui_window(title, w, h, builder)"); }
        let title = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_window 第一个参数必须是标题字符串") };
        let builder = match &args[args.len()-1] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_window 最后一个参数必须是函数") };
        // 执行 builder 闭包，收集控件声明
        self.gui_controls.clear();
        self.gui_active = true;
        let _r = self.call_closure(&builder, Vec::new(), ln);
        self.gui_active = false;
        // 进入 eframe 事件循环（此处才初始化窗口/GL，符合运行时惰性）
        let controls = std::mem::take(&mut self.gui_controls);
        let interp_ptr: *mut Interpreter = self;
        let app = GuiApp { controls, interp: interp_ptr, classify_dirty: true, top: None, side: None, bottom: None, central: Vec::new() };
        let mut options = eframe::NativeOptions::default();
        // 可选指定窗口大小：gui_window(title, w, h, builder)
        if args.len() == 4 {
            let nv = |v: &Value| -> f32 { match v { Value::Int(n) => *n as f32, Value::Float(f) => *f as f32, _ => 0.0 } };
            options.viewport.inner_size = Some(egui::Vec2::new(nv(&args[1]), nv(&args[2])));
        }
        // 窗口图标：优先 gui_icon(path) 指定的 ico，否则默认用内嵌 logo.ico
        let icon = if let Some(p) = self.gui_icon.take() {
            std::fs::read(&p).ok().and_then(|b| self.decode_ico(&b))
        } else {
            Some(egui::IconData { rgba: logo_icon::RGBA.to_vec(), width: logo_icon::W, height: logo_icon::H })
        };
        options.viewport.icon = icon.map(std::sync::Arc::new);
        let _ = eframe::run_native(&title, options, Box::new(move |cc| {
            setup_cjk_fonts(&cc.egui_ctx);
            Ok(Box::new(app))
        }));
        self.gui_ctx = None;
        Value::Int(0)
    }

    /// gui_button(label, onclick)：声明一个按钮（在 builder 闭包内调用）
    #[cfg(feature = "gui")]
    fn gui_button(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 2 && args.len() != 3 { self.panic_at(ln, 1, "gui_button(label, onclick[, btn_height]) 需要2或3个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_button 第一个参数必须是标签字符串") };
        let onclick = args[1].clone();
        let size = if args.len() == 3 {
            match &args[2] { Value::Int(n) => *n as f32, Value::Float(f) => *f as f32, _ => self.panic_at(ln, 1, "gui_button 第三个参数必须是数字（按钮高度像素）") }
        } else { 0.0 };
        self.push_control(GuiControl::Button { label, onclick, size });
        Value::Int(0)
    }

    /// gui_text(text)：显示一段文本（在 builder 闭包内调用）
    #[cfg(feature = "gui")]
    fn gui_text(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_text(text) 需要1个参数"); }
        let text = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_text 参数必须是字符串") };
        self.push_control(GuiControl::Text { text });
        Value::Int(0)
    }

    /// gui_heading(text)：大标题
    #[cfg(feature = "gui")]
    fn gui_heading(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_heading(text) 需要1个参数"); }
        let text = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_heading 参数必须是字符串") };
        self.push_control(GuiControl::Heading { text });
        Value::Int(0)
    }

    /// gui_separator()：分隔线
    #[cfg(feature = "gui")]
    fn gui_separator(&mut self, _args: Vec<Value>, _ln: u32) -> Value {
        self.push_control(GuiControl::Separator);
        Value::Int(0)
    }

    /// gui_spacer()：垂直间距
    #[cfg(feature = "gui")]
    fn gui_spacer(&mut self, _args: Vec<Value>, _ln: u32) -> Value {
        self.push_control(GuiControl::Spacer);
        Value::Int(0)
    }

    /// gui_input(label, initial, fn(new_value){...})：单行文本输入框，值变化时回调
    #[cfg(feature = "gui")]
    fn gui_input(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 3 { self.panic_at(ln, 1, "gui_input(label, initial, onchange) 需要3个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_input 第一个参数必须是标签字符串") };
        let value = match &args[1] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_input 第二个参数必须是初始字符串") };
        let onchange = args[2].clone();
        self.push_control(GuiControl::Input { label, value, onchange });
        Value::Int(0)
    }

    /// gui_input_var(label, varname)：单行输入框，绑定到 XLang 共享变量（编辑写回变量，外部改变量→输入框实时刷新/清空）
    #[cfg(feature = "gui")]
    fn gui_input_var(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 2 { self.panic_at(ln, 1, "gui_input_var(label, varname) 需要2个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_input_var 第一个参数必须是标签字符串") };
        let varname = match &args[1] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_input_var 第二个参数必须是绑定变量名字符串") };
        let id = interner().get(&varname);
        if let Some(unit) = self.env.promote_to_shared_id(id) {
            self.push_control(GuiControl::InputVar { label, unit });
            Value::Int(0)
        } else {
            self.panic_at(ln, 1, &format!("gui_input_var 绑定变量未定义: {}", varname))
        }
    }

    /// gui_textarea(label, varname)：多行文本编辑器，绑定到 XLang 共享变量（编辑器与变量双向同步）
    /// 变量被提升为共享单元：编辑写回变量，外部改变量编辑器实时刷新（IDE 联动核心）
    #[cfg(feature = "gui")]
    fn gui_textarea(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 2 { self.panic_at(ln, 1, "gui_textarea(label, varname) 需要2个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_textarea 第一个参数必须是标签字符串") };
        let varname = match &args[1] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_textarea 第二个参数必须是绑定变量名字符串") };
        let id = interner().get(&varname);
        if let Some(unit) = self.env.promote_to_shared_id(id) {
            self.push_control(GuiControl::TextArea { label, unit });
            Value::Int(0)
        } else {
            self.panic_at(ln, 1, &format!("gui_textarea 绑定变量未定义: {}", varname))
        }
    }

    /// gui_output(varname)：只读实时显示共享变量的字符串值（IDE 状态栏/运行输出用）
    #[cfg(feature = "gui")]
    fn gui_output(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_output(varname) 需要1个参数"); }
        let varname = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_output 参数必须是绑定变量名字符串") };
        let id = interner().get(&varname);
        if let Some(unit) = self.env.promote_to_shared_id(id) {
            self.push_control(GuiControl::Output { unit, last: None });
            Value::Int(0)
        } else {
            self.panic_at(ln, 1, &format!("gui_output 绑定变量未定义: {}", varname))
        }
    }

    /// gui_terminal(varname, fn(input){...})：可编辑终端（输出+输入一体）
    /// 绑定共享变量显示输出；可自由输入，Ctrl+Enter 提交输入给回调并清空
    #[cfg(feature = "gui")]
    fn gui_terminal(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 2 { self.panic_at(ln, 1, "gui_terminal(varname, onsubmit) 需要2个参数"); }
        let varname = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_terminal 第一个参数必须是绑定变量名字符串") };
        let onsubmit = args[1].clone();
        let id = interner().get(&varname);
        if let Some(unit) = self.env.promote_to_shared_id(id) {
            self.push_control(GuiControl::Terminal { unit, onsubmit });
            Value::Int(0)
        } else {
            self.panic_at(ln, 1, &format!("gui_terminal 绑定变量未定义: {}", varname))
        }
    }

    /// gui_checkbox(label, initial_bool, fn(bool){...})：勾选框，状态变化时回调
    #[cfg(feature = "gui")]
    fn gui_checkbox(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 3 { self.panic_at(ln, 1, "gui_checkbox(label, initial, onchange) 需要3个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_checkbox 第一个参数必须是标签字符串") };
        let value = match &args[1] { Value::Bool(b) => *b, _ => self.panic_at(ln, 1, "gui_checkbox 第二个参数必须是布尔值") };
        let onchange = args[2].clone();
        self.push_control(GuiControl::Checkbox { label, value, onchange });
        Value::Int(0)
    }

    /// gui_slider(label, min, max, initial, fn(f64){...})：滑杆，数值变化时回调
    #[cfg(feature = "gui")]
    fn gui_slider(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 5 { self.panic_at(ln, 1, "gui_slider(label, min, max, initial, onchange) 需要5个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_slider 第一个参数必须是标签字符串") };
        let num = |v: &Value| -> f64 { match v { Value::Int(n) => *n as f64, Value::Float(f) => *f, _ => 0.0 } };
        let min = num(&args[1]);
        let max = num(&args[2]);
        let value = num(&args[3]);
        let onchange = args[4].clone();
        self.push_control(GuiControl::Slider { label, min, max, value, onchange });
        Value::Int(0)
    }

    /// 收集控件到当前容器（gui_row/gui_col 布局嵌套时入栈到子容器）
    #[cfg(feature = "gui")]
    fn push_control(&mut self, c: GuiControl) {
        if let Some(&top) = self.gui_stack.last() {
            unsafe { (*top).push(c); }
        } else {
            self.gui_controls.push(c);
        }
    }

    /// gui_row(fn(){...})：水平布局容器（子控件横向排列）
    #[cfg(feature = "gui")]
    fn gui_row(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_row(fn) 需要1个函数参数"); }
        let cb = match &args[0] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_row 参数必须是函数") };
        let mut child: Vec<GuiControl> = Vec::new();
        let ptr = &mut child as *mut Vec<GuiControl>;
        self.gui_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_stack.pop();
        self.push_control(GuiControl::Row(child));
        Value::Int(0)
    }

    /// gui_col(fn(){...})：垂直布局容器（子控件纵向排列）
    #[cfg(feature = "gui")]
    fn gui_col(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_col(fn) 需要1个函数参数"); }
        let cb = match &args[0] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_col 参数必须是函数") };
        let mut child: Vec<GuiControl> = Vec::new();
        let ptr = &mut child as *mut Vec<GuiControl>;
        self.gui_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_stack.pop();
        self.push_control(GuiControl::Col(child));
        Value::Int(0)
    }

    /// gui_topbar(fn(){...})：顶部工具条/菜单栏（渲染到窗口顶部面板）
    #[cfg(feature = "gui")]
    fn gui_topbar(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_topbar(fn) 需要1个函数参数"); }
        let cb = match &args[0] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_topbar 参数必须是函数") };
        let mut child: Vec<GuiControl> = Vec::new();
        let ptr = &mut child as *mut Vec<GuiControl>;
        self.gui_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_stack.pop();
        self.push_control(GuiControl::TopBar { children: child, horizontal: true });
        Value::Int(0)
    }

    /// gui_topbar_v(fn(){...})：顶部工具条（竖着平铺：子控件各占一行、多行堆叠）
    #[cfg(feature = "gui")]
    fn gui_topbar_v(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_topbar_v(fn) 需要1个函数参数"); }
        let cb = match &args[0] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_topbar_v 参数必须是函数") };
        let mut child: Vec<GuiControl> = Vec::new();
        let ptr = &mut child as *mut Vec<GuiControl>;
        self.gui_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_stack.pop();
        self.push_control(GuiControl::TopBar { children: child, horizontal: false });
        Value::Int(0)
    }

    /// gui_sidebar(fn(){...})：左侧面板（文件列表等）
    #[cfg(feature = "gui")]
    fn gui_sidebar(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_sidebar(fn) 需要1个函数参数"); }
        let cb = match &args[0] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_sidebar 参数必须是函数") };
        let mut child: Vec<GuiControl> = Vec::new();
        let ptr = &mut child as *mut Vec<GuiControl>;
        self.gui_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_stack.pop();
        self.push_control(GuiControl::SideBar(child));
        Value::Int(0)
    }

    /// gui_bottombar(fn(){...})：底部面板（输出区等，固定可见，不被编辑区挤走）
    #[cfg(feature = "gui")]
    fn gui_bottombar(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 1 { self.panic_at(ln, 1, "gui_bottombar(fn) 需要1个函数参数"); }
        let cb = match &args[0] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_bottombar 参数必须是函数") };
        let mut child: Vec<GuiControl> = Vec::new();
        let ptr = &mut child as *mut Vec<GuiControl>;
        self.gui_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_stack.pop();
        self.push_control(GuiControl::BottomBar(child));
        Value::Int(0)
    }

    /// gui_run(cmd, input, varname)：异步执行命令（stdin 喂 input），完成后把 stdout
    /// 写回共享变量 varname。不阻塞 GUI 主线程——可运行 input()/带 GUI 窗口等阻塞程序，
    /// IDE 不会未响应。
    #[cfg(feature = "gui")]
    fn gui_run(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 3 { self.panic_at(ln, 1, "gui_run(cmd, input, varname) 需要3个参数"); }
        let cmd = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_run 第一个参数必须是命令字符串") };
        let input = match &args[1] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_run 第二个参数必须是 stdin 输入字符串") };
        let varname = match &args[2] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_run 第三个参数必须是绑定变量名字符串") };
        let (tx, rx) = std::sync::mpsc::channel::<(String, String)>();
        std::thread::spawn(move || {
            use std::io::Write as _;
            let result = if cfg!(windows) {
                std::process::Command::new("cmd").args(["/C", &cmd])
                    .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()
            } else {
                std::process::Command::new("sh").args(["-c", &cmd])
                    .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()
            };
            let out = match result {
                Ok(mut c) => {
                    if let Some(mut si) = c.stdin.take() { let _ = si.write_all(input.as_bytes()); }
                    match c.wait_with_output() {
                        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
                        Err(e) => format!("等待失败: {}", e),
                    }
                }
                Err(e) => format!("执行失败: {}", e),
            };
            let _ = tx.send((varname, out));
        });
        self.gui_async_rx.push(rx);
        Value::Int(0)
    }

    /// gui_color_edit(label, [r,g,b,a], fn(color_arr){...})：颜色选择器，回调传 [r,g,b,a] 数组
    #[cfg(feature = "gui")]
    fn gui_color_edit(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 3 { self.panic_at(ln, 1, "gui_color_edit(label, [r,g,b,a], onchange) 需要3个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_color_edit 第一个参数必须是标签字符串") };
        let color = match &args[1] {
            Value::Array(a) => {
                let mut c = [255u8; 4];
                for (i, v) in a.iter().take(4).enumerate() {
                    c[i] = match v { Value::Int(n) => (*n).clamp(0, 255) as u8, _ => 0 };
                }
                c
            }
            _ => self.panic_at(ln, 1, "gui_color_edit 第二个参数必须是 [r,g,b,a] 数组"),
        };
        let onchange = args[2].clone();
        self.push_control(GuiControl::ColorEdit { label, color, onchange });
        Value::Int(0)
    }

    /// gui_combo(label, options, initial_index, fn(sel_str){...})：下拉选择，回调传选中字符串
    #[cfg(feature = "gui")]
    fn gui_combo(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 4 { self.panic_at(ln, 1, "gui_combo(label, options, initial_index, onchange) 需要4个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_combo 第一个参数必须是标签字符串") };
        let options = match &args[1] {
            Value::Array(a) => a.iter().map(|v| match v { Value::String(s) => s.as_str().to_string(), _ => String::new() }).collect(),
            _ => self.panic_at(ln, 1, "gui_combo 第二个参数必须是字符串数组"),
        };
        let selected = match &args[2] { Value::Int(n) => (*n).max(0) as usize, _ => 0 };
        let onchange = args[3].clone();
        self.push_control(GuiControl::Combo { label, options, selected, onchange });
        Value::Int(0)
    }

    /// gui_radio(label, options, initial_index, fn(sel_str){...})：单选按钮组，回调传选中字符串
    #[cfg(feature = "gui")]
    fn gui_radio(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 4 { self.panic_at(ln, 1, "gui_radio(label, options, initial_index, onchange) 需要4个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_radio 第一个参数必须是标签字符串") };
        let options = match &args[1] {
            Value::Array(a) => a.iter().map(|v| match v { Value::String(s) => s.as_str().to_string(), _ => String::new() }).collect(),
            _ => self.panic_at(ln, 1, "gui_radio 第二个参数必须是字符串数组"),
        };
        let selected = match &args[2] { Value::Int(n) => (*n).max(0) as usize, _ => 0 };
        let onchange = args[3].clone();
        self.push_control(GuiControl::Radio { label, options, selected, onchange });
        Value::Int(0)
    }

    /// gui_table(headers, rows[, onrow])：表格，rows 是二维数组
    #[cfg(feature = "gui")]
    fn gui_table(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() < 2 || args.len() > 3 { self.panic_at(ln, 1, "gui_table(headers, rows[, onrow]) 需要2-3个参数"); }
        let headers = match &args[0] {
            Value::Array(a) => a.iter().map(|v| match v { Value::String(s) => s.as_str().to_string(), _ => String::new() }).collect(),
            _ => self.panic_at(ln, 1, "gui_table 第一个参数必须是表头字符串数组"),
        };
        let rows = match &args[1] {
            Value::Array(a) => a.iter().map(|row| match row { Value::Array(cells) => (**cells).clone(), _ => Vec::new() }).collect(),
            _ => self.panic_at(ln, 1, "gui_table 第二个参数必须是二维数组（每行一个数组）"),
        };
        let onrow = if args.len() >= 3 { Some(args[2].clone()) } else { None };
        self.push_control(GuiControl::Table { headers, rows, onrow });
        Value::Int(0)
    }

    /// 解析颜色参数：[r,g,b,a] 数组 -> [u8;4]
    #[cfg(feature = "gui")]
    fn parse_color(&self, v: &Value, ln: u32) -> [u8; 4] {
        match v {
            Value::Array(a) => {
                let mut c = [255u8; 4];
                for (i, x) in a.iter().take(4).enumerate() {
                    c[i] = match x { Value::Int(n) => (*n).clamp(0, 255) as u8, _ => 0 };
                }
                c
            }
            _ => self.panic_at(ln, 1, "颜色必须是 [r,g,b,a] 数组"),
        }
    }

    /// gui_multiselect(label, options, initial_indices, fn(indices){...})：多选下拉，回调传选中索引数组
    #[cfg(feature = "gui")]
    fn gui_multiselect(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 4 { self.panic_at(ln, 1, "gui_multiselect(label, options, initial_indices, onchange) 需要4个参数"); }
        let label = match &args[0] { Value::String(s) => s.as_str().to_string(), _ => self.panic_at(ln, 1, "gui_multiselect 第一个参数必须是标签字符串") };
        let options = match &args[1] { Value::Array(a) => a.iter().map(|v| match v { Value::String(s) => s.as_str().to_string(), _ => String::new() }).collect(), _ => self.panic_at(ln, 1, "gui_multiselect 第二个参数必须是字符串数组") };
        let selected = match &args[2] { Value::Array(a) => a.iter().filter_map(|v| match v { Value::Int(n) => Some((*n).max(0) as usize), _ => None }).collect(), _ => self.panic_at(ln, 1, "gui_multiselect 第三个参数必须是初始索引数组") };
        let onchange = args[3].clone();
        self.push_control(GuiControl::Multiselect { label, options, selected, onchange });
        Value::Int(0)
    }

    /// gui_tabs(names, initial_index, fn(idx){...})：标签页切换，回调传选中索引
    #[cfg(feature = "gui")]
    fn gui_tabs(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 3 { self.panic_at(ln, 1, "gui_tabs(names, initial_index, onchange) 需要3个参数"); }
        let names = match &args[0] { Value::Array(a) => a.iter().map(|v| match v { Value::String(s) => s.as_str().to_string(), _ => String::new() }).collect(), _ => self.panic_at(ln, 1, "gui_tabs 第一个参数必须是字符串数组") };
        let selected = match &args[1] { Value::Int(n) => (*n).max(0) as usize, _ => 0 };
        let onchange = args[2].clone();
        self.push_control(GuiControl::Tabs { names, selected, onchange });
        Value::Int(0)
    }

    /// gui_canvas(width, height, fn(){...})：画布，draw_fn 内调用 canvas_line/rect/circle 收集绘制命令
    #[cfg(feature = "gui")]
    fn gui_canvas(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 3 { self.panic_at(ln, 1, "gui_canvas(width, height, draw_fn) 需要3个参数"); }
        let num = |v: &Value| -> f32 { match v { Value::Int(n) => *n as f32, Value::Float(f) => *f as f32, _ => 0.0 } };
        let width = num(&args[0]);
        let height = num(&args[1]);
        let cb = match &args[2] { Value::Closure(c) => c.clone(), _ => self.panic_at(ln, 1, "gui_canvas 第三个参数必须是绘制函数") };
        let mut cmds: Vec<CanvasCmd> = Vec::new();
        let ptr = &mut cmds as *mut Vec<CanvasCmd>;
        self.gui_canvas_stack.push(ptr);
        let _r = self.call_closure(&cb, Vec::new(), ln);
        self.gui_canvas_stack.pop();
        self.push_control(GuiControl::Canvas { width, height, cmds });
        Value::Int(0)
    }

    /// canvas 绘制命令入栈（draw_fn 内调用）
    #[cfg(feature = "gui")]
    fn canvas_push(&mut self, cmd: CanvasCmd) {
        if let Some(&top) = self.gui_canvas_stack.last() {
            unsafe { (*top).push(cmd); }
        }
    }

    /// canvas_line(x1,y1,x2,y2,color,width)：画线（在 gui_canvas 的 draw_fn 内调用）
    #[cfg(feature = "gui")]
    fn canvas_line(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 6 { self.panic_at(ln, 1, "canvas_line(x1,y1,x2,y2,color,width) 需要6个参数"); }
        let num = |v: &Value| -> f32 { match v { Value::Int(n) => *n as f32, Value::Float(f) => *f as f32, _ => 0.0 } };
        let (x1, y1, x2, y2) = (num(&args[0]), num(&args[1]), num(&args[2]), num(&args[3]));
        let color = self.parse_color(&args[4], ln);
        let width = num(&args[5]).max(0.5);
        self.canvas_push(CanvasCmd::Line { x1, y1, x2, y2, color, width });
        Value::Int(0)
    }

    /// canvas_rect(x,y,w,h,color)：画填充矩形（在 gui_canvas 的 draw_fn 内调用）
    #[cfg(feature = "gui")]
    fn canvas_rect(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 5 { self.panic_at(ln, 1, "canvas_rect(x,y,w,h,color) 需要5个参数"); }
        let num = |v: &Value| -> f32 { match v { Value::Int(n) => *n as f32, Value::Float(f) => *f as f32, _ => 0.0 } };
        let (x, y, w, h) = (num(&args[0]), num(&args[1]), num(&args[2]), num(&args[3]));
        let color = self.parse_color(&args[4], ln);
        self.canvas_push(CanvasCmd::Rect { x, y, w, h, color });
        Value::Int(0)
    }

    /// canvas_circle(cx,cy,r,color)：画填充圆（在 gui_canvas 的 draw_fn 内调用）
    #[cfg(feature = "gui")]
    fn canvas_circle(&mut self, args: Vec<Value>, ln: u32) -> Value {
        if args.len() != 4 { self.panic_at(ln, 1, "canvas_circle(cx,cy,r,color) 需要4个参数"); }
        let num = |v: &Value| -> f32 { match v { Value::Int(n) => *n as f32, Value::Float(f) => *f as f32, _ => 0.0 } };
        let (cx, cy, r) = (num(&args[0]), num(&args[1]), num(&args[2]));
        let color = self.parse_color(&args[3], ln);
        self.canvas_push(CanvasCmd::Circle { cx, cy, r, color });
        Value::Int(0)
    }

    /// quit()：通用退出。命令行立即退出进程；GUI 事件循环中关闭当前窗口（命令行与 GUI 通用）
    fn quit(&mut self, _args: Vec<Value>, _ln: u32) -> Value {
        #[cfg(feature = "gui")]
        {
            if let Some(ctx_ptr) = self.gui_ctx {
                unsafe { (*ctx_ptr).send_viewport_cmd(egui::ViewportCommand::Close); }
                return Value::Int(0);
            }
        }
        std::process::exit(0)
    }

    /// 实例方法/构造器调用：自动注入 self 变量（无论方法是否显式声明 self 形参），
    /// 并在调用期间记录当前类以支持私有字段访问
    fn call_func_with_self(&mut self, func: Func, self_val: Value, args: Vec<Value>, call_line: u32, method_class: Option<String>) -> Value {
        // 方法/构造器：支持显式 self 或隐式 self（构造器 init/new 可省 self，自动注入）
        let has_explicit_self = func.params.first() == Some(&"self".to_string());
        let expect_args = if has_explicit_self { func.params.len() - 1 } else { func.params.len() };
        if expect_args != args.len() {
            self.panic_at(call_line, 1, &format!("方法参数数量不匹配：需要 {}，实际 {}", expect_args, args.len()));
        }
        self.ctx_stack.push((format!("{}.{}", method_class.as_deref().unwrap_or("?"), func.name), func.line));
        let parent_env = std::mem::replace(&mut self.env, Env::new());
        let mut local_env = Env::nested(parent_env);
        // self 始终注入（隐式 self 不占形参位）
        local_env.define_var("self".to_string(), self_val.clone());
        // 绑定形参：显式 self 跳过第一个，隐式 self 全部绑定
        for (i, p) in func.params.iter().enumerate() {
            if has_explicit_self && i == 0 { continue; }
            let arg_idx = if has_explicit_self { i - 1 } else { i };
            local_env.define_var(p.clone(), args[arg_idx].clone());
        }
        self.env = local_env;
        let saved_class = self.current_class.clone();
        let saved_self = self.current_self.clone();
        self.current_class = method_class;
        self.current_self = Some(self_val);
        let (result, _ctrl) = self.exec_stmts_return_last(&func.body);
        let result = match &func.ret_ty {
            Some(rt) => self.cast_value(result, rt, func.line),
            None => result,
        };
        self.current_class = saved_class;
        self.current_self = saved_self;
        let local_env = std::mem::replace(&mut self.env, Env::new());
        self.env = *local_env.outer.unwrap();
        self.ctx_stack.pop();
        result
    }

    /// 隐藏类字段布局：沿继承链合并所有字段名 -> 槽位索引（祖先→子类），结果缓存到 field_cache
    fn get_field_layout(&mut self, class: &Rc<ClassDef>) -> Rc<FieldLayout> {
        let key = Rc::as_ptr(class) as usize;
        if let Some(l) = self.field_cache.get(&key) { return l.clone(); }
        let mut index: FastMap<u32, usize> = FastMap::new();
        let mut total = 0usize;
        let mut chain: Vec<Rc<ClassDef>> = Vec::new();
        let mut cur = Some(class.clone());
        while let Some(c) = cur.clone() {
            chain.push(c.clone());
            cur = self.superclass_rc(&c);
        }
        for c in chain.iter().rev() {
            for f in &c.fields { if !index.contains_key(f) { index.insert(*f, total); total += 1; } }
            for f in &c.private_fields { if !index.contains_key(f) { index.insert(*f, total); total += 1; } }
        }
        let layout = Rc::new(FieldLayout { index, total });
        self.field_cache.insert(key, layout.clone());
        layout
    }

    /// 沿继承链取父类定义
    fn superclass_rc(&self, class: &Rc<ClassDef>) -> Option<Rc<ClassDef>> {
        match &class.superclass {
            Some(p) => self.classes.get(p).cloned().map(Rc::new),
            None => None,
        }
    }

    /// 沿继承链判断某个类（含祖先）是否声明了字段
    fn class_declares_field(&self, class: &Rc<ClassDef>, field: u32) -> bool {
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            if c.fields.iter().any(|f| *f == field) || c.private_fields.iter().any(|f| *f == field) {
                return true;
            }
            cur = self.superclass_rc(&c);
        }
        false
    }

    /// 沿继承链查找实例方法（子类优先，支持重写），返回 (方法, 声明它的类名)
    fn find_method(&self, class: &Rc<ClassDef>, name: u32) -> Option<(Func, String)> {
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            if let Some(f) = c.methods.get(&name) { return Some((f.clone(), c.name.clone())); }
            cur = self.superclass_rc(&c);
        }
        None
    }

    /// 沿继承链查找静态方法
    fn find_static(&self, class: &Rc<ClassDef>, name: u32) -> Option<Func> {
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            if let Some(f) = c.statics.get(&name) { return Some(f.clone()); }
            cur = self.superclass_rc(&c);
        }
        None
    }

    /// 沿继承链找声明某私有字段的类名（用于权限判断）
    fn declaring_class_name(&self, class: &Rc<ClassDef>, field: u32) -> Option<String> {
        let mut cur = Some(class.clone());
        while let Some(c) = cur {
            if c.private_fields.iter().any(|f| *f == field) { return Some(c.name.clone()); }
            cur = self.superclass_rc(&c);
        }
        None
    }

    /// 数组访问辅助：返回数组长度（类型/长度检查在块内完成，借用结束后再报错）
    fn array_checked_len_id(&mut self, id: u32) -> usize {
        let outcome: Result<usize, String> = (|| {
            let entry = self.env.get_mut_id(id).unwrap_or_else(|| panic!("数组 {} 不存在", interner().lookup(id)));
            match &entry.value {
                VarStorage::Plain(Value::Array(v)) => Ok(v.len()),
                VarStorage::Plain(_) => Err("[]不是数组".to_string()),
                VarStorage::Shared(u) => match &*u.borrow() {
                    Value::Array(v) => Ok(v.len()),
                    _ => Err("[]不是数组".to_string()),
                },
            }
        })();
        match outcome {
            Ok(len) => len,
            Err(msg) => self.panic_here(&msg),
        }
    }

    fn array_checked_len(&mut self, arr_name: &str) -> usize {
        let outcome: Result<usize, String> = (|| {
            let entry = self.env.get_mut(arr_name).unwrap_or_else(|| panic!("数组 {} 不存在", arr_name));
            match &entry.value {
                VarStorage::Plain(Value::Array(v)) => Ok(v.len()),
                VarStorage::Plain(_) => Err("[]不是数组".to_string()),
                VarStorage::Shared(u) => match &*u.borrow() {
                    Value::Array(v) => Ok(v.len()),
                    _ => Err("[]不是数组".to_string()),
                },
            }
        })();
        match outcome {
            Ok(len) => len,
            Err(msg) => self.panic_here(&msg),
        }
    }

    /// 修改数组某下标（越界报错）
    fn array_set_index_id(&mut self, id: u32, idx: usize, val: Value) {
        let len = {
            let entry = self.env.get_mut_id(id).unwrap_or_else(|| panic!("数组 {} 不存在", interner().lookup(id)));
            match &entry.value {
                VarStorage::Plain(Value::Array(v)) => Ok(v.len()),
                VarStorage::Plain(_) => Err("[]不是数组".to_string()),
                VarStorage::Shared(u) => match &*u.borrow() {
                    Value::Array(v) => Ok(v.len()),
                    _ => Err("[]不是数组".to_string()),
                },
            }
        };
        let len = match len { Ok(l) => l, Err(msg) => self.panic_here(&msg) };
        if idx >= len { self.panic_here("数组下标越界"); }
        let entry = self.env.get_mut_id(id).unwrap_or_else(|| panic!("数组 {} 不存在", interner().lookup(id)));
        match &mut entry.value {
            VarStorage::Plain(Value::Array(v)) => { Rc::make_mut(v)[idx] = val; }
            VarStorage::Plain(_) => unreachable!(),
            VarStorage::Shared(u) => {
                let mut g = u.borrow_mut();
                let arr = match &mut *g { Value::Array(v) => v, _ => unreachable!() };
                Rc::make_mut(arr)[idx] = val;
            }
        }
    }

    fn array_set_index(&mut self, arr_name: &str, idx: usize, val: Value) {
        let len = {
            let entry = self.env.get_mut(arr_name).unwrap_or_else(|| panic!("数组 {} 不存在", arr_name));
            match &entry.value {
                VarStorage::Plain(Value::Array(v)) => Ok(v.len()),
                VarStorage::Plain(_) => Err("[]不是数组".to_string()),
                VarStorage::Shared(u) => match &*u.borrow() {
                    Value::Array(v) => Ok(v.len()),
                    _ => Err("[]不是数组".to_string()),
                },
            }
        };
        let len = match len { Ok(l) => l, Err(msg) => self.panic_here(&msg) };
        if idx >= len { self.panic_here("数组下标越界"); }
        let entry = self.env.get_mut(arr_name).unwrap_or_else(|| panic!("数组 {} 不存在", arr_name));
        match &mut entry.value {
            VarStorage::Plain(Value::Array(v)) => { Rc::make_mut(v)[idx] = val; }
            VarStorage::Plain(_) => unreachable!(),
            VarStorage::Shared(u) => {
                let mut g = u.borrow_mut();
                let arr = match &mut *g { Value::Array(v) => v, _ => unreachable!() };
                Rc::make_mut(arr)[idx] = val;
            }
        }
    }

    /// 读取数组某下标（越界报错）
    fn array_get_index(&mut self, arr_name: &str, idx: usize) -> Value {
        let len = {
            let entry = self.env.get_mut(arr_name).unwrap_or_else(|| panic!("数组 {} 不存在", arr_name));
            match &entry.value {
                VarStorage::Plain(Value::Array(v)) => Ok(v.len()),
                VarStorage::Plain(_) => Err("[]不是数组".to_string()),
                VarStorage::Shared(u) => match &*u.borrow() {
                    Value::Array(v) => Ok(v.len()),
                    _ => Err("[]不是数组".to_string()),
                },
            }
        };
        let len = match len { Ok(l) => l, Err(msg) => self.panic_here(&msg) };
        if idx >= len { self.panic_here("数组下标越界"); }
        let entry = self.env.get_mut(arr_name).unwrap_or_else(|| panic!("数组 {} 不存在", arr_name));
        match &entry.value {
            VarStorage::Plain(Value::Array(v)) => v[idx].clone(),
            VarStorage::Plain(_) => unreachable!(),
            VarStorage::Shared(u) => {
                let g = u.borrow();
                let arr = match &*g { Value::Array(v) => v, _ => unreachable!() };
                arr[idx].clone()
            }
        }
    }

    /// 实例化类：创建实例（字段默认 Int(0)），调用构造函数 new（若存在）
    fn instantiate(&mut self, class_def: ClassDef, args: Vec<Value>, ln: u32) -> Value {
        let class_rc = Rc::new(class_def);
        let layout = self.get_field_layout(&class_rc);
        let fields = vec![Value::Int(0); layout.total];
        let instance = Rc::new(RefCell::new(InstanceData {
            class: class_rc.clone(),
            fields,
        }));
        // 调用构造函数（若有）：自动注入 self（无论 new 是否显式声明 self 形参）
        if let Some(ctor) = class_rc.constructor.clone() {
            self.call_func_with_self(ctor, Value::Instance(instance.clone()), args, ln, Some(class_rc.name.clone()));
        }
        Value::Instance(instance)
    }

    fn exec_stmts_return_last(&mut self, stmts: &[Stmt]) -> (Value, LoopControl) {
        let mut result = Value::Int(0);
        for s in stmts {
            self.line = stmt_line(s);
            match s {
                Stmt::Return(ret_opt, _) => {
                    let ret_val = match ret_opt {
                        Some(e) => self.eval_expr(e),
                        None => Value::Int(0),
                    };
                    // 返回 Return 控制流，向上冒泡终止外层执行
                    return (ret_val.clone(), LoopControl::Return(ret_val));
                }
                Stmt::Break(_) => return (result, LoopControl::Break),
                Stmt::Continue(_) => return (result, LoopControl::Continue),
                other => {
                    let (res, ctrl) = self.exec_stmt(other);
                    result = res;
                    // 所有非 None 控制流（Break/Continue/Return）全部向上冒泡
                    if !matches!(ctrl, LoopControl::None) {
                        return (result, ctrl);
                    }
                }
            }
        }
        (result, LoopControl::None)
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> (Value, LoopControl) {
        match stmt {
            Stmt::Break(_) => (Value::Int(0), LoopControl::Break),
            Stmt::Continue(_) => (Value::Int(0), LoopControl::Continue),
            // yield：生成器平铺路径由 exec_stmts_until_yield 处理；此处仅保证穷尽
            Stmt::Yield(expr, _) => { let v = self.eval_expr(expr); (v, LoopControl::None) }

            // throw 表达式：保存值后以 ThrowMarker panic 跨栈冒泡（含跨函数/循环）
            Stmt::Throw(expr, _) => {
                let v = self.eval_expr(expr);
                self.pending_throw = Some(v);
                std::panic::panic_any(ThrowMarker);
            }

            // try/catch：捕获 ThrowMarker，绑定异常变量，执行 handler
            Stmt::Try { body, catch_var, handler, line: _ } => {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.exec_stmts_return_last(body)
                }));
                match res {
                    Ok((val, ctrl)) => (val, ctrl),
                    Err(payload) => {
                        if payload.downcast_ref::<ThrowMarker>().is_some() {
                            let thrown = self.pending_throw.take().unwrap_or(Value::Int(0));
                            if let Some(cvar) = catch_var {
                                self.env.define_var(cvar.clone(), thrown);
                            }
                            self.exec_stmts_return_last(handler)
                        } else {
                            // 非用户 throw（脚本错误等）向上重抛，不被 catch 捕获
                            std::panic::resume_unwind(payload)
                        }
                    }
                }
            }

            Stmt::Let(name, expr, _) => {
                let val = self.eval_expr(expr);
                self.env.define_var_id(*name, val);
                (Value::Int(0), LoopControl::None)
            }

            Stmt::Const(name, expr, _) => {
                let val = self.eval_expr(expr);
                self.env.define_const_id(*name, val);
                (Value::Int(0), LoopControl::None)
            }

            Stmt::FnDef(name, params, body, _ret, ln) => {
                self.funcs.insert(name.clone(), Func { name: name.clone(), line: *ln, params: params.clone(), body: body.clone(), ret_ty: None });
                (Value::Int(0), LoopControl::None)
            }

            Stmt::Class(def, _) => {
                let cname = def.name.clone();
                self.classes.insert(cname, def.clone());
                (Value::Int(0), LoopControl::None)
            }

            Stmt::If(cond, then_body, elif_branches, else_body, _) => {
                let val = self.eval_expr(cond);
                if val.is_truthy() {
                    return self.exec_stmts_return_last(then_body);
                } else {
                    for (e_cond, e_body) in elif_branches {
                        let e_val = self.eval_expr(e_cond);
                        if e_val.is_truthy() {
                            return self.exec_stmts_return_last(e_body);
                        }
                    }
                }
                self.exec_stmts_return_last(else_body)
            }

            Stmt::While(cond, body, _) => {
                loop {
                    let v = self.eval_expr(cond);
                    if !v.is_truthy() {
                        break;
                    }
                    let (res, ctrl) = self.exec_stmts_return_last(body);
                    match ctrl {
                        LoopControl::Break => break,
                        LoopControl::Continue => continue,
                        LoopControl::Return(_) => {
                            // 遇到 return 直接终止循环，向上冒泡
                            return (res, ctrl);
                        }
                        LoopControl::None => (),
                    }
                }
                (Value::Int(0), LoopControl::None)
            }

            Stmt::Print(args, _) => {
                fn print_value(v: &Value) {
                    match v {
                        Value::Int(n) => print!("{}", n),
                        Value::Int64(n) => print!("{}", n),
                        Value::Float(f) => print!("{}", f),
                        Value::String(s) => print!("{}", s.as_str()),
                        Value::Bool(b) => print!("{}", b),
                        Value::Range(lo, hi) => print!("{}..{}", fmt_range_num(*lo), fmt_range_num(*hi)),
                        Value::Array(arr) => {
                            print!("[");
                            for (i, item) in arr.iter().enumerate() {
                                if i > 0 { print!(", "); }
                                print_value(item);
                            }
                            print!("]");
                        }
                        Value::ArenaPtr(_) => print!("(ArenaPtr)"),
                        Value::RawPtr(_) => print!("(RawPtr)"),
                        Value::ArrayElementPtr(name, idx) => print!("(ArrayElementPtr) &{name}[{idx}]"),
                        Value::Class(class_rc) => print!("(class {})", class_rc.name),
                        Value::Instance(inst) => print!("(instance of {})", inst.borrow().class.name),
                        Value::Closure(_) => print!("(closure)"),
                        Value::Generator(_) => print!("(generator)"),
                        Value::Dict(map) => {
                            print!("{{");
                            let mut first = true;
                            for (k, v) in map.iter() {
                                if !first { print!(", "); }
                                print!("{}: ", k);
                                print_value(v);
                                first = false;
                            }
                            print!("}}");
                        }
                    }
                }

                for e in args {
                    let val = self.eval_expr(e);
                    print_value(&val);
                }
                println!();
                (Value::Int(0), LoopControl::None)
            }

            Stmt::For(var_name, start, end, body, _) => {
                let start_val = self.eval_expr(&start);
                // 生成器遍历：for x in gen()
                if let Value::Generator(g) = &start_val {
                    loop {
                        match self.generator_next_val(g.clone()) {
                            Some(v) => {
                                self.env.define_var_id(*var_name, v);
                                let (res, ctrl) = self.exec_stmts_return_last(body);
                                self.env.vars.remove(var_name);
                                match ctrl {
                                    LoopControl::Break => break,
                                    LoopControl::Continue => continue,
                                    LoopControl::Return(_) => return (res, ctrl),
                                    LoopControl::None => (),
                                }
                            }
                            None => break,
                        }
                    }
                    return (Value::Int(0), LoopControl::None);
                }
                // 数组遍历：for x in arr
                if let Value::Array(arr) = &start_val {
                    for item in arr.iter() {
                        self.env.define_var_id(*var_name, item.clone());
                        let (res, ctrl) = self.exec_stmts_return_last(body);
                        self.env.vars.remove(var_name);
                        match ctrl {
                            LoopControl::Break => break,
                            LoopControl::Continue => continue,
                            LoopControl::Return(_) => return (res, ctrl),
                            LoopControl::None => (),
                        }
                    }
                    return (Value::Int(0), LoopControl::None);
                }
                // 区间遍历（原有）
                let end_val = self.eval_expr(&end);
                let start_num = match start_val {
                    Value::Int(n) => n,
                    Value::Float(f) => f as i32,
                    _ => self.panic_here( "for 区间必须为数字类型"),
                };
                let end_num = match end_val {
                    Value::Int(n) => n,
                    Value::Float(f) => f as i32,
                    _ => self.panic_here( "for 区间必须为数字类型"),
                };

                if start_num <= end_num {
                    for i in start_num..end_num {
                        self.env.define_var_id(*var_name, Value::Int(i));
                        let (res, ctrl) = self.exec_stmts_return_last(body);
                        self.env.vars.remove(var_name);
                        match ctrl {
                            LoopControl::Break => break,
                            LoopControl::Continue => continue,
                            LoopControl::Return(_) => {
                                // 遇到 return 直接终止循环，向上冒泡
                                return (res, ctrl);
                            }
                            LoopControl::None => (),
                        }
                    }
                } else {
                    for i in (end_num..start_num).rev() {
                        self.env.define_var_id(*var_name, Value::Int(i));
                        let (res, ctrl) = self.exec_stmts_return_last(body);
                        self.env.vars.remove(var_name);
                        match ctrl {
                            LoopControl::Break => break,
                            LoopControl::Continue => continue,
                            LoopControl::Return(_) => {
                                return (res, ctrl);
                            }
                            LoopControl::None => (),
                        }
                    }
                }
                (Value::Int(0), LoopControl::None)
            }

            Stmt::Expr(expr, _) => {
                let val = self.eval_expr(expr);
                (val, LoopControl::None)
            }

            Stmt::ImportItem { lib_path: _, parts, alias, import_all: _, names, .. } => {
                if let Some(ns) = names {
                    resolve_named_import(self, parts, ns);
                } else {
                    resolve_import(self, parts);
                }
                if let Some(alias_name) = alias {
                    let full_mod_path = parts.join("::");
                    self.mod_alias.insert(alias_name.clone(), full_mod_path);
                }
                (Value::Int(0), LoopControl::None)
            }

            Stmt::UnsafeBlock(body, _) => {
                self.exec_stmts_return_last(body)
            }

            Stmt::Return(ret_opt, _) => {
                let val = match ret_opt {
                    Some(e) => self.eval_expr(e),
                    None => Value::Int(0),
                };
                (val, LoopControl::None)
            }
        }
    }

    /// 类型转换：expr -> i32/i64/float/str/bool
    fn eval_cast(&mut self, inner: &Expr, ty: &str, ln: u32) -> Value {
        let v = self.eval_expr(inner);
        self.cast_value(v, ty, ln)
    }

    /// 值类型转换：v -> i32/i64/float/str/bool
    fn cast_value(&mut self, v: Value, ty: &str, ln: u32) -> Value {
        match ty {
            "i32" | "int" => match v {
                Value::Int(i) => Value::Int(i),
                Value::Int64(i) => Value::Int(i as i32),
                Value::Float(f) => Value::Int(f as i32),
                Value::Bool(b) => Value::Int(if b { 1 } else { 0 }),
                _ => self.panic_at(ln, 1, "类型转换失败：不能转 i32"),
            },
            "i64" => match v {
                Value::Int(i) => Value::Int64(i as i64),
                Value::Int64(i) => Value::Int64(i),
                Value::Float(f) => Value::Int64(f as i64),
                Value::Bool(b) => Value::Int64(if b { 1 } else { 0 }),
                _ => self.panic_at(ln, 1, "类型转换失败：不能转 i64"),
            },
            "float" | "f64" | "double" => match v {
                Value::Int(i) => Value::Float(i as f64),
                Value::Int64(i) => Value::Float(i as f64),
                Value::Float(f) => Value::Float(f),
                Value::Bool(b) => Value::Float(if b { 1.0 } else { 0.0 }),
                _ => self.panic_at(ln, 1, "类型转换失败：不能转 float"),
            },
            "str" | "string" => Value::String(PoolStr::new(&fmt_value(&v))),
            "bool" => Value::Bool(v.is_truthy()),
            "void" => v,
            _ => self.panic_at(ln, 1, &format!("未知类型: {}", ty)),
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Cast(inner, ty, ln) => self.eval_cast(inner, ty, *ln),
            Expr::Int64(n, _) => Value::Int64(*n),
            Expr::Match(subject, arms, ln) => {
                let sv = self.eval_expr(subject);
                for (pat, body) in arms {
                    let hit = match pat {
                        MatchPat::Wildcard => true,
                        MatchPat::Lit(lv) => value_matches(lv, &sv),
                        MatchPat::Range(lo, hi) => match &sv {
                            Value::Int(n) => (*n as f64) >= *lo && (*n as f64) <= *hi,
                            Value::Float(f) => *f >= *lo && *f <= *hi,
                            _ => false,
                        },
                    };
                    if hit { return self.eval_expr(body); }
                }
                self.panic_at(*ln, 1, "match 无匹配分支（可添加 _ 通配分支）");
            }
            Expr::Range(l, r, _ln) => {
                let lv = self.eval_expr(l);
                let rv = self.eval_expr(r);
                let rg = match (lv, rv) {
                    (Value::Int(a), Value::Int(b)) => (a as f64, b as f64),
                    (Value::Int64(a), Value::Int64(b)) => (a as f64, b as f64),
                    (Value::Int64(a), Value::Int(b)) => (a as f64, b as f64),
                    (Value::Int(a), Value::Int64(b)) => (a as f64, b as f64),
                    (Value::Int(a), Value::Float(b)) => (a as f64, b),
                    (Value::Float(a), Value::Int(b)) => (a, b as f64),
                    (Value::Int64(a), Value::Float(b)) => (a as f64, b),
                    (Value::Float(a), Value::Int64(b)) => (a, b as f64),
                    (Value::Float(a), Value::Float(b)) => (a, b),
                    _ => self.panic_here(".. 区间端点必须是数字"),
                };
                Value::Range(rg.0, rg.1)
            }
            Expr::Neg(inner, _) => {
                let val = self.eval_expr(inner);
                match val {
                    Value::Int(n) => Value::Int(-n),
                    Value::Int64(n) => Value::Int64(-n),
                    Value::Float(f) => Value::Float(-f),
                    _ => self.panic_here( "一元负号仅支持整数和浮点数"),
                }
            }
            Expr::Not(inner, _) => {
                let val = self.eval_expr(inner);
                let t = match val {
                    Value::Bool(b) => b,
                    Value::Int(n) => n != 0,
                    Value::Float(f) => f != 0.0,
                    Value::String(x) => !x.as_str().is_empty(),
                    _ => true, // 其余值视为非空真
                };
                Value::Bool(!t)
            }
            Expr::Member(e, member, ln) => {
                let v = self.eval_expr(e);
                let mid = *member;
                if mid == interner().get("f") {
                    match v {
                        Value::Int(i) => Value::Float(i as f64),
                        Value::Float(f) => Value::Float(f),
                        val => self.panic_at(*ln, 1, &format!("类型 {:?} 没有成员 .f", val)),
                    }
                } else if mid == interner().get("len") {
                    match v {
                        Value::String(s) => Value::Int(s.len() as i32),
                        Value::Array(arr) => Value::Int(arr.len() as i32),
                        val => self.panic_at(*ln, 1, &format!("类型 {:?} 没有成员 .len", val)),
                    }
                } else {
                    match v {
                        Value::Instance(inst) => {
                            let class_rc = inst.borrow().class.clone();
                            let layout = self.get_field_layout(&class_rc);
                            match layout.index.get(&mid) {
                                Some(&idx) => inst.borrow().fields[idx].clone(),
                                None => self.panic_at(*ln, 1, &format!("实例没有字段 {}", interner().lookup(mid))),
                            }
                        }
                        val => self.panic_at(*ln, 1, &format!("类型 {:?} 没有成员 .{}", val, interner().lookup(mid))),
                    }
                }
            }
            // ========== Arena 安全指针 &expr ==========
            Expr::AddrOf(inner_expr, addr_line) => {
                match inner_expr.as_ref() {
                    // 情况1：普通变量 &a
                    Expr::Ident(var_name, _) => {
                        if !self.env.contains_id(*var_name) {
                            self.panic_at(*addr_line, 1, &format!("变量 {} 不存在", interner().lookup(*var_name)));
                        }
                        let entry = self.env.get_mut_id(*var_name).unwrap();
                        let is_str = match &entry.value {
                            VarStorage::Plain(v) => matches!(v, Value::String(_)),
                            VarStorage::Shared(u) => matches!(*u.borrow(), Value::String(_)),
                        };
                        if is_str {
                            self.panic_at(*addr_line, 1, "运行时错误：字符串变量禁止使用unsafe &");
                        }
                        let mut shared_guard: Option<std::cell::RefMut<Value>>;
                        let val: &mut Value = match &mut entry.value {
                            VarStorage::Plain(v) => v,
                            VarStorage::Shared(u) => {
                                shared_guard = Some(u.borrow_mut());
                                &mut *shared_guard.as_mut().unwrap()
                            }
                        };
                        let addr = val as *mut Value as *mut u8;
                        Value::ArenaPtr(addr)
                    }
                    // 情况2：数组下标 &arr[1]
                    Expr::Index(arr_expr, idx_expr, _) => {
                        let idx_val = self.eval_expr(idx_expr);
                        let idx = match idx_val {
                            Value::Int(i) => i as usize,
                            _ => self.panic_here( "数组下标必须是整数"),
                        };
                        let arr_name = match arr_expr.as_ref() {
                            Expr::Ident(name, _) => name.clone(),
                            _ => self.panic_here( "仅支持变量数组取元素地址"),
                        };
                        if self.env.is_const_id(arr_name) {
                            self.panic_here( "常量数组禁止取地址");
                        }
                        // 仅校验下标合法，不分配内存、不取临时元素裸指针
                        let len = self.array_checked_len_id(arr_name);
                        if idx >= len {
                            self.panic_here( "数组下标越界");
                        }
                        // 只存数组名+下标，返回安全指针，彻底规避悬空
                        Value::ArrayElementPtr(interner().lookup(arr_name), idx)
                    }
                    _ => self.panic_here("仅支持对变量、数组元素取地址"),
                }
            }

            // 指针赋值 *ptr = value
            Expr::DerefAssign(ptr_expr, val_expr, _) => {
                let ptr_val = self.eval_expr(ptr_expr);
                let new_val = self.eval_expr(val_expr);
                match ptr_val {
                    Value::ArenaPtr(addr) | Value::RawPtr(addr) => {
                        let dst = addr as *mut Value;
                        unsafe {
                            let _old = std::ptr::replace(dst, new_val.clone());
                        }
                        new_val
                    }
                    // 新增：直接修改原数组对应下标，不再改拷贝
                    Value::ArrayElementPtr(arr_name, idx) => {
                        self.array_set_index(&arr_name, idx, new_val.clone());
                        new_val
                    }
                    _ => self.panic_here( "*ptr=仅支持指针"),
                }
            }

            // ========== Raw 裸指针 unsafe & expr ==========
            Expr::RawAddr(inner_expr, addr_line) => {
                match inner_expr.as_ref() {
                    // 普通变量 unsafe & b
                    Expr::Ident(var_name, _) => {
                        if !self.env.contains_id(*var_name) {
                            self.panic_at(*addr_line, 1, &format!("变量 {} 不存在", interner().lookup(*var_name)));
                        }
                        let entry = self.env.vars.get_mut(var_name).unwrap();
                        let is_str = match &entry.value {
                            VarStorage::Plain(v) => matches!(v, Value::String(_)),
                            VarStorage::Shared(u) => matches!(*u.borrow(), Value::String(_)),
                        };
                        if is_str {
                            self.panic_at(*addr_line, 1, "运行时错误：字符串变量禁止使用unsafe &");
                        }
                        let mut shared_guard: Option<std::cell::RefMut<Value>>;
                        let val: &mut Value = match &mut entry.value {
                            VarStorage::Plain(v) => v,
                            VarStorage::Shared(u) => {
                                shared_guard = Some(u.borrow_mut());
                                &mut *shared_guard.as_mut().unwrap()
                            }
                        };
                        let addr = val as *mut Value as *mut u8;
                        Value::RawPtr(addr)
                    }
                    // 数组元素 unsafe & arr[0]
                    Expr::Index(arr_expr, idx_expr, _) => {
                        let idx_val = self.eval_expr(idx_expr);
                        let idx = match idx_val {
                            Value::Int(i) => i as usize,
                            _ => self.panic_here( "数组下标必须是整数"),
                        };
                        let arr_name = match arr_expr.as_ref() {
                            Expr::Ident(name, _) => name.clone(),
                            _ => self.panic_here( "仅支持变量数组取元素地址"),
                        };
                        if self.env.is_const_id(arr_name) {
                            self.panic_here( "常量数组禁止取地址");
                        }
                        // 仅校验下标合法，不分配内存、不取临时元素裸指针
                        let len = self.array_checked_len_id(arr_name);
                        if idx >= len {
                            self.panic_here( "数组下标越界");
                        }
                        // 只存数组名+下标，返回安全指针
                        Value::ArrayElementPtr(interner().lookup(arr_name), idx)
                    }
                    _ => self.panic_here("裸指针 unsafe & 仅支持变量、数组元素"),
                }
            }

            // ========== 解引用 *expr ==========
            Expr::Deref(expr, _) => {
                let ptr_val = self.eval_expr(expr);
                match ptr_val {
                    Value::ArenaPtr(addr) | Value::RawPtr(addr) => {
                        let p = addr as *mut Value;
                        unsafe { (*p).clone() }
                    }
                    // 新增：数组安全指针实时读取原数组元素
                    Value::ArrayElementPtr(arr_name, idx) => {
                        self.array_get_index(&arr_name, idx)
                    }
                    _ => self.panic_here( "*仅支持指针类型"),
                }
            }

            Expr::Number(n, _) => Value::Int(*n),
            Expr::Float(f, _) => Value::Float(*f),
            Expr::String(s, str_line) => {
                let mut res = String::new();
                let mut chars = s.chars().peekable();
                while let Some(c) = chars.next() {
                    if c == '$' {
                        // 转义 $$ 输出单个$
                        if chars.peek() == Some(&'$') {
                            chars.next();
                            res.push('$');
                            continue;
                        }
                        // 判断是否为标准 ${}
                        if let Some(&'{') = chars.peek() {
                            chars.next();
                            let mut expr_src = String::new();
                            let mut brace_depth = 0;
                            loop {
                                match chars.next() {
                                    Some('{') => {
                                        brace_depth += 1;
                                        expr_src.push('{');
                                    }
                                    Some('}') => {
                                        if brace_depth == 0 {
                                            break;
                                        }
                                        brace_depth -= 1;
                                        expr_src.push('}');
                                    }
                                    Some(ch) => expr_src.push(ch),
                                    None => self.panic_here( "插值 ${} 缺少闭合 }")
                                }
                            }

                            // ============ 新增：检测格式化模式 expr , (digits).f ============
                            // 在顶层括号深度0分割逗号
                            let mut top_level_comma_pos: Option<usize> = None;
                            let mut paren_depth = 0;
                            for (idx, ch) in expr_src.chars().enumerate() {
                                match ch {
                                    '(' | '[' | '{' => paren_depth +=1,
                                    ')' | ']' | '}' => paren_depth -=1,
                                    ',' if paren_depth == 0 => {
                                        top_level_comma_pos = Some(idx);
                                        break;
                                    }
                                    _ => {}
                                }
                            }

                            if let Some(comma_idx) = top_level_comma_pos {
                                // 存在顶层逗号：尝试解析格式化 ${val , (n).f }
                                let val_src = &expr_src[0..comma_idx];
                                let fmt_src = &expr_src[comma_idx+1..];

                                // fmt_src 必须匹配模式： ( ... ).f
                                let fmt_trim = fmt_src.trim();
                                if fmt_trim.starts_with('(') && fmt_trim.ends_with(".f") {
                                    let inner_part = &fmt_trim[1..fmt_trim.len()-2];
                                    // 解析value表达式
                                    {
                                        let tmp_tok = Tokenizer::new(val_src.trim());
                                        let mut tmp_parser = Parser::new(tmp_tok);
                                        tmp_parser.file = self.file.clone();
                                        tmp_parser.line = self.line;
                                        let val_expr = tmp_parser.parse_expr();
                                        let val = self.eval_expr(&val_expr);

                                        // 解析位数表达式
                                        let tmp_tok2 = Tokenizer::new(inner_part.trim());
                                        let mut tmp_parser2 = Parser::new(tmp_tok2);
                                        tmp_parser2.file = self.file.clone();
                                        tmp_parser2.line = self.line;
                                        let digit_expr = tmp_parser2.parse_expr();
                                        let digit_val = self.eval_expr(&digit_expr);

                                        let digits = match digit_val {
                                            Value::Int(i) => i,
                                            _ => self.panic_at(*str_line,1,"格式化 .f 的位数必须是整数"),
                                        };
                                        if digits <0 || digits>20 {
                                            self.panic_at(*str_line,1,"格式化位数必须0~20");
                                        }

                                        // 将输入值转为f64
                                        let f_in = match val {
                                            Value::Int(i) => i as f64,
                                            Value::Float(f) => f,
                                            _ => self.panic_at(*str_line,1,", (N).f 格式化只支持数字"),
                                        };
                                        // 四舍五入保留digits位小数
                                        let mul = 10_f64.powi(digits);
                                        let rounded = (f_in * mul).round() / mul;
                                        res.push_str(&format!("{0:.1$}", rounded, digits as usize));
                                    }
                                } else {
                                    // 逗号存在但是不是 (xxx).f 格式 → 报错
                                    self.panic_at(*str_line,1,"插值内逗号仅支持格式：${expr, (N).f}");
                                }
                            } else {
                                // ============ 原有逻辑：无逗号，普通插值 ============
                                let tmp_tok = Tokenizer::new(&expr_src);
                                let mut tmp_parser = Parser::new(tmp_tok);
                                tmp_parser.file = self.file.clone();
                                tmp_parser.line = self.line;
                                let inner_expr = tmp_parser.parse_expr();
                                let inner_val = self.eval_expr(&inner_expr);

                                let var_str = match inner_val {
                                    Value::Int(n) => n.to_string(),
                                    Value::Int64(n) => n.to_string(),
                                    Value::Float(f) => f.to_string(),
                                    Value::String(s) => s.as_str().to_string(),
                                    Value::Bool(b) => b.to_string(),
                                    Value::Dict(map) => fmt_value(&Value::Dict(map)),
                                    Value::Array(arr) => {
                                        let mut arr_s = String::from("[");
                                        for (idx, item) in arr.iter().enumerate() {
                                            if idx > 0 { arr_s.push_str(", "); }
                                            match item {
                                                Value::Int(n) => arr_s.push_str(&n.to_string()),
                                                Value::Int64(n) => arr_s.push_str(&n.to_string()),
                                                Value::Float(f) => arr_s.push_str(&f.to_string()),
                                                Value::String(st) => arr_s.push_str(st.as_str()),
                                                Value::Bool(b) => arr_s.push_str(&b.to_string()),
                                                Value::Range(lo, hi) => arr_s.push_str(&format!("{}..{}", fmt_range_num(*lo), fmt_range_num(*hi))),
                                                Value::Array(_) => arr_s.push_str("[]"),
                                                Value::ArenaPtr(_) => arr_s.push_str("(ArenaPtr)"),
                                                Value::RawPtr(_) => arr_s.push_str("(RawPtr)"),
                                                Value::ArrayElementPtr(n, i) => arr_s.push_str(&format!("(ArrayElementPtr) &{}[{}]", n, i)),
                                                Value::Class(c) => arr_s.push_str(&format!("(class {})", c.name)),
                                                Value::Instance(i) => arr_s.push_str(&format!("(instance of {})", i.borrow().class.name)),
                                                Value::Closure(_) => arr_s.push_str("(closure)"),
                                                Value::Dict(map) => arr_s.push_str(&fmt_value(&Value::Dict(map.clone()))),
                                                Value::Generator(_) => arr_s.push_str("(generator)"),
                                            }
                                        }
                                        arr_s.push(']');
                                        arr_s
                                    }
                                    Value::ArenaPtr(_) => "(ArenaPtr)".to_string(),
                                    Value::RawPtr(_) => "(RawPtr)".to_string(),
                                    Value::ArrayElementPtr(n, i) => format!("(ArrayElementPtr) &{}[{}]", n, i),
                                    Value::Range(lo, hi) => format!("{}..{}", fmt_range_num(lo), fmt_range_num(hi)),
                                    Value::Class(c) => format!("(class {})", c.name),
                                    Value::Instance(i) => format!("(instance of {})", i.borrow().class.name),
        Value::Closure(_) => "(closure)".to_string(),
        Value::Generator(_) => "(generator)".to_string(),
                                };
                                res.push_str(&var_str);
                            }
                        } else {
                            // $后面不是{，直接报错
                            script_panic(
                                &self.file,
                                *str_line,
                                "插值语法错误：$ 后必须紧跟 {表达式}，例 ${X}；如需输出美元符号请写 $$"
                            );
                        }
                    } else {
                        res.push(c);
                    }
                }
                Value::String(PoolStr::new(&res))
            }
            Expr::RawString(s, _str_line) => Value::String(PoolStr::new(s)),
            Expr::Bool(b, _) => Value::Bool(*b),
            Expr::Ident(name, ln) => {
                // 快路径：普通变量直接用整数 ID 查（免字符串哈希），一次查表拿 &Value 再克隆
                let id = *name;
                if let Some(v) = self.env.get_ref_id(id) {
                    return v.clone();
                }
                if self.env.contains_id(id) {
                    return self.env.get_id(id);
                }
                // 慢路径：模块别名展开 / 类名（非热点）
                let name_str = interner().lookup(id);
                let segs: Vec<&str> = name_str.split("::").collect();
                let real_full = if !segs.is_empty() {
                    if let Some(real_mod) = self.mod_alias.get(segs[0]) {
                        let mut new_parts = vec![real_mod.as_str()];
                        new_parts.extend(&segs[1..]);
                        new_parts.join("::")
                    } else {
                        name_str.clone()
                    }
                } else {
                    name_str.clone()
                };

                if !self.env.contains(&real_full) {
                    // 类名作为值：返回 Value::Class，支持 Person.hello() 静态调用
                    if let Some(class_def) = self.classes.get(&real_full) {
                        return Value::Class(Rc::new(class_def.clone()));
                    }
                    self.panic_at(*ln, 1, &format!("变量未定义: {}", real_full));
                }
                self.env.get(&real_full)
            }
            Expr::Array(items, _) => {
                let mut vals = Vec::new();
                for item in items {
                    vals.push(self.eval_expr(item));
                }
                Value::Array(Rc::new(vals))
            }
            Expr::Index(arr_expr, idx_expr, _) => {
                let arr_val = self.eval_expr(arr_expr);
                let idx_val = self.eval_expr(idx_expr);
                match (arr_val, idx_val) {
                    (Value::Array(arr), Value::Int(i)) => {
                        let idx = i as usize;
                        if idx >= arr.len() {
                            self.panic_here("数组下标越界");
                        }
                        arr[idx].clone()
                    }
                    (Value::Dict(map), Value::String(k)) => {
                        let ks = k.as_str().to_string();
                        match map.get(&ks) {
                            Some(v) => v.clone(),
                            None => self.panic_here(&format!("字典没有键 {}", ks)),
                        }
                    }
                    _ => self.panic_here("[]下标仅支持：数组(整数下标)或字典(字符串键)"),
                }
            }
            Expr::IndexAssign(arr_expr, idx_expr, val_expr, _) => {
                if let Expr::Ident(arr_name, _) = &**arr_expr {
                    if self.env.is_const_id(*arr_name) {
                        self.panic_here(&format!("常量数组{}不可修改", interner().lookup(*arr_name)));
                    }
                }
                let mut arr_val = self.eval_expr(arr_expr);
                let idx_val = self.eval_expr(idx_expr);
                let new_val = self.eval_expr(val_expr);
                match &arr_val {
                    Value::Array(_) => {
                        let idx = match idx_val {
                            Value::Int(i) => i as usize,
                            _ => self.panic_here("数组下标必须为整数"),
                        };
                        match &mut arr_val {
                            Value::Array(arr) => {
                                if idx >= arr.len() {
                                    self.panic_here("数组下标越界");
                                }
                                Rc::make_mut(arr)[idx] = new_val.clone();
                            }
                            _ => unreachable!(),
                        }
                    }
                    Value::Dict(_) => {
                        let k = match idx_val {
                            Value::String(s) => s.as_str().to_string(),
                            _ => self.panic_here("字典键必须是字符串"),
                        };
                        match &mut arr_val {
                            Value::Dict(map) => { Rc::make_mut(map).insert(k, new_val.clone()); }
                            _ => unreachable!(),
                        }
                    }
                    _ => self.panic_here("[]赋值仅支持：数组(整数下标)或字典(字符串键)"),
                }
                match &**arr_expr {
                    Expr::Ident(name, _) => self.env.set_id(*name, arr_val),
                    _ => self.panic_here("仅支持变量数组/字典赋值"),
                }
                new_val
            }
            Expr::BinOp(lhs, op, rhs, op_line) => {
                // 逻辑或 or：短路，左边为真直接返回，不计算右边
                if let Op::Or = op {
                    let left_val = self.eval_expr(lhs);
                    if left_val.is_truthy() {
                        return Value::Bool(true);
                    }
                    let right_val = self.eval_expr(rhs);
                    return Value::Bool(right_val.is_truthy());
                }

                // 逻辑与 and：短路求值，左边为假直接返回，不执行右边
                if let Op::And = op {
                    let left_val = self.eval_expr(lhs);
                    if !left_val.is_truthy() {
                        return Value::Bool(false);
                    }
                    let right_val = self.eval_expr(rhs);
                    return Value::Bool(right_val.is_truthy());
                }

                if let Op::RelCmp = op {
                    let l = self.eval_expr(lhs);
                    let r = self.eval_expr(rhs);
                    let res = match (l, r) {
                        (Value::Int(a), Value::Int(b)) => {
                            if a > b { ">" }
                            else if a < b { "<" }
                            else { "=" }
                        }
                        (Value::Float(a), Value::Float(b)) => {
                            if a > b { ">" }
                            else if a < b { "<" }
                            else { "=" }
                        }
                        _ => self.panic_here( "?= 仅支持数字"),
                    };
                    return Value::String(PoolStr::new(res));
                }

                // 其他运算符正常求值两边
                let l = self.eval_expr(lhs);
                let r = self.eval_expr(rhs);

                // 仅左操作数为数组安全指针且为四则运算才执行数组原地运算
                if matches!(op, Op::Add | Op::Sub | Op::Mul | Op::Div) {
                if let Value::ArrayElementPtr(arr_name, idx) = &l {
                    // 读取数组当前原值
                    let origin = self.array_get_index(arr_name, *idx);
                    // 仅数字四则
                    let res = match (origin, r, op) {
                        (Value::Int(a), Value::Int(b), Op::Add) => Value::Int(a + b),
                        (Value::Int(a), Value::Int(b), Op::Sub) => Value::Int(a - b),
                        (Value::Int(a), Value::Int(b), Op::Mul) => Value::Int(a * b),
                        (Value::Int(a), Value::Int(b), Op::Div) => Value::Int(a / b),

                        (Value::Float(a), Value::Float(b), Op::Add) => Value::Float(a + b),
                        (Value::Float(a), Value::Float(b), Op::Sub) => Value::Float(a - b),
                        (Value::Float(a), Value::Float(b), Op::Mul) => Value::Float(a * b),
                        (Value::Float(a), Value::Float(b), Op::Div) => Value::Float(a / b),

                        _ => self.panic_here( "数组指针仅支持数字四则"),
                    };
                    // 直接写回原始数组，修改永久生效
                    self.array_set_index(&arr_name, *idx, res.clone());
                    return res;
                }
                }

                // ========== 下面原有全部代码完全不动 ==========
                if l.is_arena_ptr() || l.is_raw_ptr() || r.is_arena_ptr() || r.is_raw_ptr() {
                    if !matches!(op, Op::Equal | Op::Neq) {
                        self.panic_at(*op_line, 1, "运行时错误：指针禁止参与算术运算（仅支持 == / !=）");
                    }
                }

                match (l, r) {
                    (Value::Int(a), Value::Int(b)) => match op {
                        Op::Add => Value::Int(a + b),
                        Op::Sub => Value::Int(a - b),
                        Op::Mul => Value::Int(a * b),
                        Op::Div => Value::Int(a / b),
                        Op::Mod => Value::Int(a % b),
                        Op::Gt => Value::Int(if a > b { 1 } else { 0 }),
                        Op::Lt => Value::Int(if a < b { 1 } else { 0 }),
                        Op::Ge => Value::Int(if a >= b { 1 } else { 0 }),
                        Op::Le => Value::Int(if a <= b { 1 } else { 0 }),
                        Op::Equal => Value::Bool(a == b),
                        Op::Neq => Value::Bool(a != b),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Int64(a), Value::Int64(b)) => match op {
                        Op::Add => Value::Int64(a + b),
                        Op::Sub => Value::Int64(a - b),
                        Op::Mul => Value::Int64(a * b),
                        Op::Div => Value::Int64(a / b),
                        Op::Mod => Value::Int64(a % b),
                        Op::Gt => Value::Int(if a > b { 1 } else { 0 }),
                        Op::Lt => Value::Int(if a < b { 1 } else { 0 }),
                        Op::Ge => Value::Int(if a >= b { 1 } else { 0 }),
                        Op::Le => Value::Int(if a <= b { 1 } else { 0 }),
                        Op::Equal => Value::Bool(a == b),
                        Op::Neq => Value::Bool(a != b),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Int64(a), Value::Int(b)) => match op {
                        Op::Add => Value::Int64(a + b as i64),
                        Op::Sub => Value::Int64(a - b as i64),
                        Op::Mul => Value::Int64(a * b as i64),
                        Op::Div => Value::Int64(a / b as i64),
                        Op::Mod => Value::Int64(a % b as i64),
                        Op::Gt => Value::Int(if a > b as i64 { 1 } else { 0 }),
                        Op::Lt => Value::Int(if a < b as i64 { 1 } else { 0 }),
                        Op::Ge => Value::Int(if a >= b as i64 { 1 } else { 0 }),
                        Op::Le => Value::Int(if a <= b as i64 { 1 } else { 0 }),
                        Op::Equal => Value::Bool(a == b as i64),
                        Op::Neq => Value::Bool(a != b as i64),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Int(a), Value::Int64(b)) => match op {
                        Op::Add => Value::Int64(a as i64 + b),
                        Op::Sub => Value::Int64(a as i64 - b),
                        Op::Mul => Value::Int64(a as i64 * b),
                        Op::Div => Value::Int64(a as i64 / b),
                        Op::Mod => Value::Int64(a as i64 % b),
                        Op::Gt => Value::Int(if (a as i64) > b { 1 } else { 0 }),
                        Op::Lt => Value::Int(if (a as i64) < b { 1 } else { 0 }),
                        Op::Ge => Value::Int(if (a as i64) >= b { 1 } else { 0 }),
                        Op::Le => Value::Int(if (a as i64) <= b { 1 } else { 0 }),
                        Op::Equal => Value::Bool((a as i64) == b),
                        Op::Neq => Value::Bool((a as i64) != b),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Int64(a), Value::Float(b)) => match op {
                        Op::Add => Value::Float((a as f64) + b),
                        Op::Sub => Value::Float((a as f64) - b),
                        Op::Mul => Value::Float((a as f64) * b),
                        Op::Div => Value::Float((a as f64) / b),
                        Op::Mod => Value::Float((a as f64) % b),
                        Op::Gt => Value::Int(if (a as f64) > b { 1 } else { 0 }),
                        Op::Lt => Value::Int(if (a as f64) < b { 1 } else { 0 }),
                        Op::Ge => Value::Int(if (a as f64) >= b { 1 } else { 0 }),
                        Op::Le => Value::Int(if (a as f64) <= b { 1 } else { 0 }),
                        Op::Equal => Value::Bool((a as f64) == b),
                        Op::Neq => Value::Bool((a as f64) != b),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Float(a), Value::Int64(b)) => match op {
                        Op::Add => Value::Float(a + (b as f64)),
                        Op::Sub => Value::Float(a - (b as f64)),
                        Op::Mul => Value::Float(a * (b as f64)),
                        Op::Div => Value::Float(a / (b as f64)),
                        Op::Mod => Value::Float(a % (b as f64)),
                        Op::Gt => Value::Int(if a > (b as f64) { 1 } else { 0 }),
                        Op::Lt => Value::Int(if a < (b as f64) { 1 } else { 0 }),
                        Op::Ge => Value::Int(if a >= (b as f64) { 1 } else { 0 }),
                        Op::Le => Value::Int(if a <= (b as f64) { 1 } else { 0 }),
                        Op::Equal => Value::Bool(a == (b as f64)),
                        Op::Neq => Value::Bool(a != (b as f64)),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    // Int 与 Float 混合运算：自动提升为 Float（原实现会直接报类型不匹配）
                    (Value::Int(a), Value::Float(b)) => match op {
                        Op::Add => Value::Float((a as f64) + b),
                        Op::Sub => Value::Float((a as f64) - b),
                        Op::Mul => Value::Float((a as f64) * b),
                        Op::Div => Value::Float((a as f64) / b),
                        Op::Mod => Value::Float((a as f64) % b),
                        Op::Gt => Value::Int(if (a as f64) > b { 1 } else { 0 }),
                        Op::Lt => Value::Int(if (a as f64) < b { 1 } else { 0 }),
                        Op::Ge => Value::Int(if (a as f64) >= b { 1 } else { 0 }),
                        Op::Le => Value::Int(if (a as f64) <= b { 1 } else { 0 }),
                        Op::Equal => Value::Bool((a as f64) == b),
                        Op::Neq => Value::Bool((a as f64) != b),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Float(a), Value::Int(b)) => match op {
                        Op::Add => Value::Float(a + (b as f64)),
                        Op::Sub => Value::Float(a - (b as f64)),
                        Op::Mul => Value::Float(a * (b as f64)),
                        Op::Div => Value::Float(a / (b as f64)),
                        Op::Mod => Value::Float(a % (b as f64)),
                        Op::Gt => Value::Int(if a > (b as f64) { 1 } else { 0 }),
                        Op::Lt => Value::Int(if a < (b as f64) { 1 } else { 0 }),
                        Op::Ge => Value::Int(if a >= (b as f64) { 1 } else { 0 }),
                        Op::Le => Value::Int(if a <= (b as f64) { 1 } else { 0 }),
                        Op::Equal => Value::Bool(a == (b as f64)),
                        Op::Neq => Value::Bool(a != (b as f64)),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    (Value::Float(a), Value::Float(b)) => match op {
                        Op::Add => Value::Float(a + b),
                        Op::Sub => Value::Float(a - b),
                        Op::Mul => Value::Float(a * b),
                        Op::Div => Value::Float(a / b),
                        Op::Mod => Value::Float(a % b),
                        Op::Gt => Value::Int(if a > b { 1 } else { 0 }),
                        Op::Lt => Value::Int(if a < b { 1 } else { 0 }),
                        Op::Ge => Value::Int(if a >= b { 1 } else { 0 }),
                        Op::Le => Value::Int(if a <= b { 1 } else { 0 }),
                        Op::Equal => Value::Bool(a == b),
                        Op::Neq => Value::Bool(a != b),
                        Op::And => unreachable!(),
                        Op::Or => unreachable!(),
                        Op::RelCmp => unreachable!(),
                    },
                    // 字符串：+ 拼接 / == != 比较
                    (Value::String(a), Value::String(b)) => match op {
                        Op::Add => Value::String(PoolStr::concat(&a, &b)),
                        Op::Equal => Value::Bool(a.as_str() == b.as_str()),
                        Op::Neq => Value::Bool(a.as_str() != b.as_str()),
                        _ => self.panic_here( "字符串仅支持 + 拼接、== 和 != 比较，不支持 > < >= <= ?="),
                    },
                    (Value::Array(mut a_arr), Value::Array(b_arr)) if matches!(op, Op::Add) => {
                        Rc::make_mut(&mut a_arr).extend(b_arr.iter().cloned());
                        Value::Array(a_arr)
                    }
                    (Value::String(a), b) | (b, Value::String(a)) if matches!(op, Op::Add) => {
                        let b_str = match b {
                            Value::Int(n) => PoolStr::new(&n.to_string()),
                            Value::Int64(n) => PoolStr::new(&n.to_string()),
                            Value::Float(f) => PoolStr::new(&f.to_string()),
                            Value::Bool(bl) => PoolStr::new(&bl.to_string()),
                            Value::String(s) => s,
                            Value::Range(lo, hi) => PoolStr::new(&format!("{}..{}", fmt_range_num(lo), fmt_range_num(hi))),
                            Value::Array(_) => PoolStr::new("[]"),
                            Value::ArenaPtr(_) => PoolStr::new("(ArenaPtr)"),
                            Value::RawPtr(_) => PoolStr::new("(RawPtr)"),
                            Value::ArrayElementPtr(n, i) => PoolStr::new(&format!("(ArrayElementPtr) &{}[{}]", n, i)),
                            Value::Class(c) => PoolStr::new(&format!("(class {})", c.name)),
                            Value::Instance(i) => PoolStr::new(&format!("(instance of {})", i.borrow().class.name)),
                            Value::Closure(_) => PoolStr::new("(closure)"),
                            Value::Generator(_) => PoolStr::new("(generator)"),
                            Value::Dict(map) => PoolStr::new(&fmt_value(&Value::Dict(map))),
                        };
                        Value::String(PoolStr::concat(&a, &b_str))
                    }
                    (Value::Bool(a), Value::Bool(b)) => match op {
                        Op::Equal => Value::Bool(a == b),
                        Op::Neq => Value::Bool(a != b),
                        Op::Or => Value::Bool(a || b),
                        Op::And => Value::Bool(a && b),
                        _ => Value::Int(0),
                    },
                    // 指针比较：== / !=（ArenaPtr/RawPtr 比地址，ArrayElementPtr 比名+下标）
                    (a, b) if matches!(a, Value::ArenaPtr(_) | Value::RawPtr(_) | Value::ArrayElementPtr(..))
                        || matches!(b, Value::ArenaPtr(_) | Value::RawPtr(_) | Value::ArrayElementPtr(..)) => {
                        match op {
                            Op::Equal => Value::Bool(ptr_value_equal(&a, &b)),
                            Op::Neq => Value::Bool(!ptr_value_equal(&a, &b)),
                            _ => self.panic_here("指针仅支持 == / != 比较"),
                        }
                    }
                    // 不同类型之间 == / != 直接返回 false
                    (_, _) => match op {
                        Op::Equal => Value::Bool(false),
                        Op::Neq => Value::Bool(true),
                        _ => self.panic_here( "运算符左右两侧类型不匹配"),
                    }
                }
            }
            Expr::Call(name, args, ln) => {
                // super(...)：在子类构造器内调用父类构造
                if name.as_str() == "super" {
                    let mut evaluated = Vec::new();
                    for a in args { evaluated.push(self.eval_expr(a)); }
                    let class_name = self.current_class.clone()
                        .unwrap_or_else(|| self.panic_at(*ln, 1, "super 只能在类方法内使用"));
                    let self_inst = self.current_self.clone()
                        .unwrap_or_else(|| self.panic_at(*ln, 1, "super 必须与 self 实例一起使用"));
                    let parent_def = {
                        let cur = self.classes.get(&class_name).cloned()
                            .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("未知类 {}", class_name)));
                        let pn = cur.superclass.clone()
                            .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("类 {} 没有父类，不能调用 super", class_name)));
                        self.classes.get(&pn).cloned()
                            .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("父类 {} 不存在", pn)))
                    };
                    let ctor = parent_def.constructor.clone()
                        .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("父类 {} 没有构造函数", parent_def.name)));
                    return self.call_func_with_self(ctor, self_inst, evaluated, *ln, Some(parent_def.name.clone()));
                }
                let segs: Vec<&str> = name.split("::").collect();
                let real_full = if !segs.is_empty() {
                    if let Some(real_mod) = self.mod_alias.get(segs[0]) {
                        let mut new_parts = vec![real_mod.as_str()];
                        new_parts.extend(&segs[1..]);
                        new_parts.join("::")
                    } else {
                        name.clone()
                    }
                } else {
                    name.clone()
                };
                let all_parts: Vec<&str> = real_full.split("::").collect();
                let func_name = all_parts.last().unwrap().to_string();

                if func_name == "printnowrap" || func_name == "printnw" {
                    if args.len() != 1 {
                        panic!("{} 仅接收 1 个参数", func_name);
                    }
                    let val = self.eval_expr(&args[0]);
                    fn print_val_no_lf(v: &Value) {
                        match v {
                            Value::Int(n) => print!("{}", n),
                            Value::Int64(n) => print!("{}", n),
                            Value::Float(f) => print!("{}", f),
                            Value::String(s) => print!("{}", s.as_str()),
                            Value::Bool(b) => print!("{}", b),
                            Value::Range(lo, hi) => print!("{}..{}", fmt_range_num(*lo), fmt_range_num(*hi)),
                            Value::Array(arr) => {
                                print!("[");
                                for (i, item) in arr.iter().enumerate() {
                                    if i > 0 { print!(", "); }
                                    print_val_no_lf(item);
                                }
                                print!("]");
                            }
                            Value::ArenaPtr(_) => print!("(ArenaPtr)"),
                            Value::RawPtr(_) => print!("(RawPtr)"),
                            Value::ArrayElementPtr(name, idx) => print!("(ArrayElementPtr) &{name}[{idx}]"),
                            Value::Class(class_rc) => print!("(class {})", class_rc.name),
                            Value::Instance(inst) => print!("(instance of {})", inst.borrow().class.name),
                            Value::Closure(_) => print!("(closure)"),
                        Value::Generator(_) => print!("(generator)"),
                            Value::Dict(map) => {
                                print!("{{");
                                let mut first = true;
                                for (k, v) in map.iter() {
                                    if !first { print!(", "); }
                                    print!("{}: ", k);
                                    print_val_no_lf(v);
                                    first = false;
                                }
                                print!("}}");
                            }
                        }
                    }
                    print_val_no_lf(&val);
                    std::io::stdout().flush().expect("刷新输出失败");
                    return Value::Int(0);
                }
                if func_name == "input" {
                    if !args.is_empty() {
                        panic!("input 不允许传入参数");
                    }
                    let mut buf = String::new();
                    let nread = std::io::stdin().read_line(&mut buf).expect("读取输入失败");
                    if nread == 0 {
                        // 输入流结束（EOF），返回特殊标记供脚本检测
                        return Value::String(PoolStr::new("__EOF__"));
                    }
                    let s = buf.trim_end_matches(&['\n', '\r'][..]).to_string();
                    return Value::String(PoolStr::new(&s));
                }
                if func_name == "len" {
                    if args.len() != 1 {
                        panic!("len() 只接收一个参数");
                    }
                    let arg_val = self.eval_expr(&args[0]);
                    let len = match arg_val {
                        Value::String(s) => s.len(),
                        Value::Array(arr) => arr.len(),
                        _ => self.panic_here("len() 仅支持字符串和数组"),
                    };
                    return Value::Int(len as i32);
                }
                if func_name == "now" {
                    if !args.is_empty() {
                        panic!("now 不接受参数");
                    }
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default();
                    return Value::Float(now.as_micros() as f64);
                }

                let mut evaluated_args = Vec::new();
                for a in args {
                    evaluated_args.push(self.eval_expr(a));
                }
                // 数组高阶函数（需闭包回调，走解释器方法而非静态内置）；仅裸调用生效，避免拦截 os::join 等模块函数
                if all_parts.len() == 1 && (func_name == "map" || func_name == "filter" || func_name == "reduce" || func_name == "join" || func_name == "sort") {
                    return self.call_array_hof(&func_name, evaluated_args, *ln);
                }
                #[cfg(feature = "gui")]
                {
                    // GUI 函数（脚本调用 gui_* 才初始化窗口）；仅裸调用生效
                    if all_parts.len() == 1 && (func_name == "gui_window" || func_name == "gui_icon" || func_name == "gui_button" || func_name == "gui_text" || func_name == "gui_heading" || func_name == "gui_separator" || func_name == "gui_spacer" || func_name == "gui_input" || func_name == "gui_input_var" || func_name == "gui_textarea" || func_name == "gui_output" || func_name == "gui_terminal" || func_name == "gui_checkbox" || func_name == "gui_slider" || func_name == "gui_row" || func_name == "gui_col" || func_name == "gui_topbar" || func_name == "gui_topbar_v" || func_name == "gui_sidebar" || func_name == "gui_bottombar" || func_name == "gui_run" || func_name == "gui_color_edit" || func_name == "gui_combo" || func_name == "gui_radio" || func_name == "gui_table" || func_name == "gui_multiselect" || func_name == "gui_tabs" || func_name == "gui_canvas" || func_name == "canvas_line" || func_name == "canvas_rect" || func_name == "canvas_circle") {
                        return self.call_gui(&func_name, evaluated_args, *ln);
                    }
                }
                // quit：通用退出（命令行退出进程 / GUI 关闭窗口），裸调用
                if all_parts.len() == 1 && func_name == "quit" {
                    return self.quit(evaluated_args, *ln);
                }
                // 先按完整模块名查内置（如 os::getcwd），再按短名查全局内置（如 substr）
                if let Some(&lib_fn) = self.lib_funcs.get(&real_full) {
                    return self.call_builtin(lib_fn, evaluated_args);
                }
                if let Some(&lib_fn) = self.lib_funcs.get(&func_name) {
                    return self.call_builtin(lib_fn, evaluated_args);
                }
                // 类静态方法：mymod.Person.hello() → real_full = 模块::类::方法
                if let Some(ci) = real_full.rfind("::") {
                    let (cls_path, meth) = real_full.split_at(ci);
                    let meth = &meth[2..];
                    if let Some(class_def) = self.classes.get(cls_path).cloned() {
                        let class_rc = Rc::new(class_def);
                        if let Some(func) = self.find_static(&class_rc, interner().get(meth)) {
                            if func.params.len() != evaluated_args.len() {
                                self.panic_at(*ln, 1, &format!("静态方法 {} 参数数量不匹配", real_full));
                            }
                            return self.call_func(func, evaluated_args);
                        }
                    }
                }
                // 类实例化：类名作为函数调用，如 Person("Tom", 18)
                if let Some(class_def) = self.classes.get(&real_full).cloned() {
                    return self.instantiate(class_def, evaluated_args, *ln);
                }
                // 闭包调用：变量值是 Closure -> 调用闭包（无论普通直存还是共享捕获形态）
                if self.env.contains(&real_full) {
                    if let Value::Closure(c) = self.env.get(&real_full) {
                        return self.call_closure(&c, evaluated_args, *ln);
                    }
                }
                let func = self.funcs.get(&real_full)
                    .cloned()
                    .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("函数或类不存在: {}", real_full)));

                self.call_func(func, evaluated_args)
            }
            Expr::MethodCall(obj_expr, method_name, args, ln) => {
                // 模块点号调用兼容：os.getcwd(...) → 查 lib_funcs 里 os::getcwd；先于对象求值
                if let Expr::Ident(name, _) = &**obj_expr {
                    let full = format!("{}::{}", interner().lookup(*name), interner().lookup(*method_name));
                    if let Some(&lib_fn) = self.lib_funcs.get(&full) {
                        let mut evaluated_args = Vec::new();
                        for a in args {
                            evaluated_args.push(self.eval_expr(a));
                        }
                        return self.call_builtin(lib_fn, evaluated_args);
                    }
                }
                let obj = self.eval_expr(obj_expr);
                let mut evaluated_args = Vec::new();
                for a in args {
                    evaluated_args.push(self.eval_expr(a));
                }
                match obj {
                    // 静态方法：类名.方法(args)
                    Value::Class(class_rc) => {
                        let mname = *method_name;
                        let func = self.find_static(&class_rc, mname)
                            .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("类 {} 没有静态方法 {}", class_rc.name, interner().lookup(mname))));
                        if func.params.len() != evaluated_args.len() {
                            self.panic_at(*ln, 1, &format!("静态方法 {} 参数数量不匹配", interner().lookup(mname)));
                        }
                        self.call_func(func, evaluated_args)
                    }
                    // 实例方法：实例.方法(args)，自动绑定 self（方法可不声明 self 形参）
                    Value::Instance(inst_rc) => {
                        let mname = *method_name;
                        let class_rc = inst_rc.borrow().class.clone();
                        let (func, decl_class) = self.find_method(&class_rc, mname)
                            .unwrap_or_else(|| self.panic_at(*ln, 1, &format!("实例没有方法 {}", interner().lookup(mname))));
                        self.call_func_with_self(func, Value::Instance(inst_rc.clone()), evaluated_args, *ln, Some(decl_class))
                    }
                    // 生成器：g.next() 推进到下一个 yield（done 后返回哨兵 0）
                    Value::Generator(g) => {
                        if *method_name == interner().get("next") {
                            match self.generator_next_val(g) {
                                Some(v) => v,
                                None => Value::Int(0),
                            }
                        } else {
                            self.panic_at(*ln, 1, &format!("生成器没有方法 {}", interner().lookup(*method_name)));
                        }
                    }
                    _ => self.panic_at(*ln, 1, &format!("对象没有方法 {}", interner().lookup(*method_name))),
                }
            }
            Expr::MemberAssign(obj_expr, member, value_expr, ln) => {
                let obj = self.eval_expr(obj_expr);
                let val = self.eval_expr(value_expr);
                match obj {
                    Value::Instance(inst) => {
                        let class_rc = inst.borrow().class.clone();
                        let layout = self.get_field_layout(&class_rc);
                        match layout.index.get(member) {
                            Some(&idx) => { inst.borrow_mut().fields[idx] = val.clone(); val }
                            None => self.panic_at(*ln, 1, &format!("实例没有字段 {}（类 {} 未声明）", interner().lookup(*member), class_rc.name)),
                        }
                    }
                    _ => self.panic_at(*ln, 1, "成员赋值左侧必须是实例"),
                }
            }
            Expr::PrivateMember(obj_expr, member, ln) => {
                let obj = self.eval_expr(obj_expr);
                match obj {
                    Value::Instance(inst) => {
                        let class_rc = inst.borrow().class.clone();
                        // 沿继承链找到声明该私有字段的类（可能是父类）
                        let mid = *member;
                        let decl = self.declaring_class_name(&class_rc, mid);
                        let decl_name = match decl {
                            Some(d) => d,
                            None => self.panic_at(*ln, 1, &format!("{} 不是 {} 的私有字段", interner().lookup(mid), class_rc.name)),
                        };
                        // 权限检查：仅声明它的类的方法内可访问
                        if self.current_class.as_deref() != Some(decl_name.as_str()) {
                            self.panic_at(*ln, 1, &format!("外部不能访问私有字段 #{}", interner().lookup(mid)));
                        }
                        let layout = self.get_field_layout(&class_rc);
                        match layout.index.get(&mid) {
                            Some(&idx) => inst.borrow().fields[idx].clone(),
                            None => self.panic_at(*ln, 1, &format!("实例没有字段 {}", interner().lookup(mid))),
                        }
                    }
                    _ => self.panic_at(*ln, 1, "私有字段访问左侧必须是实例"),
                }
            }
            Expr::PrivateMemberAssign(obj_expr, member, value_expr, ln) => {
                let obj = self.eval_expr(obj_expr);
                let val = self.eval_expr(value_expr);
                match obj {
                    Value::Instance(inst) => {
                        let class_rc = inst.borrow().class.clone();
                        let mid = *member;
                        let decl = self.declaring_class_name(&class_rc, mid);
                        let decl_name = match decl {
                            Some(d) => d,
                            None => self.panic_at(*ln, 1, &format!("{} 不是 {} 的私有字段", interner().lookup(mid), class_rc.name)),
                        };
                        if self.current_class.as_deref() != Some(decl_name.as_str()) {
                            self.panic_at(*ln, 1, &format!("外部不能访问私有字段 #{}", interner().lookup(mid)));
                        }
                        let layout = self.get_field_layout(&class_rc);
                        match layout.index.get(&mid) {
                            Some(&idx) => { inst.borrow_mut().fields[idx] = val.clone(); val }
                            None => self.panic_at(*ln, 1, &format!("实例没有字段 {}", interner().lookup(mid))),
                        }
                    }
                    _ => self.panic_at(*ln, 1, "私有字段赋值左侧必须是实例"),
                }
            }
            Expr::Assign(name, expr, ln) => {
                let id = *name;
                if self.env.is_const_id(id) {
                    self.panic_at(*ln, 1, &format!("编译/运行预检：常量 {} 禁止赋值", interner().lookup(id)));
                }
                if !self.env.contains_id(id) {
                    self.panic_at(*ln, 1, &format!("变量未定义: {}", interner().lookup(id)));
                }
                let val = self.eval_expr(expr);
                // 新增：运行时拦截两类指针互赋
                let old_val = self.env.get_id(id);
                let is_old_ptr = matches!(old_val, Value::ArenaPtr(_) | Value::RawPtr(_) | Value::ArrayElementPtr(_,_));
                let is_new_ptr = matches!(val, Value::ArenaPtr(_) | Value::RawPtr(_) | Value::ArrayElementPtr(_,_));

                if is_old_ptr && is_new_ptr {
                    match (&old_val, &val) {
                        (Value::ArenaPtr(_), Value::RawPtr(_)) |
                        (Value::RawPtr(_), Value::ArenaPtr(_)) |
                        (Value::ArrayElementPtr(_,_), Value::ArenaPtr(_)) |
                        (Value::ArrayElementPtr(_,_), Value::RawPtr(_)) |
                        (Value::ArenaPtr(_), Value::ArrayElementPtr(_,_)) |
                        (Value::RawPtr(_), Value::ArrayElementPtr(_,_))
                        => {
                            panic!("不同类型指针禁止互相赋值");
                        }
                        _ => {}
                    }
                }

                self.env.set_id(id, val.clone());
                val
            }
            // ========== 匿名函数字面量（闭包）：fn(params){body} ==========
            Expr::Lambda(func, _ln) => {
                // 扫描自由变量并捕获为共享单元（引用捕获）
                let free_vars = collect_closure_free(func);
                let mut captured = FastMap::new();
                for name in &free_vars {
                    if let Some(unit) = self.env.promote_to_shared_id(*name) {
                        captured.insert(*name, unit);
                    }
                }
                Value::Closure(Rc::new(ClosureData { func: func.clone(), captured }))
            }
            // ========== 字典字面量：{ "k": expr, ... } ==========
            Expr::Dict(items, _ln) => {
                let mut map = FastMap::new();
                for (k, vexpr) in items {
                    let v = self.eval_expr(vexpr);
                    map.insert(k.clone(), v);
                }
                Value::Dict(Rc::new(map))
            }
        }
    }
}

// ==============================
// 库加载 & 导入解析
// ==============================
// ============ std 标准库目录机制 ============
/// exe 所在目录下的 std 标准库根目录（xlang.exe / xlang_gui.exe 同级）
static STD_ROOT: OnceLock<PathBuf> = OnceLock::new();
fn std_root_dir() -> PathBuf {
    STD_ROOT.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("std")
    }).clone()
}

/// 加载一个 std 模块目录：init.x 必须最先，其余 .x 按名序合并为一个模块
fn load_std_dir_module(interp: &mut Interpreter, dir: &PathBuf, module_path: &str) {
    if !dir.join("init.x").exists() {
        script_panic("", 0, &format!("导入失败：模块目录 {} 缺少 init.x", dir.display()));
    }
    // 收集目录下所有 .x，init.x 必须第一个，其余按名称序
    let mut files: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".x"))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    if let Some(pos) = files.iter().position(|f| f == "init.x") {
        files.remove(pos);
        files.insert(0, "init.x".to_string());
    }

    let mut entry = ModuleCacheEntry { exports: Vec::new(), initialized: false };
    // 每个 .x 文件（除 init.x）额外建立一个子命名空间模块：std::math::<file>
    let mut sub_entries: Vec<(String, Vec<ModuleExportItem>)> = Vec::new();
    for f in files {
        let fp = dir.join(&f);
        let content = match std::fs::read_to_string(&fp) {
            Ok(c) => c,
            Err(_) => script_panic("", 0, &format!("导入失败：无法读取标准库文件 {}", fp.display())),
        };
        let tokenizer = Tokenizer::new(&content);
        let mut parser = Parser::new(tokenizer);
        let stmts = parser.parse_program();
        let mut sub = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::FnDef(name, params, body, _ret, line) => {
                    entry.exports.push(ModuleExportItem::Fn(name.clone(), params.clone(), body.clone(), line));
                    if f != "init.x" { sub.push(ModuleExportItem::Fn(name, params, body, line)); }
                }
                Stmt::Let(name, expr, line) => {
                    let n = interner().lookup(name);
                    entry.exports.push(ModuleExportItem::Let(n.clone(), expr.clone(), line));
                    if f != "init.x" { sub.push(ModuleExportItem::Let(n, expr, line)); }
                }
                Stmt::Const(name, expr, line) => {
                    let n = interner().lookup(name);
                    entry.exports.push(ModuleExportItem::Const(n.clone(), expr.clone(), line));
                    if f != "init.x" { sub.push(ModuleExportItem::Const(n, expr, line)); }
                }
                Stmt::Class(def, line) => {
                    entry.exports.push(ModuleExportItem::Class(def.name.clone(), def.clone(), line));
                    if f != "init.x" { sub.push(ModuleExportItem::Class(def.name.clone(), def, line)); }
                }
                _ => {}
            }
        }
        if f != "init.x" && !sub.is_empty() {
            let base = f.trim_end_matches(".x").to_string();
            sub_entries.push((base, sub));
        }
    }
    interp.module_cache.insert(module_path.to_string(), entry);
    for (base, sub) in sub_entries {
        let sp = format!("{}::{}", module_path, base);
        if !interp.module_cache.contains_key(&sp) {
            interp.module_cache.insert(sp, ModuleCacheEntry { exports: sub, initialized: false });
        }
    }
}

fn load_xlang_module(interp: &mut Interpreter, file_path: &str, module_path: &str) {
    // 如果已经缓存，直接返回，不再重复读取解析
    if interp.module_cache.contains_key(module_path) {
        return;
    }

    use std::fs;
    let content = match fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(_) => script_panic("", 0, &format!("导入失败：模块文件不存在 {}", file_path)),
    };
    let tokenizer = Tokenizer::new(&content);
    let mut parser = Parser::new(tokenizer);
    let stmts = parser.parse_program();

    let mut entry = ModuleCacheEntry {
        exports: Vec::new(),
        initialized: false,
    };

    for stmt in stmts {
        match stmt {
            Stmt::FnDef(name, params, body, _ret, line) => {
                entry.exports.push(ModuleExportItem::Fn(name, params, body, line));
            }
            Stmt::Let(name, expr, line) => {
                // 仅顶层let，加入模块导出
                entry.exports.push(ModuleExportItem::Let(interner().lookup(name), expr, line));
            }
            Stmt::Const(name, expr, line) => {
                // 仅顶层const，加入模块导出
                entry.exports.push(ModuleExportItem::Const(interner().lookup(name), expr, line));
            }
            Stmt::Class(def, line) => {
                // 类导出：注册为 模块路径::类名
                entry.exports.push(ModuleExportItem::Class(def.name.clone(), def, line));
            }
            // 其他语句（if/while/import等）模块顶层直接忽略，不执行
            _ => {}
        }
    }

    interp.module_cache.insert(module_path.to_string(), entry);
}

/// 初始化模块：执行顶层let/const，注册模块函数，注入当前解释器环境
fn init_module(interp: &mut Interpreter, module_path: &str) {
    // 1. TAKE：把entry从HashMap中移出来，不再是借用，拿到所有权
    let mut entry = interp.module_cache.remove(module_path)
        .expect(&format!("模块 {} 尚未加载", module_path));

    if entry.initialized {
        // 已经初始化，放回map直接返回，防止重复执行顶层代码
        interp.module_cache.insert(module_path.to_string(), entry);
        return;
    }

    // 遍历导出项：此时entry是局部所有权，不再借用interp，所以可以随便调用interp.eval_expr
    for export_item in entry.exports.drain(..) {
        match export_item {
            ModuleExportItem::Fn(name, params, body, _line) => {
                // 注册函数：模块路径::函数名
                let full_name = format!("{}::{}", module_path, name);
                interp.funcs.insert(full_name.clone(), Func { name: full_name, line: 0, params, body, ret_ty: None });
            }
            ModuleExportItem::Let(var_name, expr, _line) => {
                // 变量名字：模块路径::变量名
                let full_var = format!("{}::{}", module_path, var_name);
                // 安全调用eval_expr，不存在双重借用
                let val = interp.eval_expr(&expr);
                interp.env.define_var(full_var, val);
            }
            ModuleExportItem::Const(var_name, expr, _line) => {
                let full_var = format!("{}::{}", module_path, var_name);
                let val = interp.eval_expr(&expr);
                interp.env.define_const(full_var, val);
            }
            ModuleExportItem::Class(name, mut def, _line) => {
                // 注册为 模块路径::类名，使 mymod.Person / mymod::Person 可用
                let full_class = format!("{}::{}", module_path, name);
                // 父类若为同模块裸名（class A : B），重写为 模块::B
                if let Some(sc) = def.superclass.clone() {
                    if !sc.contains("::") {
                        def.superclass = Some(format!("{}::{}", module_path, sc));
                    }
                }
                // 类名同步重写为全名：super()/current_class 用该类名查类表才能命中
                def.name = full_class.clone();
                interp.classes.insert(full_class, def);
            }
        }
    }
    entry.initialized = true;

    // 处理完毕，把entry重新塞回module_cache HashMap
    interp.module_cache.insert(module_path.to_string(), entry);
}

fn load_lib_functions(file_path: &str) -> Vec<String> {
    use std::fs;
    let code = match fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let tokenizer = Tokenizer::new(&code);
    let mut parser = Parser::new(tokenizer);
    let stmts = parser.parse_program();
    let mut funcs = Vec::new();
    for stmt in stmts {
        if let Stmt::FnDef(name, _, _, _, _) = stmt {
            funcs.push(name);
        }
    }
    funcs
}

/// 注册 os:: 前缀的系统/文件/路径内置函数（功能对齐 Python os 模块）
/// 全部以 "os::函数名" 全名注册，脚本通过 os::xxx() 或 lib::os::xxx() 调用
fn register_os_funcs(interp: &mut Interpreter) {
    // 辅助：取字符串参数
    fn os_arg_str(args: &[Value], idx: usize, fname: &str) -> String {
        match &args[idx] {
            Value::String(s) => s.as_str().to_string(),
            _ => panic!("os::{} 第 {} 个参数必须是字符串", fname, idx + 1),
        }
    }
    fn os_str(s: String) -> Value {
        Value::String(PoolStr::new(&s))
    }
    fn os_arr(v: Vec<Value>) -> Value {
        Value::Array(Rc::new(v))
    }

    // 当前工作目录
    interp.lib_funcs.insert("os::getcwd".to_string(), |args| {
        if args.len() != 0 { panic!("os::getcwd 不需要参数"); }
        match std::env::current_dir() {
            Ok(p) => os_str(p.to_string_lossy().to_string()),
            Err(e) => panic!("os::getcwd 失败: {}", e),
        }
    });
    // 切换当前目录
    interp.lib_funcs.insert("os::chdir".to_string(), |args| {
        if args.len() != 1 { panic!("os::chdir 需要 1 个参数：目录路径"); }
        let p = os_arg_str(&args, 0, "chdir");
        match std::env::set_current_dir(&p) {
            Ok(_) => Value::Int(0),
            Err(e) => panic!("os::chdir 失败 ({}): {}", p, e),
        }
    });
    // 列出目录内容（排序后的文件名数组）
    interp.lib_funcs.insert("os::listdir".to_string(), |args| {
        if args.len() != 1 { panic!("os::listdir 需要 1 个参数：目录路径"); }
        let p = os_arg_str(&args, 0, "listdir");
        let mut names: Vec<String> = Vec::new();
        match std::fs::read_dir(&p) {
            Ok(rd) => {
                for entry in rd {
                    match entry {
                        Ok(e) => names.push(e.file_name().to_string_lossy().to_string()),
                        Err(e) => panic!("os::listdir 读取条目失败: {}", e),
                    }
                }
            }
            Err(e) => panic!("os::listdir 失败 ({}): {}", p, e),
        }
        names.sort();
        os_arr(names.into_iter().map(|n| os_str(n)).collect())
    });
    // 创建目录（非递归）
    interp.lib_funcs.insert("os::mkdir".to_string(), |args| {
        if args.len() != 1 { panic!("os::mkdir 需要 1 个参数：目录路径"); }
        let p = os_arg_str(&args, 0, "mkdir");
        match std::fs::create_dir(&p) {
            Ok(_) => Value::Int(0),
            Err(e) => panic!("os::mkdir 失败 ({}): {}", p, e),
        }
    });
    // 递归创建目录（父目录不存在时一并创建）
    interp.lib_funcs.insert("os::mkdirs".to_string(), |args| {
        if args.len() != 1 { panic!("os::mkdirs 需要 1 个参数：目录路径"); }
        let p = os_arg_str(&args, 0, "mkdirs");
        match std::fs::create_dir_all(&p) {
            Ok(_) => Value::Int(0),
            Err(e) => panic!("os::mkdirs 失败 ({}): {}", p, e),
        }
    });
    // 删除空目录
    interp.lib_funcs.insert("os::rmdir".to_string(), |args| {
        if args.len() != 1 { panic!("os::rmdir 需要 1 个参数：目录路径"); }
        let p = os_arg_str(&args, 0, "rmdir");
        match std::fs::remove_dir(&p) {
            Ok(_) => Value::Int(0),
            Err(e) => panic!("os::rmdir 失败 ({}): {}", p, e),
        }
    });
    // 删除文件
    interp.lib_funcs.insert("os::remove".to_string(), |args| {
        if args.len() != 1 { panic!("os::remove 需要 1 个参数：文件路径"); }
        let p = os_arg_str(&args, 0, "remove");
        match std::fs::remove_file(&p) {
            Ok(_) => Value::Int(0),
            Err(e) => panic!("os::remove 失败 ({}): {}", p, e),
        }
    });
    // 重命名/移动文件
    interp.lib_funcs.insert("os::rename".to_string(), |args| {
        if args.len() != 2 { panic!("os::rename 需要 2 个参数：旧路径，新路径"); }
        let a = os_arg_str(&args, 0, "rename");
        let b = os_arg_str(&args, 1, "rename");
        match std::fs::rename(&a, &b) {
            Ok(_) => Value::Int(0),
            Err(e) => panic!("os::rename 失败 ({} -> {}): {}", a, b, e),
        }
    });
    // 路径是否存在
    interp.lib_funcs.insert("os::exists".to_string(), |args| {
        if args.len() != 1 { panic!("os::exists 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "exists");
        Value::Bool(std::path::Path::new(&p).exists())
    });
    // 是否为目录
    interp.lib_funcs.insert("os::isdir".to_string(), |args| {
        if args.len() != 1 { panic!("os::isdir 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "isdir");
        Value::Bool(std::path::Path::new(&p).is_dir())
    });
    // 是否为文件
    interp.lib_funcs.insert("os::isfile".to_string(), |args| {
        if args.len() != 1 { panic!("os::isfile 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "isfile");
        Value::Bool(std::path::Path::new(&p).is_file())
    });
    // 路径拼接（可变参数，自动加平台分隔符）
    interp.lib_funcs.insert("os::join".to_string(), |args| {
        if args.is_empty() { panic!("os::join 至少需要 1 个参数"); }
        let mut buf = std::path::PathBuf::new();
        for i in 0..args.len() {
            let s = os_arg_str(&args, i, "join");
            buf.push(&s);
        }
        os_str(buf.to_string_lossy().to_string())
    });
    // 取文件名部分
    interp.lib_funcs.insert("os::basename".to_string(), |args| {
        if args.len() != 1 { panic!("os::basename 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "basename");
        let path = std::path::Path::new(&p);
        os_str(match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => String::new(),
        })
    });
    // 取目录部分
    interp.lib_funcs.insert("os::dirname".to_string(), |args| {
        if args.len() != 1 { panic!("os::dirname 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "dirname");
        let path = std::path::Path::new(&p);
        os_str(match path.parent() {
            Some(d) => d.to_string_lossy().to_string(),
            None => String::new(),
        })
    });
    // 拆分为 [目录, 文件名]（同 Python os.path.split）
    interp.lib_funcs.insert("os::split".to_string(), |args| {
        if args.len() != 1 { panic!("os::split 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "split");
        let path = std::path::Path::new(&p);
        let head = path.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
        let tail = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        os_arr(vec![os_str(head), os_str(tail)])
    });
    // 拆分为 [主名, 扩展名(含点)]（同 Python os.path.splitext）
    interp.lib_funcs.insert("os::splitext".to_string(), |args| {
        if args.len() != 1 { panic!("os::splitext 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "splitext");
        let path = std::path::Path::new(&p);
        let fname = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        match path.extension() {
            Some(ext) => {
                let ext_len = ext.to_string_lossy().len();
                let stem_len = fname.len() - ext_len - 1;
                let stem = fname[..stem_len].to_string();
                os_arr(vec![os_str(stem), os_str(format!(".{}", ext.to_string_lossy()))])
            }
            None => os_arr(vec![os_str(fname), os_str(String::new())]),
        }
    });
    // 绝对路径（不要求路径存在）
    interp.lib_funcs.insert("os::abspath".to_string(), |args| {
        if args.len() != 1 { panic!("os::abspath 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "abspath");
        match std::path::absolute(&p) {
            Ok(abs) => os_str(abs.to_string_lossy().to_string()),
            Err(e) => panic!("os::abspath 失败 ({}): {}", p, e),
        }
    });
    // 文件大小（字节）
    interp.lib_funcs.insert("os::filesize".to_string(), |args| {
        if args.len() != 1 { panic!("os::filesize 需要 1 个参数：路径"); }
        let p = os_arg_str(&args, 0, "filesize");
        match std::fs::metadata(&p) {
            Ok(md) => Value::Int(md.len() as i32),
            Err(e) => panic!("os::filesize 失败 ({}): {}", p, e),
        }
    });
    // 读取环境变量（不存在返回空字符串）
    interp.lib_funcs.insert("os::getenv".to_string(), |args| {
        if args.len() != 1 { panic!("os::getenv 需要 1 个参数：变量名"); }
        let n = os_arg_str(&args, 0, "getenv");
        os_str(std::env::var(&n).unwrap_or_default())
    });
    // 设置环境变量
    interp.lib_funcs.insert("os::setenv".to_string(), |args| {
        if args.len() != 2 { panic!("os::setenv 需要 2 个参数：变量名，值"); }
        let n = os_arg_str(&args, 0, "setenv");
        let v = os_arg_str(&args, 1, "setenv");
        // 单线程脚本解释器环境，设置环境变量安全
        unsafe { std::env::set_var(&n, &v); }
        Value::Int(0)
    });
    // 删除环境变量
    interp.lib_funcs.insert("os::unsetenv".to_string(), |args| {
        if args.len() != 1 { panic!("os::unsetenv 需要 1 个参数：变量名"); }
        let n = os_arg_str(&args, 0, "unsetenv");
        unsafe { std::env::remove_var(&n); }
        Value::Int(0)
    });
    // 路径分隔符
    interp.lib_funcs.insert("os::sep".to_string(), |args| {
        if args.len() != 0 { panic!("os::sep 不需要参数"); }
        os_str(std::path::MAIN_SEPARATOR.to_string())
    });
    // 平台名（nt / linux / macos ...）
    interp.lib_funcs.insert("os::name".to_string(), |args| {
        if args.len() != 0 { panic!("os::name 不需要参数"); }
        os_str(std::env::consts::OS.to_string())
    });
    // 执行系统命令（对标 Python os.system，走平台默认 shell，返回退出码）
    interp.lib_funcs.insert("os::system".to_string(), |args| {
        if args.len() != 1 { panic!("os::system 需要 1 个参数：命令字符串"); }
        let cmd = os_arg_str(&args, 0, "system");
        let status = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", &cmd]).status()
        } else {
            std::process::Command::new("sh").args(["-c", &cmd]).status()
        };
        match status {
            Ok(st) => Value::Int(st.code().unwrap_or(-1)),
            Err(e) => panic!("os::system 执行失败: {}", e),
        }
    });
    // 执行系统命令并捕获输出（返回 stdout+stderr 字符串；对标 Python subprocess）
    interp.lib_funcs.insert("os::exec".to_string(), |args| {
        if args.len() != 1 { panic!("os::exec 需要 1 个参数：命令字符串"); }
        let cmd = os_arg_str(&args, 0, "exec");
        let out = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", &cmd]).output()
        } else {
            std::process::Command::new("sh").args(["-c", &cmd]).output()
        };
        match out {
            Ok(o) => {
                let mut s = String::from_utf8_lossy(&o.stdout).to_string();
                if !o.stderr.is_empty() {
                    if !s.is_empty() { s.push('\n'); }
                    s.push_str(&String::from_utf8_lossy(&o.stderr));
                }
                os_str(s)
            }
            Err(e) => panic!("os::exec 执行失败: {}", e),
        }
    });
    // 执行命令并写入 stdin、捕获输出（供 IDE 直跑代码等场景）
    interp.lib_funcs.insert("os::exec_stdin".to_string(), |args| {
        if args.len() != 2 { panic!("os::exec_stdin 需要 2 个参数：命令，stdin 输入"); }
        let cmd = os_arg_str(&args, 0, "exec_stdin");
        let input = os_arg_str(&args, 1, "exec_stdin");
        use std::io::Write as _;
        let mut child = if cfg!(windows) {
            std::process::Command::new("cmd").args(["/C", &cmd])
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()
        } else {
            std::process::Command::new("sh").args(["-c", &cmd])
                .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()
        };
        match child {
            Ok(mut c) => {
                if let Some(mut si) = c.stdin.take() {
                    let _ = si.write_all(input.as_bytes());
                }
                match c.wait_with_output() {
                    Ok(o) => {
                        // 只返回 stdout：stderr 仅含提示性噪音（如“提示：当前已关闭…”），
                        // 错误信息已由 ariadne 渲染到 stdout，避免运行输出夹带提示
                        let s = String::from_utf8_lossy(&o.stdout).to_string();
                        os_str(s)
                    }
                    Err(e) => panic!("os::exec_stdin 等待失败: {}", e),
                }
            }
            Err(e) => panic!("os::exec_stdin 执行失败: {}", e),
        }
    });
    // 用户主目录
    interp.lib_funcs.insert("os::home".to_string(), |args| {
        if args.len() != 0 { panic!("os::home 不需要参数"); }
        match std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            Some(h) => os_str(h.to_string_lossy().to_string()),
            None => panic!("os::home 无法获取用户主目录"),
        }
    });
}

/// 路径 parts 逐段 join 到 base
fn join_parts(base: &PathBuf, parts: &[String]) -> PathBuf {
    let mut p = base.clone();
    for x in parts { p = p.join(x); }
    p
}

/// 解析并加载单个模块（无通配）。命名空间 = parts.join("::")
fn resolve_single_module(interp: &mut Interpreter, parts: &[String]) {
    if parts.is_empty() { return; }
    let full = parts.join("::");
    if interp.module_cache.contains_key(&full) {
        init_module(interp, &full);
        return;
    }
    // 标准库 std（显式 std 前缀）：目录模块 std/math（init.x）或单文件 std/math/x.x
    if parts[0] == "std" {
        let sub: Vec<String> = parts[1..].to_vec();
        let dir = join_parts(&std_root_dir(), &sub);
        if dir.join("init.x").exists() {
            load_std_dir_module(interp, &dir, &full);
            std_alias(interp, &full);
            init_module(interp, &full);
            init_std_alias(interp, &full);
            init_std_submodules(interp, &full);
            return;
        }
        let file = dir.with_extension("x");
        if Path::new(&file).exists() {
            load_xlang_module(interp, &file.to_string_lossy(), &full);
            std_alias(interp, &full);
            init_module(interp, &full);
            init_std_alias(interp, &full);
            return;
        }
        script_panic("", 0, &format!("导入失败：std 标准库中不存在模块 {}", full));
    }
    // lib 目录兼容（旧 import lib.os / "lib::os"）
    if parts[0] == "lib" {
        let file = path_to_file_path(parts);
        load_xlang_module(interp, &file, &full);
        init_module(interp, &full);
        return;
    }
    // 其他（无 std 前缀）：std 优先，std 无此模块再走自定义路径
    let dir = join_parts(&std_root_dir(), parts);
    if dir.join("init.x").exists() {
        load_std_dir_module(interp, &dir, &full);
        init_module(interp, &full);
        return;
    }
    let file = path_to_file_path(parts);
    load_xlang_module(interp, &file, &full);
    init_module(interp, &full);
}

/// 无前缀别名：把 std::math 的模块 entry 克隆到 math（使 math::add 与 math.add 均可用）。
/// 必须在 init_module 之前调用，否则 exports 已被 drain 为空。
fn std_alias(interp: &mut Interpreter, full: &str) {
    if let Some(al) = full.strip_prefix("std::") {
        if !interp.module_cache.contains_key(al) {
            let e = interp.module_cache.get(full).unwrap().clone();
            interp.module_cache.insert(al.to_string(), e);
        }
    }
}

/// 初始化无前缀别名模块（std::math 已 init 后，对其别名 math 执行 init）
fn init_std_alias(interp: &mut Interpreter, full: &str) {
    if let Some(al) = full.strip_prefix("std::") {
        init_module(interp, al);
    }
}

/// 初始化 std 目录模块的子命名空间（std::math::<file>），使 std.math.complex.cmod 等可访问
fn init_std_submodules(interp: &mut Interpreter, full: &str) {
    let prefix = format!("{}::", full);
    let depth = full.matches("::").count();
    let keys: Vec<String> = interp.module_cache.keys()
        .filter(|k| k.starts_with(&prefix) && k.matches("::").count() == depth + 1)
        .cloned().collect();
    for k in keys {
        init_module(interp, &k);
    }
}

/// 通配导入：把该目录本身作为模块合并加载（含 init.x 与全部 .x），再导入其子模块目录。
/// 例如 std.* → std 下所有顶层模块；std.math.* → std/math 合并为一个模块（std::math）。
fn resolve_wildcard(interp: &mut Interpreter, dir_parts: &[String]) {
    let base = if dir_parts.first().map(|x| x.as_str()) == Some("std") {
        join_parts(&std_root_dir(), &dir_parts[1..])
    } else if dir_parts.first().map(|x| x.as_str()) == Some("lib") {
        PathBuf::from(dir_parts.join("\\"))
    } else {
        join_parts(&std_root_dir(), dir_parts)
    };
    // 本目录是模块（含 init.x）：合并加载，命名空间 = dir_parts.join("::")
    if base.join("init.x").exists() {
        resolve_single_module(interp, dir_parts);
    }
    // 再导入子模块目录
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&base) {
        for e in rd.filter_map(|x| x.ok()) {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    dirs.sort();
    for p in dirs {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let mut np = dir_parts.to_vec();
        np.push(name);
        resolve_single_module(interp, &np);
    }
}

/// 命名导入：import mod.{a, b}; 加载 mod 后，把列出的符号复制到当前命名空间（无前缀直接可用）
fn resolve_named_import(interp: &mut Interpreter, parts: &[String], names: &[String]) {
    if parts.is_empty() { return; }
    if parts.iter().any(|x| x == "*") { script_panic("", 0, "命名导入不能与通配 * 混用"); }
    resolve_single_module(interp, parts);
    let full = parts.join("::");
    // 命名空间基：std 模块额外考虑无前缀别名（std::math -> math）
    let mut bases = vec![full.clone()];
    if let Some(al) = full.strip_prefix("std::") { bases.push(al.to_string()); }
    for sym in names {
        let mut found = false;
        for base in &bases {
            let key = format!("{}::{}", base, sym);
            if interp.classes.contains_key(&key) {
                if !interp.classes.contains_key(sym) {
                    let c = interp.classes.get(&key).unwrap().clone();
                    interp.classes.insert(sym.to_string(), c);
                }
                found = true; break;
            }
            if interp.funcs.contains_key(&key) {
                if !interp.funcs.contains_key(sym) {
                    let f = interp.funcs.get(&key).unwrap().clone();
                    interp.funcs.insert(sym.to_string(), f);
                }
                found = true; break;
            }
            if interp.env.contains(&key) {
                if !interp.env.contains(sym) {
                    let v = interp.env.get(&key);
                    interp.env.define_var(sym.to_string(), v);
                }
                found = true; break;
            }
        }
        if !found { script_panic("", 0, &format!("命名导入失败：模块 {} 中不存在符号 {}", full, sym)); }
    }
}

fn resolve_import(interp: &mut Interpreter, parts: &[String]) {
    if parts.is_empty() { return; }
    // 通配（*）：展开为多个单模块导入
    if let Some(wp) = parts.iter().position(|x| x == "*") {
        resolve_wildcard(interp, &parts[..wp]);
        return;
    }
    resolve_single_module(interp, parts);
}

/// 递归检查表达式，返回表达式类型 + 静态校验报错
fn check_expr(expr: &Expr, env: &mut TypeEnv, file: &str, line: u32) -> Type {
    match expr {
        Expr::Number(_, ln) => Type::Int,
        Expr::Int64(_, _ln) => Type::Int,
        Expr::Cast(e, _, _ln) => check_expr(e, env, file, line),
        Expr::Float(_, ln) => Type::Float,
        Expr::String(_, ln) | Expr::RawString(_, ln) => Type::String,
        Expr::Bool(_, ln) => Type::Bool,
        Expr::Lambda(_, _ln) => Type::Int,
        Expr::Dict(_, _ln) => Type::Int,
        Expr::Match(subject, arms, _ln) => {
            check_expr(subject, env, file, line);
            match arms.first() {
                Some((_, b)) => check_expr(b, env, file, line),
                None => Type::Int,
            }
        }
        Expr::Range(l, r, _ln) => {
            let _ = check_expr(l, env, file, line);
            let _ = check_expr(r, env, file, line);
            Type::Int
        }

        // 变量
        Expr::Ident(name, ln) => {
            // 静态检查拿不到mod_alias别名映射，含有::的别名形式跳过类型查询
            let name_str = interner().lookup(*name);
            if name_str.contains("::") {
                // 模块别名访问，静态检查放弃校验，返回Int占位，运行期做检查
                return Type::Int;
            }
            env.get_id(*name).unwrap_or_else(|| {
                script_panic(file, *ln, &format!("静态检查：变量 {} 未定义", name_str));
            })
        }

        // Arena 安全取地址
        Expr::AddrOf(inner, ln) => {
            let inner_ty = check_expr(inner, env, file, *ln);
            // 禁止对字符串取地址
            if inner_ty == Type::String {
                script_panic(file, *ln, "静态类型错误：禁止对堆字符串使用 & 取地址");
            }
            match inner.as_ref() {
                Expr::Index(_, _, _) => Type::ArrayElemPtr,
                Expr::Ident(_, _) => Type::ArenaPtr,
                _ => script_panic(file, *ln, "仅变量/数组元素支持&"),
            }
        }

        // 新增：Raw 裸指针取地址（静态类型 RawPtr）
        Expr::RawAddr(inner, ln) => {
            let inner_ty = check_expr(inner, env, file, *ln);
            if inner_ty == Type::String {
                script_panic(file, *ln, "静态类型错误：禁止对堆字符串使用 unsafe & 取地址");
            }
            Type::RawPtr
        }

        // 解引用
        Expr::Deref(inner, ln) => {
            let t = check_expr(inner, env, file, *ln);
            match t {
                Type::ArenaPtr | Type::RawPtr => Type::Int,
                Type::ArrayElemPtr => Type::Int,
                _ => script_panic(file, *ln, "静态类型错误：* 只能作用于指针类型"),
            }
        }

        Expr::DerefAssign(_, val, ln) => {
            check_expr(val, env, file, *ln)
        }

        // 数组
        Expr::Array(items, ln) => {
            Type::Array(Box::new(Type::Int))
        }

        // 数组下标
        Expr::Index(arr, idx, ln) => {
            let arr_ty = check_expr(arr, env, file, *ln);
            let idx_ty = check_expr(idx, env, file, *ln);
            if idx_ty != Type::Int {
                script_panic(file, *ln, "静态类型错误：数组索引必须是整数");
            }
            match arr_ty {
                Type::Array(inner) => *inner,
                _ => script_panic(file, *ln, "静态类型错误：[] 只能用于数组"),
            }
        }

        // 二元运算：拦截两类指针
        Expr::BinOp(lhs, op, rhs, ln) => {
            let l_ty = check_expr(lhs, env, file, *ln);
            let r_ty = check_expr(rhs, env, file, *ln);

            // 指针禁止参与算术/比较运算（or/and 逻辑运算除外）
            if !matches!(op, Op::Or | Op::And) {
                let l_ptr = matches!(l_ty, Type::ArenaPtr | Type::RawPtr | Type::ArrayElemPtr);
                let r_ptr = matches!(r_ty, Type::ArenaPtr | Type::RawPtr | Type::ArrayElemPtr);
                if l_ptr || r_ptr {
                    script_panic(file, *ln, "静态类型错误：指针禁止参与算术/比较运算");
                }
            }

            // or / and 只允许布尔/整数
            if matches!(op, Op::Or | Op::And) {
                if !matches!(l_ty, Type::Int | Type::Bool) || !matches!(r_ty, Type::Int | Type::Bool) {
                    script_panic(file, *ln, "静态类型错误：and/or 仅支持整数、布尔");
                }
            }

            if l_ty == Type::String || r_ty == Type::String {
                match op {
                    Op::Add => {
                        // 字符串拼接：两边都必须是String
                        if l_ty != Type::String || r_ty != Type::String {
                            script_panic(file, *ln, "静态类型错误：字符串拼接仅支持 string + string");
                        }
                        return Type::String;
                    }
                    Op::Equal | Op::Neq => {
                        // == != 字符串比较，两边都必须是String
                        if l_ty != Type::String || r_ty != Type::String {
                            script_panic(file, *ln, "静态类型错误：字符串只能和字符串用 == / != 对比");
                        }
                        return Type::Bool;
                    }
                    _ => {
                        // > < ?= 等其他运算符禁止字符串
                        script_panic(file, *ln, "静态类型错误：字符串只能使用 == / != 比较");
                    }
                }
            }

            // 数字运算
            if l_ty == Type::Float || r_ty == Type::Float {
                Type::Float
            } else {
                Type::Int
            }
        }

        // 赋值：禁止两类指针互转
        Expr::Assign(name, val, ln) => {
            let val_ty = check_expr(val, env, file, *ln);
            let var_ty = env.get_id(*name).unwrap_or_else(|| {
                script_panic(file, *ln, &format!("静态检查：变量 {} 未定义", interner().lookup(*name)));
            });

            // 严格拦截 ArenaPtr / RawPtr 互相赋值
            let is_ptr_a = var_ty == Type::ArenaPtr || var_ty == Type::RawPtr;
            let is_ptr_b = val_ty == Type::ArenaPtr || val_ty == Type::RawPtr;
            if is_ptr_a && is_ptr_b && var_ty != val_ty {
                script_panic(file, *ln, "静态类型错误：Arena 指针 与 Raw 裸指针 禁止互相赋值");
            }

            if var_ty != val_ty {
                script_panic(
                    file,
                    *ln,
                    &format!("静态类型错误：变量 {} 类型不匹配", name),
                );
            }
            val_ty
        }

        Expr::IndexAssign(arr, idx, val, ln) => {
            let _ = check_expr(arr, env, file, *ln);
            let _ = check_expr(idx, env, file, *ln);
            let _ = check_expr(val, env, file, *ln);
            Type::Int
        }

        Expr::Member(e, _member, ln) => {
            let _ty = check_expr(e, env, file, *ln);
            Type::Int
        }

        Expr::Call(_, args, ln) => {
            for a in args {
                let _ = check_expr(a, env, file, *ln);
            }
            Type::Int
        }
        Expr::MethodCall(obj, _, args, ln) => {
            let _ = check_expr(obj, env, file, *ln);
            for a in args {
                let _ = check_expr(a, env, file, *ln);
            }
            Type::Int
        }
        Expr::MemberAssign(obj, _, rhs, ln) => {
            let _ = check_expr(obj, env, file, *ln);
            let _ = check_expr(rhs, env, file, *ln);
            Type::Int
        }
        Expr::PrivateMember(obj, _, ln) => {
            let _ = check_expr(obj, env, file, *ln);
            Type::Int
        }
        Expr::PrivateMemberAssign(obj, _, rhs, ln) => {
            let _ = check_expr(obj, env, file, *ln);
            let _ = check_expr(rhs, env, file, *ln);
            Type::Int
        }
        Expr::Neg(inner, ln) => {
            let ty = check_expr(inner, env, file, *ln);
            if !matches!(ty, Type::Int | Type::Float) {
                script_panic(file, *ln, "静态类型错误：一元负号仅支持数字类型");
            }
            ty
        }
        Expr::Not(inner, _) => {
            let _ = check_expr(inner, env, file, 0);
            Type::Bool
        }
    }
}

/// 遍历整条语句，做静态类型检查
fn check_stmt(stmt: &Stmt, env: &mut TypeEnv, file: &str) {
    match stmt {
        Stmt::Break(ln) | Stmt::Continue(ln) => {}
        Stmt::Yield(expr, ln) => { let _ = check_expr(expr, env, file, *ln); }
        Stmt::Let(name, expr, ln) => {
            let ty = check_expr(expr, env, file, *ln);
            env.define_id(*name, ty);
        }
        Stmt::Const(name, expr, ln) => {
            let ty = check_expr(expr, env, file, *ln);
            env.define_id(*name, ty);
        }
        Stmt::Print(expr, ln) => {
            // let _ = check_expr(expr, env, file, *ln);
        }
        Stmt::Expr(expr, ln) => {
            let _ = check_expr(expr, env, file, *ln);
        }
        Stmt::If(cond, then_body, elif_branches, else_body, ln) => {
            let cond_ty = check_expr(cond, env, file, *ln);
            if cond_ty != Type::Bool && cond_ty != Type::Int {
                script_panic(file, *ln, "静态类型错误：if 条件必须为布尔/整数");
            }
            let mut inner_env = TypeEnv::nested(env.clone());
            for s in then_body {
                check_stmt(s, &mut inner_env, file);
            }
            for (c, body) in elif_branches {
                let _ = check_expr(c, &mut inner_env, file, *ln);
                for s in body {
                    check_stmt(s, &mut inner_env, file);
                }
            }
            for s in else_body {
                check_stmt(s, &mut inner_env, file);
            }
        }
        Stmt::While(cond, body, ln) => {
            let cond_ty = check_expr(cond, env, file, *ln);
            if cond_ty != Type::Bool && cond_ty != Type::Int {
                script_panic(file, *ln, "静态类型错误：while 条件必须为布尔/整数");
            }
            let mut inner_env = TypeEnv::nested(env.clone());
            for s in body {
                check_stmt(s, &mut inner_env, file);
            }
        }
        Stmt::For(var_name, start, end, body, ln) => {
            let _ = check_expr(start, env, file, *ln);
            let _ = check_expr(end, env, file, *ln);
            // 不新建嵌套环境，直接在父环境注册循环变量，和运行时作用域对齐
            env.define_id(*var_name, Type::Int);
            for s in body {
                check_stmt(s, env, file);
            }
        }
        Stmt::FnDef(_name, params, body, _ret, ln) => {
            let mut inner_env = TypeEnv::nested(env.clone());
            // 把函数所有形参注册到函数局部静态环境
            for param in params {
                inner_env.define(param.clone(), Type::Int);
            }
            for s in body {
                check_stmt(s, &mut inner_env, file);
            }
        }
        Stmt::ImportItem { lib_path: _, parts, alias, import_all: _, names: _, line: _ } => {}
        Stmt::Class(def, ln) => {
            // 构造函数
            if let Some(ctor) = &def.constructor {
                let mut cenv = TypeEnv::nested(env.clone());
                for p in &ctor.params { cenv.define(p.clone(), Type::Int); }
                for s in &ctor.body { check_stmt(s, &mut cenv, file); }
            }
            // 实例方法
            for (_, m) in &def.methods {
                let mut menv = TypeEnv::nested(env.clone());
                for p in &m.params { menv.define(p.clone(), Type::Int); }
                for s in &m.body { check_stmt(s, &mut menv, file); }
            }
            // 静态方法
            for (_, m) in &def.statics {
                let mut senv = TypeEnv::nested(env.clone());
                for p in &m.params { senv.define(p.clone(), Type::Int); }
                for s in &m.body { check_stmt(s, &mut senv, file); }
            }
            let _ = ln;
        }
        Stmt::UnsafeBlock(body, ln) => {
            let mut inner_env = TypeEnv::nested(env.clone());
            for s in body {
                check_stmt(s, &mut inner_env, file);
            }
        }
        Stmt::Return(opt_expr, ln) => {
            if let Some(e) = opt_expr {
                let _ = check_expr(e, env, file, *ln);
            }
        }
        Stmt::Throw(expr, ln) => {
            let _ = check_expr(expr, env, file, *ln);
        }
        Stmt::Try { body, catch_var, handler, line: _ } => {
            let mut inner_env = TypeEnv::nested(env.clone());
            for s in body { check_stmt(s, &mut inner_env, file); }
            if let Some(cvar) = catch_var {
                inner_env.define(cvar.clone(), Type::Int);
            }
            for s in handler { check_stmt(s, &mut inner_env, file); }
        }
    }
}

/// 入口：对整个程序 AST 执行全局静态类型检查
fn type_check_program(program: &[Stmt], file: &str) {
    let mut env = TypeEnv::new();
    for stmt in program {
        check_stmt(stmt, &mut env, file);
    }
}

/// 路径转文件路径：lib::demo -> lib\demo.x
fn path_to_file_path(parts: &[String]) -> String {
    let full = parts.join("::");
    // 规则1: 以 lib:: 开头 → lib 目录
    if full.starts_with("lib::") {
        let rest = &full[5..];
        return format!("lib\\{}.x", rest);
    }
    // 规则2: 普通名称 → 当前目录
    format!("{}.x", full)
}

/// 脚本报错：文件、行号、信息
fn script_panic(file: &str, line: u32, msg: &str) -> ! {
    script_panic_at(file, line, 1, msg)
}

/// 带列号的错误抛出入口(列号从1开始)
fn script_panic_at(file: &str, line: u32, col: u32, msg: &str) -> ! {
    std::panic::panic_any(ScriptError {
        file: file.to_string(),
        line,
        col,
        msg: msg.to_string(),
        secondary: Vec::new(),
    })
}

/// 带附加定位的错误入口：主标签 + 多个根源/连带标签
fn script_panic_at_multi(file: &str, line: u32, col: u32, msg: &str, secondary: Vec<(u32, u32, String)>) -> ! {
    std::panic::panic_any(ScriptError {
        file: file.to_string(),
        line,
        col,
        msg: msg.to_string(),
        secondary,
    })
}

// ==============================
// ariadne 错误渲染
// ==============================

/// 根据行列号计算源码中的字符偏移（从0开始，行/列从1开始）。
/// 找不到目标行时返回 None。
fn line_col_to_char_idx(code: &str, line: u32, col: u32) -> Option<usize> {
    let chars: Vec<char> = code.chars().collect();
    let target_line = line as usize; // 1-based
    let target_col = col as usize;   // 1-based

    let mut cur_line: usize = 1;     // 当前遍历到的行（1-based）
    let mut line_start: usize = 0;   // 当前行第一个内容字符的索引
    let mut i: usize = 0;

    // 第一遍：定位到目标行的起始索引
    while i < chars.len() {
        if cur_line == target_line {
            break;
        }
        if chars[i] == '\n' {
            cur_line += 1;
            line_start = i + 1;
        }
        i += 1;
    }
    if cur_line != target_line {
        return None; // 行号超出文件
    }

    // 第二遍：从 line_start 向后数，找到 target_col 对应的字符索引
    let mut j = line_start;
    let mut cur_col: usize = 1; // 当前字符在行内的列（1-based）
    while j < chars.len() && chars[j] != '\n' {
        if chars[j] == '\r' {
            break; // \r\n 行尾，停止
        }
        if cur_col == target_col {
            return Some(j);
        }
        j += 1;
        cur_col += 1;
    }
    // 列号超出该行实际字符数，退化为行尾前一个字符（或行首）
    if j > line_start {
        Some(j - 1)
    } else {
        Some(line_start)
    }
}

/// 用 ariadne 将脚本错误渲染到 stderr。
/// code：若为 None 则尝试从 err.file 读取源码文件。
fn render_script_error(err: &ScriptError, code: Option<&str>) {
    // 获取源码字符串与报告用的文件名
    let owned_code;
    let file_name: String;
    let fetched = match code {
        Some(c) => Some(c.to_string()),
        None => {
            if err.file.is_empty() || err.file == "未知文件" || err.file == "<repl>" {
                None
            } else {
                fs::read_to_string(&err.file).ok()
            }
        }
    };

    let src: &str = match fetched {
        Some(ref c) => { owned_code = c; file_name = err.file.clone(); owned_code.as_str() }
        None => {
            file_name = if err.file.is_empty() { "???.x".to_string() } else { err.file.clone() };
            ""
        }
    };

    let span_start = line_col_to_char_idx(src, err.line, err.col).unwrap_or(0);
    // 从错误列开始向后扫到行尾，作为标注宽度
    let chars: Vec<char> = src.chars().collect();
    let mut span_end = span_start;
    while span_end < chars.len() && chars[span_end] != '\n' {
        span_end += 1;
    }

    let mut builder = Report::build(ReportKind::Error, file_name.clone(), span_start);
    builder = builder.with_message(err.msg.clone());
    builder = builder.with_label(
        Label::new((file_name.clone(), span_start..span_end))
            .with_color(Color::Red)
            .with_message(err.msg.clone()),
    );
    // 附加定位：根源/连带位置渲染为黄色次要标签
    for (sln, scl, smsg) in &err.secondary {
        let s_start = line_col_to_char_idx(src, *sln, *scl).unwrap_or(0);
        let mut s_end = s_start;
        while s_end < chars.len() && chars[s_end] != '\n' { s_end += 1; }
        builder = builder.with_label(
            Label::new((file_name.clone(), s_start..s_end))
                .with_color(Color::Yellow)
                .with_message(smsg.clone()),
        );
    }

    let report = builder.finish();
    // 非终端（输出被捕获/管道）时全局禁用 ANSI 颜色，避免 \x1b[31m 乱码
    use std::io::IsTerminal as _;
    if !std::io::stdout().is_terminal() {
        yansi::disable();
    }
    let _ = report.print((file_name.clone(), Source::from(src)));
}


/// panic payload 统一渲染为 ariadne 风格错误。返回是否已处理 ScriptError
fn render_panic_payload(payload: &(dyn std::any::Any + Send), code: Option<&str>) -> bool {
    if let Some(err) = payload.downcast_ref::<ScriptError>() {
        render_script_error(err, code);
        true
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        eprintln!("错误: {}", s);
        true
    } else if let Some(s) = payload.downcast_ref::<String>() {
        eprintln!("错误: {}", s);
        true
    } else {
        eprintln!("错误: 程序发生未知错误");
        true
    }
}

// ==============================
// 程序入口 main（初始化/销毁全局内存池）
// ==============================
#[derive(Default, Clone)]
struct CliConfig {
    typecheck: bool,
    clean: bool,
}

/// 解析命令行参数，支持 --typecheck=xxx / --typecheck==xxx
fn parse_cli(args: &[String]) -> CliConfig {
    let mut cfg = CliConfig::default();
    // 从第3个参数开始解析附加参数（args[0]=exe, args[1]=run, args[2]=文件）
    for arg in args.iter().skip(3) {
        if let Some(inner) = arg.strip_prefix("--typecheck") {
            // 兼容 = 和 == 两种分隔符
            let val = inner.trim_start_matches('=');
            cfg.typecheck = val.eq_ignore_ascii_case("true");
        }
        if let Some(inner) = arg.strip_prefix("--clean") {
            let val = inner.trim_start_matches('=');
            cfg.clean = val.eq_ignore_ascii_case("true");
        }
    }
    cfg
}

fn start_repl(cli_cfg: CliConfig) {
    println!("X REPL  X <Version 0.3.0> \nInput \"exit\" for exit.");
    let mut code_buf = String::new();
    let mut line_num: u32 = 1;
    // 全局唯一解释器，变量持久保留
    let mut interp = Interpreter::new();
    // REPL 专用时间测量开关：run("file.x") 是否打印耗时（time on/off 切换）
    let mut show_time: bool = true;

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        if code_buf.is_empty() {
            print!(">>> ");
        } else {
            print!("... ");
        }
        io::stdout().flush().unwrap();

        let input = match lines.next() {
            Some(Ok(s)) => s,
            _ => break,
        };

        let trim = input.trim();
        // REPL 专用：时间测量开关（仅 REPL，可关闭）
        if trim == "time" || trim == "time on" {
            show_time = true;
            println!("时间测量: 已开启");
            continue;
        }
        if trim == "time off" {
            show_time = false;
            println!("时间测量: 已关闭");
            continue;
        }
        // REPL 专用：run("file.x") 会话内执行文件（复用常驻解释器，免进程冷启动）
        if let Some((fname, rargs)) = extract_run_call(trim) {
            repl_run_file(&fname, &mut interp, show_time, &rargs);
            continue;
        }
        if trim == "exit" || trim == "quit" {
            println!("退出 REPL");
            break;
        }

        code_buf.push_str(&input);
        code_buf.push('\n');

        let (complete, overflow) = check_code_state(&code_buf);

        // 右大括号过多，直接报错，清空缓存
        if overflow {
            eprintln!("语法错误：多余的右大括号 }}，括号不匹配");
            code_buf.clear();
            continue;
        }

        if complete {
            run_repl_code(&code_buf, line_num, cli_cfg.clone(), &mut interp);
            line_num += code_buf.lines().count() as u32;
            code_buf.clear();
        }
    }
}

/// 从 `run("file.x")` 或 `run('file.x')` 中提取文件名；非此类命令返回 None
/// 按顶层逗号拆分（忽略引号/括号内部的逗号）
fn split_top_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut depth: i32 = 0;
    let mut in_str = false;
    for c in s.chars() {
        match c {
            '"' | '\'' if depth == 0 => { in_str = !in_str; cur.push(c); }
            _ if in_str => cur.push(c),
            '(' | '[' | '{' => { depth += 1; cur.push(c); }
            ')' | ']' | '}' => { depth -= 1; cur.push(c); }
            ',' if depth == 0 => { parts.push(cur.trim().to_string()); cur = String::new(); }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { parts.push(cur.trim().to_string()); }
    parts
}

/// 解析 `run("file.x", arg1, arg2)`，返回 (文件名, 参数表达式列表)
fn extract_run_call(trim: &str) -> Option<(String, Vec<String>)> {
    let t = trim.trim();
    let inner = t.strip_prefix("run(")?.strip_suffix(')')?;
    let parts = split_top_args(inner);
    if parts.is_empty() { return None; }
    let fname_part = parts[0].trim();
    let c = fname_part.chars().next()?;
    if (c != '"' && c != '\'') || fname_part.len() < 2 { return None; }
    let fname = fname_part[c.len_utf8()..fname_part.len()-c.len_utf8()].to_string();
    let args = parts[1..].to_vec();
    Some((fname, args))
}

/// 用常驻解释器求值一个 run() 参数表达式，返回其 Value
/// 指针相等：ArenaPtr/RawPtr 比地址值；ArrayElementPtr 比"名+下标"；其余组合不相等
fn ptr_value_equal(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (ArenaPtr(x), ArenaPtr(y)) | (ArenaPtr(x), RawPtr(y)) | (RawPtr(x), ArenaPtr(y)) => x == y,
        (RawPtr(x), RawPtr(y)) => x == y,
        (ArrayElementPtr(n1, i1), ArrayElementPtr(n2, i2)) => n1 == n2 && i1 == i2,
        _ => false,
    }
}

fn eval_repl_arg(interp: &mut Interpreter, text: &str) -> Result<Value, String> {
    let t = Tokenizer::new(text);
    let mut p = Parser::new(t);
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let expr = p.parse_expr();
        interp.eval_expr(&expr)
    })) {
        Ok(v) => Ok(v),
        Err(_) => Err(format!("参数表达式解析/求值失败: {}", text)),
    }
}

/// REPL 会话内执行文件：复用常驻解释器，可选打印耗时
fn repl_run_file(fname: &str, interp: &mut Interpreter, show_time: bool, arg_exprs: &[String]) {
    // 求值脚本参数（可用 REPL 变量/表达式），转成字符串注入 args
    let mut sargs = Vec::with_capacity(arg_exprs.len());
    for a in arg_exprs {
        match eval_repl_arg(interp, a) {
            Ok(v) => sargs.push(fmt_value(&v)),
            Err(e) => { eprintln!("{}", e); return; }
        }
    }
    interp.set_script_args(sargs);
    let start = Instant::now();
    let code = match fs::read_to_string(fname) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("错误: 无法读取文件 {}: {}", fname, e);
            return;
        }
    };
    let t = Tokenizer::new(&code);
    let mut p = Parser::new(t);
    p.file = fname.to_string();
    let ast = p.parse_program();
    // 每次 run 独立执行：清空跨 run 累积的文件级状态（类定义/字段布局缓存/模块缓存/异常栈），
    // 避免 REPL 中反复 run 同一文件导致类字段布局或模块缓存污染（"实例没有字段 X"）
    interp.classes.clear();
    interp.field_cache.clear();
    interp.module_cache.clear();
    interp.ctx_stack.clear();
    interp.pending_throw = None;
    interp.current_class = None;
    interp.current_self = None;
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        interp.run(&ast);
    }));
    std::panic::set_hook(old_hook);
    if let Err(payload) = res {
        if payload.downcast_ref::<ThrowMarker>().is_some() {
            let v = interp.pending_throw.clone().unwrap_or(Value::Int(0));
            eprintln!("错误: 未捕获异常: {}", fmt_value(&v));
        } else {
            render_panic_payload(&*payload, None);
        }
    }
    if show_time {
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        println!("[run] {} 耗时: {:.3} ms", fname, ms);
    }
}

fn check_code_state(src: &str) -> (bool, bool) {
    let mut brace_depth = 0;
    let mut in_str = false;
    let mut in_block_comment = false;

    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_block_comment => in_str = !in_str,
            '/' if !in_str && !in_block_comment => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    in_block_comment = true;
                }
            }
            '*' if in_block_comment => {
                if chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
            }
            '{' if !in_str && !in_block_comment => brace_depth += 1,
            '}' if !in_str && !in_block_comment => brace_depth -= 1,
            _ => ()
        }
    }

    let overflow = brace_depth < 0;
    let complete = brace_depth == 0 && !in_str && !in_block_comment;
    (complete, overflow)
}

fn is_code_complete(src: &str) -> bool {
    let mut brace_depth = 0;
    let mut in_str = false;
    let mut in_block_comment = false;

    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_block_comment => in_str = !in_str,
            '/' if !in_str && !in_block_comment => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    in_block_comment = true;
                }
            }
            '*' if in_block_comment => {
                if chars.peek() == Some(&'/') {
                    chars.next();
                    in_block_comment = false;
                }
            }
            '{' if !in_str && !in_block_comment => brace_depth += 1,
            '}' if !in_str && !in_block_comment => brace_depth -= 1,
            _ => ()
        }
    }
    // 新增：brace_depth < 0 代表多余 }，代码不完整，不执行
    brace_depth == 0 && brace_depth >= 0 && !in_str && !in_block_comment
}

fn print_repl_val(v: &Value) {
    match v {
        Value::Int(n) => println!("{}", n),
        Value::Int64(n) => println!("{}", n),
        Value::Float(f) => println!("{}", f),
        Value::String(s) => println!("{}", s.as_str()),
        Value::Bool(b) => println!("{}", b),
        Value::Range(lo, hi) => println!("{}..{}", fmt_range_num(*lo), fmt_range_num(*hi)),
        Value::Array(arr) => {
            print!("[");
            for (i, item) in arr.iter().enumerate() {
                if i > 0 { print!(", "); }
                print_repl_val(item);
            }
            println!("]");
        }
        Value::ArenaPtr(_) => println!("(ArenaPtr)"),
        Value::RawPtr(_) => println!("(RawPtr)"),
        Value::ArrayElementPtr(name, idx) => println!("&{}[{}]", name, idx),
        Value::Class(class_rc) => println!("(class {})", class_rc.name),
        Value::Instance(inst) => println!("(instance of {})", inst.borrow().class.name),
        Value::Closure(_) => println!("(closure)"),
        Value::Generator(_) => println!("(generator)"),
        Value::Dict(map) => {
            print!("{{");
            let mut first = true;
            for (k, v) in map.iter() {
                if !first { print!(", "); }
                print!("{}: ", k);
                print_repl_val(v);
                first = false;
            }
            println!("}}");
        }
    }
}

fn run_repl_code(src: &str, base_line: u32, cli_cfg: CliConfig, interp: &mut Interpreter) {
    let filename = "<repl>".to_string();

    // 临时取出并替换全局panic钩子：REPL内部错误自行处理，避免全局钩子重复打印
    let old_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {})); // 空钩子，不输出任何内容

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let tokenizer = Tokenizer::new(src);
        let mut parser = Parser::new(tokenizer);
        parser.file = filename.clone();
        parser.line = parser.line + base_line - 1;
        let ast = parser.parse_program();

        if cli_cfg.typecheck {
            if !cli_cfg.clean {
                eprintln!("提示：当前已开启全局静态类型检查");
            }
            type_check_program(&ast, &filename);
        }

        for stmt in ast {
            match &stmt {
                Stmt::Expr(expr, _) => {
                    let val = interp.eval_expr(expr);
                    print_repl_val(&val);
                }
                _ => {
                    let _ = interp.exec_stmt(&stmt);
                }
            }
        }
        let _ = io::stdout().flush();
    }));

    // 执行完毕，恢复全局panic钩子，不影响run命令的错误输出
    panic::set_hook(old_hook);

    // 统一打印错误，全程只输出一次
    if let Err(panic_payload) = result {
        render_panic_payload(&*panic_payload, Some(src));
    }
}

fn mime_from_path(p: &Path) -> &'static str {
    match p.extension().and_then(|s| s.to_str()) {
        Some("html") | Some("htm") => "text/html;charset=utf-8",
        Some("css") => "text/css;charset=utf-8",
        Some("js") => "application/javascript;charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json;charset=utf-8",
        Some("txt") => "text/plain;charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn value_to_str_for_join(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.as_str().to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => {
            let mut s = String::from("[");
            for (i, it) in a.iter().enumerate() {
                if i > 0 { s.push_str(", "); }
                s.push_str(&value_to_str_for_join(it));
            }
            s.push(']');
            s
        }
        Value::Dict(d) => {
            let mut s = String::from("{");
            for (i, (k, v)) in d.iter().enumerate() {
                if i > 0 { s.push_str(", "); }
                s.push_str(k);
                s.push_str(": ");
                s.push_str(&value_to_str_for_join(v));
            }
            s.push('}');
            s
        }
        _ => "(value)".to_string(),
    }
}

/// sort：取数值
fn value_as_f64(v: &Value) -> f64 {
    match v {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    }
}

/// sort：数值比较（Int/Float），非数值按 0
fn value_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    value_as_f64(a).partial_cmp(&value_as_f64(b)).unwrap_or(std::cmp::Ordering::Equal)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    panic::set_hook(Box::new(|panic_info| {
        if let Some(err) = panic_info.payload().downcast_ref::<ScriptError>() {
            render_script_error(err, None);
        } else if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            eprintln!("错误:");
            eprintln!("    {}", s);
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            eprintln!("错误:");
            eprintln!("    {}", s);
        } else {
            eprintln!("文件: 未知  行号: 0");
            eprintln!("    程序发生未知错误");
        }
    }));

    mem_global_init();
    let total_start = Instant::now();

    let args: Vec<String> = env::args().collect();
    if args.contains(&"--version".to_string()) {
        println!("xlang 0.3.0");
        return Ok(());
    }
    let cli_cfg = parse_cli(&args);

    // ========== 核心修改：无参数直接进REPL ==========
    if args.len() == 1 {
        // 仅exe路径，无任何子命令，默认进入REPL
        start_repl(cli_cfg);
        mem_global_destroy();
        return Ok(());
    }

    let cmd = &args[1];

    match cmd.as_str() {
        "run" => {
            if args.len() < 3 {
                eprintln!("run 命令需要传入脚本文件，用法 xlang run test.x");
                mem_global_destroy();
                return Ok(());
            }
            let file = &args[2];
            let (code, disp) = if file == "-" {
                // 从 stdin 读取源码直接运行（IDE 直跑、不写临时文件）
                let mut buf = String::new();
                use std::io::Read as _;
                std::io::stdin().read_to_string(&mut buf)?;
                (buf, "<stdin>".to_string())
            } else {
                if !file.ends_with(".x") {
                    eprintln!("仅支持运行 .x 源码文件（或 - 从 stdin 读取）");
                    mem_global_destroy();
                    return Ok(());
                }
                (fs::read_to_string(file)?, file.clone())
            };
            let t = Tokenizer::new(&code);
            let mut p = Parser::new(t);
            p.file = disp;
            let ast = p.parse_program();

            if cli_cfg.typecheck {
                if !cli_cfg.clean {
                    eprintln!("提示：当前已开启全局静态类型检查");
                }
                type_check_program(&ast, &p.file);
            } else {
                if !cli_cfg.clean {
                    eprintln!("提示：当前已关闭全局静态类型检查，语法/unsafe/指针规则由解析器实时校验");
                }
            }

            let interp_start = Instant::now();
            let mut interp = Interpreter::new();
            interp.file = p.file.clone();
            interp.line = p.line;
            // 收集脚本参数（排除 -- 开头的 CLI 选项），供脚本内 args 数组使用
            let script_args: Vec<String> = args.iter().skip(3).filter(|a| !a.starts_with("--")).cloned().collect();
            interp.set_script_args(script_args);

            let old_hook = panic::take_hook();
            panic::set_hook(Box::new(|_| {}));
            let run_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                interp.run(&ast);
            }));
            panic::set_hook(old_hook);

            if let Err(panic_payload) = run_res {
                // 未捕获的 throw：从解释器取出异常值并渲染
                if panic_payload.downcast_ref::<ThrowMarker>().is_some() {
                    let v = interp.pending_throw.clone().unwrap_or(Value::Int(0));
                    eprintln!("错误: 未捕获异常: {}", fmt_value(&v));
                    mem_global_destroy();
                    return Ok(());
                }
                render_panic_payload(&*panic_payload, None);
                mem_global_destroy();
                return Ok(());
            }

            // interp.run(&ast);
            let interp_cost = interp_start.elapsed();
            io::stdout().flush()?;
        }
        "repl" => {
            start_repl(cli_cfg);
            return Ok(());
        }
        "-server" | "server" => {
            // xlang -server [port]
            let port = if args.len() >=3 {
                match args[2].parse::<u16>() {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("端口必须是数字，用法 xlang -server [port]，默认8000");
                        mem_global_destroy();
                        return Ok(());
                    }
                }
            } else {
                8000
            };
            // 直接调用内置http_server原生函数，host固定0.0.0.0（Windows局域网可访问）
            let mut interp = Interpreter::new();
            interp.lib_funcs["http_server"](vec![
                Value::String(PoolStr::new("0.0.0.0")),
                Value::Int(port as i32)
            ]);
            mem_global_destroy();
            return Ok(());
        }
        _ => {
            eprintln!("XLang 字节虚拟机 可用指令：");
            eprintln!("  xlang          直接启动REPL");
            eprintln!("  xlang repl     显式启动REPL");
            eprintln!("  xlang run test.x [--typecheck=true] [--clean=true]");
            eprintln!("  xlang -server [port]     启动静态HTTP文件服务器（类似python -m http.server）");
        }
    }

    let total_cost = total_start.elapsed();
    // println!("程序总耗时: {} 微秒", total_cost.as_micros());
    io::stdout().flush()?;

    // 销毁所有内存池
    let mut map = unsafe { map_write() };
    for (_, mut arena) in map.drain() {
        arena.destroy();
    }
    Ok(())
}
