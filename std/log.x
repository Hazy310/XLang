// log.x —— 日志（std 根目录模块）
// 用法：import std.log;
//   std.log.set_level(lv)  // 0=debug 1=info 2=warn 3=error，默认 1
//   std.log.set_file(path) // 之后日志同时写入文件
//   std.log.debug/info/warn/error(msg)
// 级别低于当前设置的日志不输出

let level = 1;
let logfile = "";

fn set_level(lv) {
    std.log.level = lv;
}

fn set_file(path) {
    std.log.logfile = path;
}

fn out(lvname, lvnum, msg) {
    if lvnum >= std.log.level {
        let line = "[" + lvname + "] " + msg;
        print(line);
        if std.log.logfile != "" {
            append_file(std.log.logfile, line + "\n");
        }
    }
}

fn debug(msg) { std.log.out("DEBUG", 0, msg); }
fn info(msg)  { std.log.out("INFO", 1, msg); }
fn warn(msg)  { std.log.out("WARN", 2, msg); }
fn error(msg) { std.log.out("ERROR", 3, msg); }
