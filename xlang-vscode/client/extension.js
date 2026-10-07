// XLang VSCode 扩展入口
const vscode = require('vscode');
const { execFile } = require('child_process');
const path = require('path');
const fs = require('fs');

function activate(context) {
    context.subscriptions.push(
        vscode.commands.registerCommand('xlang.run', () => runScript(false)),
        vscode.commands.registerCommand('xlang.runSilent', () => runScript(true)),
        vscode.commands.registerCommand('xlang.typecheck', () => runScript(false, true)),
        vscode.commands.registerCommand('xlang.version', showVersion),
        vscode.commands.registerCommand('xlang.listStd', listStd),
        vscode.languages.registerCompletionItemProvider('xlang', stdCompletion, '.', '"'),
        vscode.languages.registerCompletionItemProvider('xlang', localCompletion),
        vscode.languages.registerHoverProvider('xlang', stdHover),
        vscode.languages.registerDocumentSymbolProvider('xlang', docSymbols),
        vscode.languages.registerDocumentSemanticTokensProvider('xlang', semanticProvider, semanticLegend)
    );
}

// 从 dir 开始向上查找最近的 xlang.json 配置文件
function findProjectConfig(dir) {
    let d = dir;
    while (d) {
        const p = path.join(d, 'xlang.json');
        try {
            if (fs.existsSync(p)) {
                const raw = JSON.parse(fs.readFileSync(p, 'utf-8'));
                if (raw && typeof raw === 'object') return raw;
            }
        } catch (e) { /* 忽略解析错误，继续向上 */ }
        const up = path.dirname(d);
        if (up === d) break;
        d = up;
    }
    return null;
}

// 将配置值解析为 exe 绝对路径；值为文件夹时在其下找 xlang.exe
function resolveExecutable(raw, baseDir) {
    if (!raw || typeof raw !== 'string') return null;
    let exe = raw;
    if (!path.isAbsolute(exe)) exe = path.resolve(baseDir || '', exe);
    if (exe.toLowerCase().endsWith('.exe')) {
        if (fs.existsSync(exe)) return exe;
        return exe; // 存在性留给后续报错
    }
    // 视为文件夹：优先 xlang.exe，其次 xlang_gui.exe
    for (const n of ['xlang.exe', 'xlang_gui.exe']) {
        const c = path.join(exe, n);
        if (fs.existsSync(c)) return c;
    }
    return path.join(exe, 'xlang.exe'); // 文件夹不存在时给出可读路径报错
}

// 优先级：项目内 xlang.json > VSCode 设置（含 [xlang] 语言特定区段）> PATH
function getExecutable(file) {
    const fileDir = file ? path.dirname(file) : vscode.workspace.workspaceFolders?.[0]?.uri?.fsPath;
    if (fileDir) {
        const cfg = findProjectConfig(fileDir);
        if (cfg) {
            const r = resolveExecutable(cfg.executablePath || cfg.exe || cfg.xlang, fileDir);
            if (r) return r;
        }
    }
    // 带 resource 读取，使 [xlang] 语言特定区段的覆盖生效
    const resource = file ? vscode.Uri.file(file) : undefined;
    const p = vscode.workspace.getConfiguration('xlang', resource).get('executablePath');
    if (p) return p;
    return 'xlang.exe';
}

function activeFile() {
    const ed = vscode.window.activeTextEditor;
    if (!ed) { vscode.window.showWarningMessage('没有活动编辑器'); return null; }
    if (ed.document.languageId !== 'xlang') { vscode.window.showWarningMessage('当前文件不是 XLang 脚本'); return null; }
    const f = ed.document.uri.fsPath;
    if (!f.endsWith('.x')) { vscode.window.showWarningMessage('XLang 脚本扩展名应为 .x'); return null; }
    return f;
}

function runScript(silent, typecheck = false) {
    const file = activeFile();
    if (!file) return;
    const exe = getExecutable(file);
    const runInTerminal = vscode.workspace.getConfiguration('xlang').get('runInTerminal', true);
    const cleanOnRun = vscode.workspace.getConfiguration('xlang').get('cleanOnRun', true);
    const useClean = silent || cleanOnRun;
    const cwd = path.dirname(file);
    if (runInTerminal) {
        let term = vscode.window.activeTerminal;
        if (!term) term = vscode.window.createTerminal('XLang');
        term.show(true);
        let cmd = '"' + exe + '" run "' + file + '"';
        if (typecheck) cmd += ' --typecheck=true';
        else if (useClean) cmd += ' --clean=true';
        // PowerShell 里 "path" run 是非法语法，需调用运算符 &
        const isPS = /powershell|pwsh/i.test(vscode.env.shell || '');
        if (isPS) cmd = '& ' + cmd;
        term.sendText(cmd, true);
        return;
    }
    const args = ['run', file];
    if (typecheck) args.push('--typecheck=true');
    else if (useClean) args.push('--clean=true');
    vscode.window.withProgress(
        { location: vscode.ProgressLocation.Notification, title: 'XLang 运行中' },
        () => new Promise((resolve) => {
            execFile(exe, args, { cwd: cwd }, (err, stdout, stderr) => {
                const out = stdout.trim();
                if (out) vscode.window.showInformationMessage(out.split('\n').slice(0, 20).join('\n'));
                if (stderr) vscode.window.showErrorMessage(stderr.trim().split('\n').slice(0, 20).join('\n'));
                if (err && err.code === 'ENOENT') {
                    vscode.window.showErrorMessage('找不到 ' + exe + '。请在项目 xlang.json 的 executablePath 或设置 xlang.executablePath 中配置 xlang.exe 路径');
                }
                resolve();
            });
        })
    );
}

function showVersion() {
    const exe = getExecutable(undefined);
    execFile(exe, ['--version'], (err, stdout, stderr) => {
        if (err) {
            vscode.window.showErrorMessage('无法执行 ' + exe + '：请配置 xlang.executablePath');
            return;
        }
        vscode.window.showInformationMessage((stdout || stderr).trim());
    });
}

// 解析当前文档内的符号：函数名、类名、顶层（外部）变量、常量、import 模块
function scanDocSymbols(text) {
    const syms = { functions: [], classes: [], vars: [], consts: [] };
    const lines = text.split(/\r?\n/);
    let depth = 0;
    for (const line of lines) {
        let m;
        if ((m = line.match(/^\s*(?:static\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/))) syms.functions.push(m[1]);
        else if ((m = line.match(/^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)/))) syms.classes.push(m[1]);
        if (depth === 0) { // 仅顶层：外部变量 / 常量
            if ((m = line.match(/^\s*let\s+([A-Za-z_][A-Za-z0-9_]*)\s*=/))) syms.vars.push(m[1]);
            else if ((m = line.match(/^\s*const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=/))) syms.consts.push(m[1]);
        }
        depth += (line.match(/\{/g) || []).length - (line.match(/\}/g) || []).length;
        if (depth < 0) depth = 0;
    }
    for (const k of Object.keys(syms)) syms[k] = [...new Set(syms[k])];
    return syms;
}

// 标识符补全：文档内函数名 / 类名 / 顶层变量 / 常量；import 行交给 stdCompletion
const localCompletion = {
    provideCompletionItems(document, position) {
        const lineText = document.lineAt(position).text;
        const before = lineText.slice(0, position.character);
        if (/import\s+[\w.*]*$/.test(before)) {
            return stdCompletion.provideCompletionItems(document, position);
        }
        const syms = scanDocSymbols(document.getText());
        const kw = vscode.CompletionItemKind;
        const items = [];
        for (const f of syms.functions) items.push(new vscode.CompletionItem(f, kw.Function));
        for (const c of syms.classes) items.push(new vscode.CompletionItem(c, kw.Class));
        for (const v of syms.vars) items.push(new vscode.CompletionItem(v, kw.Variable));
        for (const c of syms.consts) items.push(new vscode.CompletionItem(c, kw.Constant));
        // 导入模块的导出符号（完整 模块.符号 形式）
        const mods = collectImports(document.uri.fsPath, document.getText());
        for (const [mod, exp] of mods) {
            for (const c of exp.classes) { const it = new vscode.CompletionItem(mod + '.' + c, kw.Class); it.insertText = mod + '.' + c; items.push(it); }
            for (const f of exp.functions) { const it = new vscode.CompletionItem(mod + '.' + f, kw.Function); it.insertText = mod + '.' + f; items.push(it); }
            for (const v of exp.vars) { const it = new vscode.CompletionItem(mod + '.' + v, kw.Variable); it.insertText = mod + '.' + v; items.push(it); }
            for (const c of exp.consts) { const it = new vscode.CompletionItem(mod + '.' + c, kw.Constant); it.insertText = mod + '.' + c; items.push(it); }
            for (const [cn, ms] of Object.entries(exp.classMethods || {})) {
                for (const m of ms) { const it = new vscode.CompletionItem(mod + '.' + cn + '.' + m, kw.Method); it.insertText = mod + '.' + cn + '.' + m; items.push(it); }
            }
        }
        return items;
    }
};

// 文档符号（大纲）：分解当前文件的函数名 / 类 / 顶层变量 / 常量 / import
const docSymbols = {
    provideDocumentSymbols(document) {
        const lines = document.getText().split(/\r?\n/);
        const symbols = [];
        let depth = 0;
        const rng = (i, start, len) => new vscode.Range(i, start, i, start + len);
        for (let i = 0; i < lines.length; i++) {
            const line = lines[i];
            const idx = (m) => line.indexOf(m[1]);
            let m;
            if ((m = line.match(/^\s*(?:static\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(/))) {
                symbols.push(new vscode.SymbolInformation(m[1], vscode.SymbolKind.Function, rng(i, idx(m), m[1].length), document.uri));
            } else if ((m = line.match(/^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)/))) {
                symbols.push(new vscode.SymbolInformation(m[1], vscode.SymbolKind.Class, rng(i, idx(m), m[1].length), document.uri));
            } else if (depth === 0 && (m = line.match(/^\s*let\s+([A-Za-z_][A-Za-z0-9_]*)\s*=/))) {
                symbols.push(new vscode.SymbolInformation(m[1], vscode.SymbolKind.Variable, rng(i, idx(m), m[1].length), document.uri));
            } else if (depth === 0 && (m = line.match(/^\s*const\s+([A-Za-z_][A-Za-z0-9_]*)\s*=/))) {
                symbols.push(new vscode.SymbolInformation(m[1], vscode.SymbolKind.Constant, rng(i, idx(m), m[1].length), document.uri));
            } else if (depth === 0 && (m = line.match(/^\s*import\s+([\w.*]+)/))) {
                symbols.push(new vscode.SymbolInformation(m[1], vscode.SymbolKind.Module, rng(i, idx(m), m[1].length), document.uri));
            }
            depth += (line.match(/\{/g) || []).length - (line.match(/\}/g) || []).length;
            if (depth < 0) depth = 0;
        }
        return symbols;
    }
};


// ---- 语义高亮：导入模块的库名/类名/函数名点链着色 ----
const semanticLegend = new vscode.SemanticTokensLegend(['class', 'function']);

// 读取模块文件，提取顶层 class / fn
function readModuleExports(fp) {
    const out = { classes: [], functions: [], vars: [], consts: [], classMethods: {} };
    try {
        const text = fs.readFileSync(fp, 'utf8');
        let depth = 0, curClass = null, classDepth = -1;
        for (const line of text.split(/\r?\n/)) {
            let m;
            // 类内方法（含 static fn）
            if (curClass && depth > classDepth) {
                if ((m = line.match(/^\s*(?:static\s+)?fn\s+([A-Za-z_]\w*)\s*\(/))) {
                    if (!out.classMethods[curClass]) out.classMethods[curClass] = [];
                    out.classMethods[curClass].push(m[1]);
                }
            }
            if (depth === 0) {
                if ((m = line.match(/^\s*let\s+([A-Za-z_]\w*)\s*=/))) out.vars.push(m[1]);
                else if ((m = line.match(/^\s*const\s+([A-Za-z_]\w*)\s*=/))) out.consts.push(m[1]);
            }
            if ((m = line.match(/^\s*class\s+([A-Za-z_]\w*)/))) {
                out.classes.push(m[1]);
                curClass = m[1];
                classDepth = depth;
                out.classMethods[m[1]] = [];
            } else if (!curClass && (m = line.match(/^\s*(?:static\s+)?fn\s+([A-Za-z_]\w*)\s*\(/))) {
                out.functions.push(m[1]);
            }
            depth += (line.match(/\{/g) || []).length - (line.match(/\}/g) || []).length;
            if (depth < 0) depth = 0;
            if (curClass && depth <= classDepth) curClass = null;
        }
    } catch (e) {}
    for (const k of ['classes', 'functions', 'vars', 'consts']) out[k] = [...new Set(out[k])];
    for (const k of Object.keys(out.classMethods)) out.classMethods[k] = [...new Set(out.classMethods[k])];
    return out;
}

// 合并 std 模块目录下所有 .x 的导出（平铺 + 文件级 fileExports）
function mergeDirExports(dir) {
    const merged = { classes: [], functions: [], vars: [], consts: [], classMethods: {}, fileExports: {} };
    try {
        const files = fs.readdirSync(dir).filter(f => f.endsWith('.x') && f !== 'init.x').sort();
        for (const f of files) {
            const e = readModuleExports(path.join(dir, f));
            for (const k of ['classes', 'functions', 'vars', 'consts']) merged[k] = merged[k].concat(e[k]);
            for (const cn of Object.keys(e.classMethods)) {
                if (!merged.classMethods[cn]) merged.classMethods[cn] = [];
                merged.classMethods[cn] = merged.classMethods[cn].concat(e.classMethods[cn]);
            }
            merged.fileExports[f.replace(/\.x$/, '')] = e;
        }
    } catch (e) {}
    for (const k of ['classes', 'functions', 'vars', 'consts']) merged[k] = [...new Set(merged[k])];
    for (const cn of Object.keys(merged.classMethods)) merged.classMethods[cn] = [...new Set(merged.classMethods[cn])];
    return merged;
}

// 收集文档内 import 的模块 -> 导出符号
function collectImports(file, text) {
    const mods = new Map();
    const exe = getExecutable(file);
    const exeStd = exe ? findStdRoot(exe) : null;
    const ws = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(file));
    const dir = path.dirname(file);
    for (const line of text.split(/\r?\n/)) {
        const m = line.match(/^\s*import\s+([\w.*]+)/);
        if (!m) continue;
        const pth = m[1].replace(/\*.*$/, '');
        const segs = pth.split(/[.::]+/).filter(Boolean);
        if (!segs.length) continue;
        const modName = (segs[0] === 'std' || segs[0] === 'lib') ? (segs[1] || segs[0]) : segs[0];
        if (!modName || mods.has(modName)) continue;
        let exp = null;
        if (segs[0] === 'std' && exeStd && segs[1]) {
            const d = path.join(exeStd, segs[1]);
            if (segs.length >= 3) {
                // 文件级：std.<mod>.<file>（如 std.math.trig）
                const cands = [path.join(d, segs.slice(2).join('/') + '.x')];
                for (const c of cands) if (fs.existsSync(c)) { exp = readModuleExports(c); break; }
                if (exp) { exp.fileExports = {}; exp.fileExports[segs.slice(2).join('/').replace(/\.x$/, '')] = exp; }
            } else {
                // 根目录文件模块（std/bigint.x）优先
                const fileMod = d + '.x';
                if (fs.existsSync(fileMod)) {
                    exp = readModuleExports(fileMod);
                    exp.fileExports = {};
                    exp.fileExports[segs[1]] = exp;
                } else {
                    // 目录级：std.<mod>  -> 合并目录全部文件（平铺 + fileExports）
                    exp = mergeDirExports(d);
                }
            }
        } else {
            const base = ws ? ws.uri.fsPath : dir;
            const cands = [path.join(dir, modName + '.x'), path.join(base, modName + '.x'), path.join(dir, modName, 'init.x'), path.join(base, modName, 'init.x')];
            for (const c of cands) if (fs.existsSync(c)) { exp = readModuleExports(c); break; }
        }
        if (exp) exp.isStd = (segs[0] === 'std');
        mods.set(modName, exp || { classes: [], functions: [], vars: [], consts: [], classMethods: {}, fileExports: {} });
    }
    return mods;
}

function encodeSemanticTokens(raw) {
    raw.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
    const data = [];
    let lastLine = 0, lastChar = 0;
    for (const [line, char, len, type, mods] of raw) {
        data.push(line - lastLine);
        data.push(line === lastLine ? char - lastChar : char);
        data.push(len);
        data.push(type);
        data.push(mods);
        lastLine = line;
        lastChar = char;
    }
    return new vscode.SemanticTokens(new Uint32Array(data));
}

const semanticProvider = {
    provideDocumentSemanticTokens(document) {
        const text = document.getText();
        const file = document.uri.fsPath;
        const lines = text.split(/\r?\n/);
        const mods = collectImports(file, text);
        if (mods.size === 0) return null;
        const raw = [];
        const push = (line, char, len, type) => raw.push([line, char, len, type === 'class' ? 0 : 1, 0]);
        for (let i = 0; i < lines.length; i++) {
            const line = lines[i];
            // 命名导入花括号：import mod.{a, b, ...} 内所有符号 -> class 色（任意数量）
            const brm = line.match(/import\s+[\w.*]+?\s*\{\s*([^}]*)\}/);
            if (brm) {
                const base = line.indexOf(brm[1]);
                const rsym = /[A-Za-z_][A-Za-z0-9_]*/g;
                let sm;
                while ((sm = rsym.exec(brm[1]))) push(i, base + sm.index, sm[0].length, 'class');
            }
            for (const [mod, exp] of mods) {
                const esc = mod.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
                // 库名 mymod.  -> class 色（对标 class）
                const reMod = new RegExp('(?<![\\w.])' + esc + '(?=\\.)', 'g');
                let mm;
                while ((mm = reMod.exec(line))) push(i, mm.index, mod.length, 'class');
                // 模块导出的类名 mymod.Person -> class 色
                for (const c of exp.classes) {
                    const reC = new RegExp('\\b' + esc + '\\.' + c + '\\b', 'g');
                    let m2;
                    while ((m2 = reC.exec(line))) push(i, m2.index + mod.length + 1, c.length, 'class');
                }
                // 模块导出的函数名：std 库 -> class 色（对标 class），自定义模块 -> function 色
                const fnColor = exp.isStd ? 'class' : 'function';
                for (const f of exp.functions) {
                    const reF = new RegExp('\\b' + esc + '\\.' + f + '\\b', 'g');
                    let m3;
                    while ((m3 = reF.exec(line))) push(i, m3.index + mod.length + 1, f.length, fnColor);
                }
                // 文件级 std 库函数：std.math.trig.sin -> class 色（trig.x 子文件）
                const fe = exp.fileExports;
                if (fe) {
                    for (const file of Object.keys(fe)) {
                        const fexp = fe[file];
                        for (const f of fexp.functions) {
                            const reF2 = new RegExp('\\b' + esc + '\\.' + file + '\\.' + f + '\\b', 'g');
                            let m4;
                            while ((m4 = reF2.exec(line))) push(i, m4.index + mod.length + 1 + file.length + 1, f.length, 'class');
                        }
                        for (const c of fexp.classes) {
                            const reC2 = new RegExp('\\b' + esc + '\\.' + file + '\\.' + c + '\\b', 'g');
                            let m5;
                            while ((m5 = reC2.exec(line))) push(i, m5.index + mod.length + 1 + file.length + 1, c.length, 'class');
                        }
                    }
                }
            }
        }
        if (raw.length === 0) return null;
        return encodeSemanticTokens(raw);
    }
};

function deactivate() {}
module.exports = { activate, deactivate };

// ==================== std 标准库自动识别 ====================
// 根据 xlang.exe 的目录，自动发现其同级 std/ 下的模块目录与 .x 文件（不手动添加）。

// 定位 exe 同级的 std 根目录
function findStdRoot(exe) {
    if (!exe || exe === 'xlang.exe') return null;
    try {
        const std = path.join(path.dirname(exe), 'std');
        return fs.existsSync(std) ? std : null;
    } catch (e) { return null; }
}

// 递归扫描 std 根目录：{ 模块名: { files: [...], hasInit: bool } }
function scanStd(stdRoot) {
    const tree = {};
    // std 根目录下的 .x 文件（如 bigint.x/json.x）作为根文件模块
    try {
        for (const f of fs.readdirSync(stdRoot)) {
            if (f.endsWith('.x') && f !== 'init.x') {
                const base = f.replace(/\.x$/, '');
                tree[base] = { files: [], hasInit: false, rootFile: f };
            }
        }
    } catch (e) {}
    try {
        for (const entry of fs.readdirSync(stdRoot, { withFileTypes: true })) {
            if (!entry.isDirectory()) continue;
            const modDir = path.join(stdRoot, entry.name);
            const files = [];
            let hasInit = false;
            for (const f of fs.readdirSync(modDir)) {
                if (f === 'init.x') { hasInit = true; }
                else if (f.endsWith('.x')) { files.push(f); }
            }
            files.sort();
            tree[entry.name] = { files, hasInit };
        }
    } catch (e) {}
    return tree;
}

function stdTreeText(tree) {
    const lines = [];
    for (const mod of Object.keys(tree).sort()) {
        const t = tree[mod];
        if (t.rootFile) { lines.push('std/' + t.rootFile); continue; }
        lines.push('std/' + mod + (t.hasInit ? '  (init.x)' : ''));
        for (const f of t.files) lines.push('    ' + f);
    }
    return lines.join('\n') || '（std 目录为空）';
}

// 命令：列出自动发现的 std 标准库模块与文件
function listStd() {
    const exe = getExecutable(undefined);
    const std = findStdRoot(exe);
    if (!std) {
        vscode.window.showInformationMessage('未找到 std 标准库目录（xlang.exe 同级应有 std/）。当前 exe：' + exe);
        return;
    }
    vscode.window.showInformationMessage('标准库 ' + std + '：\n' + stdTreeText(scanStd(std)));
}

// 扫描当前打开的文件夹（workspace）下的自定义模块：顶层 .x 文件 / 含 .x 的子目录 / 自定义 std
function scanWorkspaceModules(wsRoot) {
    const mods = new Set();
    try {
        for (const e of fs.readdirSync(wsRoot, { withFileTypes: true })) {
            const name = e.name;
            if (name === 'std' || name === 'lib' || name.startsWith('.')) continue;
            if (e.isDirectory()) {
                try {
                    const hasX = fs.readdirSync(path.join(wsRoot, name)).some(f => f.endsWith('.x'));
                    if (hasX) mods.add(name);
                } catch (e) {}
            } else if (name.endsWith('.x')) {
                mods.add(name.replace(/\.x$/, ''));
            }
        }
    } catch (e) {}
    return mods;
}

// 合并 exe 同级 std 与 workspace 自定义 std，得到全部标准库模块
function allStdModules(file) {
    const tree = {};
    const exeStd = findStdRoot(getExecutable(file));
    if (exeStd) Object.assign(tree, scanStd(exeStd));
    const ws = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(file));
    if (ws) {
        const wsStd = path.join(ws.uri.fsPath, 'std');
        if (fs.existsSync(wsStd)) {
            const w = scanStd(wsStd);
            for (const k of Object.keys(w)) if (!tree[k]) tree[k] = w[k];
        }
    }
    return tree;
}

// 命名导入符号：import mod.{ 花括号内提示该模块导出的类/函数/变量/常量
function moduleExportItems(modPath, file) {
    const kw = vscode.CompletionItemKind;
    const items = [];
    const add = (exp) => {
        for (const c of exp.classes) items.push(new vscode.CompletionItem(c, kw.Class));
        for (const f of exp.functions) items.push(new vscode.CompletionItem(f, kw.Function));
        for (const v of exp.vars) items.push(new vscode.CompletionItem(v, kw.Variable));
        for (const c of exp.consts) items.push(new vscode.CompletionItem(c, kw.Constant));
    };
    const segs = modPath.split('.').filter(Boolean);
    if (segs[0] === 'std') {
        const exe = getExecutable(file);
        const root = exe ? findStdRoot(exe) : null;
        if (root) {
            const dir = path.join(root, ...segs.slice(1));
            const rootFile = dir + '.x';
            if (fs.existsSync(rootFile)) { add(readModuleExports(rootFile)); return items; }
            try {
                const files = fs.readdirSync(dir).filter(x => x.endsWith('.x') && x !== 'init.x');
                for (const f of files) items.push(new vscode.CompletionItem(f.replace(/\.x$/, ''), kw.File));
                const merged = { classes: [], functions: [], vars: [], consts: [], classMethods: {} };
                for (const f of files) {
                    const e = readModuleExports(path.join(dir, f));
                    for (const k of ['classes', 'functions', 'vars', 'consts']) merged[k] = merged[k].concat(e[k]);
                }
                add(merged);
            } catch (e) {}
        }
    } else {
        const ws = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(file));
        const wsRoot = ws ? ws.uri.fsPath : path.dirname(file);
        for (const cand of [path.join(wsRoot, modPath + '.x'), path.join(path.dirname(file), modPath + '.x')]) {
            try { if (fs.existsSync(cand)) { add(readModuleExports(cand)); break; } } catch (e) {}
        }
    }
    return items;
}

// 普通代码 std 动态补全：std. / std.math. / std.math.<file>. 逐级提示下一级
function stdDynamicItems(prefix, file) {
    const kw = vscode.CompletionItemKind;
    const items = [];
    const segs = prefix.split('.');
    const exe = getExecutable(file);
    const root = exe ? findStdRoot(exe) : null;
    if (!root) return items;
    if (segs.length === 1) {
        for (const m of Object.keys(scanStd(root)).sort()) items.push(new vscode.CompletionItem(m, kw.Module));
        return items;
    }
    const dir = path.join(root, ...segs.slice(1));
    const filePath = dir + '.x';
    const add = (exp) => {
        for (const c of exp.classes) items.push(new vscode.CompletionItem(c, kw.Class));
        for (const f of exp.functions) items.push(new vscode.CompletionItem(f, kw.Function));
        for (const v of exp.vars) items.push(new vscode.CompletionItem(v, kw.Variable));
        for (const c of exp.consts) items.push(new vscode.CompletionItem(c, kw.Constant));
    };
    if (fs.existsSync(filePath)) { add(readModuleExports(filePath)); return items; }
    try {
        const files = fs.readdirSync(dir).filter(x => x.endsWith('.x') && x !== 'init.x');
        for (const f of files) items.push(new vscode.CompletionItem(f.replace(/\.x$/, ''), kw.File));
        const merged = { classes: [], functions: [], vars: [], consts: [], classMethods: {} };
        for (const f of files) { const e = readModuleExports(path.join(dir, f)); for (const k of ['classes','functions','vars','consts']) merged[k] = merged[k].concat(e[k]); }
        add(merged);
    } catch (e) {}
    return items;
}

// import 路径自动补全：输入到 . 立即提示下一级（全自动扫描，无硬编码）
const stdCompletion = {
    provideCompletionItems(document, position) {
        const lineText = document.lineAt(position).text;
        const before = lineText.slice(0, position.character);
        const file = document.uri.fsPath;
        // 命名导入花括号内：import mod.{  -> 提示该模块导出符号
        const brace = before.match(/import\s+([\w.*]+?)\s*\{\s*[^}]*$/);
        if (brace) {
            return moduleExportItems(brace[1].replace(/\.$/, ''), file);
        }
        if (!/import\s+[\w.*]*$/.test(before)) {
            const mods = collectImports(file, document.getText());
            const kw = vscode.CompletionItemKind;
            // 普通代码 std 动态补全：std. / std.math. / std.math.<file>.
            const stdm = before.match(/(std(?:\.[A-Za-z_]\w*)*)\.$/);
            if (stdm) return stdDynamicItems(stdm[1], file);
            // 两级：<模块>.<类>.  -> 该类的方法
            const two = before.match(/([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)\.$/);
            if (two) {
                const exp = mods.get(two[1]);
                const ms = exp && exp.classMethods ? exp.classMethods[two[2]] : null;
                if (ms) {
                    const its = [];
                    for (const m of ms) its.push(new vscode.CompletionItem(m, kw.Method));
                    return its;
                }
                return [];
            }
            // 单级：<模块>.  -> 模块导出的类/函数/变量/常量
            const dot = before.match(/([A-Za-z_][A-Za-z0-9_]*)\.$/);
            if (dot) {
                const exp = mods.get(dot[1]);
                if (exp) {
                    const its = [];
                    for (const c of exp.classes) its.push(new vscode.CompletionItem(c, kw.Class));
                    for (const f of exp.functions) its.push(new vscode.CompletionItem(f, kw.Function));
                    for (const v of exp.vars) its.push(new vscode.CompletionItem(v, kw.Variable));
                    for (const c of exp.consts) its.push(new vscode.CompletionItem(c, kw.Constant));
                    return its;
                }
            }
            return [];
        }
        const tree = allStdModules(file);
        const stdMods = Object.keys(tree).sort();
        const ws = vscode.workspace.getWorkspaceFolder(document.uri);
        const wsRoot = ws ? ws.uri.fsPath : path.dirname(file);
        const custom = [...scanWorkspaceModules(wsRoot)].sort();
        const afterImport = before.replace(/^.*import\s*/, '');
        // 尾点：正在 . 后输入（import std. 立即提示下一级）
        const endsDot = afterImport.endsWith('.');
        const segs = afterImport.split('.').filter(Boolean);
        const cur = endsDot ? '' : (segs.pop() || ''); // 正在输入的段（用于前缀过滤）
        const parent = segs;                            // 已确定路径（父级）
        const items = [];
        const filterBy = (list) => list.filter(m => m.toLowerCase().startsWith(cur.toLowerCase()) || !cur);
        const modOf = (name) => {
            if (tree[name]) return tree[name];
            const d = path.join(wsRoot, name);
            try { if (fs.existsSync(d)) return { files: fs.readdirSync(d).filter(f => f.endsWith('.x') && f !== 'init.x'), hasInit: false }; } catch (e) {}
            return null;
        };
        if (parent.length === 0) {
            // 顶层：std、lib + 全部模块（标准库 + 自定义库）
            for (const m of filterBy(['std', 'lib', ...new Set([...stdMods, ...custom])])) {
                items.push(new vscode.CompletionItem(m, vscode.CompletionItemKind.Module));
            }
        } else if (parent.length === 1 && parent[0] === 'std') {
            // std.<模块> 或 std. ：标准库 + workspace std + *
            for (const m of filterBy(stdMods)) items.push(new vscode.CompletionItem(m, vscode.CompletionItemKind.Module));
            items.push(new vscode.CompletionItem('*', vscode.CompletionItemKind.Keyword));
        } else if (parent.length === 1 && parent[0] === 'lib') {
            // lib.<文件>
            const libDir = path.join(wsRoot, 'lib');
            try {
                for (const f of filterBy(fs.readdirSync(libDir).filter(x => x.endsWith('.x')).map(x => x.replace(/\.x$/, '')))) {
                    items.push(new vscode.CompletionItem(f, vscode.CompletionItemKind.File));
                }
            } catch (e) {}
        } else if (parent.length === 2 && parent[0] === 'std') {
            // std.<模块>.<文件> 或 std.math.
            const mod = modOf(parent[1]);
            if (mod) {
                for (const f of filterBy(mod.files.map(x => x.replace(/\.x$/, '')))) {
                    items.push(new vscode.CompletionItem(f, vscode.CompletionItemKind.File));
                }
                items.push(new vscode.CompletionItem('*', vscode.CompletionItemKind.Keyword));
            }
        } else if (parent.length === 1) {
            // <自定义模块>.<文件>
            const mod = modOf(parent[0]);
            if (mod) {
                for (const f of filterBy(mod.files.map(x => x.replace(/\.x$/, '')))) {
                    items.push(new vscode.CompletionItem(f, vscode.CompletionItemKind.File));
                }
                items.push(new vscode.CompletionItem('*', vscode.CompletionItemKind.Keyword));
            }
        }
        return items;
    }
};

// 悬停：import std.math / import <自定义模块> 显示模块内容
const stdHover = {
    provideHover(document, position) {
        const lineText = document.lineAt(position).text;
        const m = lineText.match(/import\s+([\w.\*]+)/);
        if (!m) return null;
        const tree = allStdModules(document.uri.fsPath);
        const segs = m[1].replace(/\*/g, '').split('.').filter(Boolean);
        if (segs.length === 1 && (segs[0] === 'std' || segs[0] === 'lib')) {
            const lines = ['**' + segs[0] + '/**'];
            for (const mod of Object.keys(tree).sort()) lines.push('- ' + mod + '/');
            return new vscode.Hover(lines.join('\n'));
        }
        if (segs.length >= 2 && segs[0] === 'std') {
            const mod = tree[segs[1]];
            if (mod) {
                const lines = ['**std/' + segs[1] + '**'];
                if (mod.rootFile) { lines.push('- ' + mod.rootFile); return new vscode.Hover(lines.join('\n')); }
                if (mod.hasInit) lines.push('- init.x');
                for (const f of mod.files) lines.push('- ' + f);
                return new vscode.Hover(lines.join('\n'));
            }
        }
        return null;
    }
};
