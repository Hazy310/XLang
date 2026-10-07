// hash.x —— 哈希与编码：FNV-1a / CRC-32 / Base64 编解码（仅 ASCII 输入）
// 无位运算、无 +=；32 位运算一律用浮点 + 16 位拆分乘法 + 逐位异或模拟
// 模块内引用一律 std.data. 前缀（与 heap.x 一致）

// 可打印 ASCII 全表（空格到 ~，共 95 字符）：字符 <-> 字节互转
let ALPHA = " !\"#$$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{|}~";

// Base64 字母表（标准）
let B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

// 字符 -> ASCII 码：表内即其码；表外（index_of=-1）返回 31.0
fn char_value(ch) {
    index_of(std.data.ALPHA, ch) + 32.0
}

// 32 位按位异或（算术模拟；a、b 均为 <2^32 的整数浮点）
// 逐位检查：除以 2 的幂精确，取模 2 后 to_int 截掉小数即得该位
fn xor32(a, b) {
    let r = 0.0;
    let p = 1.0;
    for i in 0..32 {
        let ba = to_int((a / p) % 2.0);
        let bb = to_int((b / p) % 2.0);
        if ba != bb { r = r + p; }
        p = p * 2.0;
    }
    r
}

// (a*b) mod 2^32：16 位拆分，全部中间量 <2^33 逐项精确
fn mulmod32(a, b) {
    let a0 = a % 65536.0;
    let a1 = (a - a0) / 65536.0;
    let b0 = b % 65536.0;
    let b1 = (b - b0) / 65536.0;
    let lo = a0 * b0;
    let hi = a0 * b1 + a1 * b0;
    let hi16 = hi - (hi / 65536.0 - (hi / 65536.0 % 1.0)) * 65536.0;
    let t = lo + hi16 * 65536.0;
    if t >= 4294967296.0 { t - 4294967296.0 } else { t }
}

// FNV-1a 32 位：h = (h XOR byte) * prime mod 2^32
fn fnv1a(s) {
    let h = 2166136261.0;
    for i in 0..len(s) {
        let c = std.data.char_value(substr(s, i, 1));
        h = std.data.xor32(h, c);
        h = std.data.mulmod32(h, 16777619.0);
    }
    h
}

// CRC-32（IEEE 802.3，多项式反射 0xEDB88320=3988292384）
fn crc32(s) {
    let crc = 4294967295.0;
    for i in 0..len(s) {
        crc = std.data.xor32(crc, std.data.char_value(substr(s, i, 1)));
        for k in 0..8 {
            if crc % 2.0 == 1.0 {
                crc = std.data.xor32(crc / 2.0, 3988292384.0);
            } else {
                crc = crc / 2.0;
            }
        }
    }
    std.data.xor32(crc, 4294967295.0)
}

// Base64 编码（标准，按 3 字节一组，末尾补 '='）
fn base64_encode(s) {
    let out = "";
    let i = 0;
    let n = len(s);
    while i < n {
        let rem = n - i;
        if rem >= 3 {
            let b = std.data.char_value(substr(s, i, 1)) * 65536.0 + std.data.char_value(substr(s, i + 1, 1)) * 256.0 + std.data.char_value(substr(s, i + 2, 1));
            out = out + substr(std.data.B64, to_int(b / 262144.0), 1) + substr(std.data.B64, to_int(b / 4096.0 % 64.0), 1) + substr(std.data.B64, to_int(b / 64.0 % 64.0), 1) + substr(std.data.B64, to_int(b % 64.0), 1);
        } else {
            if rem == 2 {
                let b = std.data.char_value(substr(s, i, 1)) * 256.0 + std.data.char_value(substr(s, i + 1, 1));
                out = out + substr(std.data.B64, to_int(b / 1024.0), 1) + substr(std.data.B64, to_int(b / 16.0 % 64.0), 1) + substr(std.data.B64, to_int(b % 16.0 * 4.0), 1) + "=";
            } else {
                let b = std.data.char_value(substr(s, i, 1));
                out = out + substr(std.data.B64, to_int(b / 4.0), 1) + substr(std.data.B64, to_int(b % 4.0 * 16.0), 1) + "==";
            }
        }
        i = i + 3;
    }
    out
}

// Base64 解码（仅 ASCII 编码域；越界字节 substr(ALPHA,...) 返回空串）
fn base64_decode(s) {
    let n = len(s);
    if n == 0 { return ""; }
    // 统计末尾连续 '=' 个数
    let pad = 0;
    while pad < n and substr(s, n - 1 - pad, 1) == "=" {
        pad = pad + 1;
    }
    let groups = n / 4;
    let out = "";
    let g = 0;
    while g < groups {
        let off = g * 4;
        if g == groups - 1 {
            // 最后一组：按 pad 决定输出字节数
            if pad == 0 {
                let v0 = index_of(std.data.B64, substr(s, off, 1));
                let v1 = index_of(std.data.B64, substr(s, off + 1, 1));
                let v2 = index_of(std.data.B64, substr(s, off + 2, 1));
                let v3 = index_of(std.data.B64, substr(s, off + 3, 1));
                let b = v0 * 262144.0 + v1 * 4096.0 + v2 * 64.0 + v3;
                let c0 = to_int(b / 65536.0);
                let c1 = to_int(b / 256.0 % 256.0);
                let c2 = to_int(b % 256.0);
                out = out + substr(std.data.ALPHA, c0 - 32, 1) + substr(std.data.ALPHA, c1 - 32, 1) + substr(std.data.ALPHA, c2 - 32, 1);
            } else {
                if pad == 1 {
                    let v0 = index_of(std.data.B64, substr(s, off, 1));
                    let v1 = index_of(std.data.B64, substr(s, off + 1, 1));
                    let v2 = index_of(std.data.B64, substr(s, off + 2, 1));
                    let b = v0 * 4096.0 + v1 * 64.0 + v2;
                    let c0 = to_int(b / 1024.0);
                    let c1 = to_int(b / 4.0 % 256.0);
                    out = out + substr(std.data.ALPHA, c0 - 32, 1) + substr(std.data.ALPHA, c1 - 32, 1);
                } else {
                    let v0 = index_of(std.data.B64, substr(s, off, 1));
                    let v1 = index_of(std.data.B64, substr(s, off + 1, 1));
                    let b = v0 * 64.0 + v1;
                    let c0 = to_int(b / 16.0);
                    out = out + substr(std.data.ALPHA, c0 - 32, 1);
                }
            }
        } else {
            let v0 = index_of(std.data.B64, substr(s, off, 1));
            let v1 = index_of(std.data.B64, substr(s, off + 1, 1));
            let v2 = index_of(std.data.B64, substr(s, off + 2, 1));
            let v3 = index_of(std.data.B64, substr(s, off + 3, 1));
            let b = v0 * 262144.0 + v1 * 4096.0 + v2 * 64.0 + v3;
            let c0 = to_int(b / 65536.0);
            let c1 = to_int(b / 256.0 % 256.0);
            let c2 = to_int(b % 256.0);
            out = out + substr(std.data.ALPHA, c0 - 32, 1) + substr(std.data.ALPHA, c1 - 32, 1) + substr(std.data.ALPHA, c2 - 32, 1);
        }
        g = g + 1;
    }
    out
}
