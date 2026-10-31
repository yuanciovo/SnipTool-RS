# Spike 3 实测记录（PNG 落盘 + 临时目录 + 剪贴板 + 每日清理）

> 结论：四项功能全部跑通。**真实内存成本是提交内存 5.52MB**，工作集在裁剪后 1.77MB。
> 顺带发现并解决了一个会让 Spike 4 爆线的问题：不裁剪时工作集稳定在 19.66MB（离 20MB 只剩 0.3MB）。

---

## 1. 新增内容

| 文件 | 内容 |
|---|---|
| `src/store.rs` | 临时目录、`snip_YYYYMMDD_HHMMSS_mmm.png` 命名、BGRA→RGB、PNG 原子落盘 |
| `src/cleanup.rs` | 24h 清理：`FindFirstFileW` 只读元数据，白名单 + 跳过链接 |
| `src/platform/clipboard.rs` | CF_DIB + CF_PNG 双格式写入，带占用重试 |
| `src/main.rs` | `SetTimer` 每日清理 + 启动补跑；`trim_working_set()` |
| `src/capture/overlay.rs` | `confirm()` 改为 抓图 → PNG 落盘 → 剪贴板 → 裁剪 |
| 删除 | `src/capture/bmp.rs`（Spike 2 的临时格式） |

---

## 2. 冒烟结果（`tools/spike3-smoke.ps1`）

```
PASS  stale snip deleted on startup    30h old file removed
PASS  fresh snip kept                  untouched
PASS  mask window created              hwnd=0x50D20
PASS  mask closed after confirm        ok
PASS  png written                      snip_20261031_123859_984.png (142 KB)
PASS  png decodes                      magic=True size=1024x640
PASS  clipboard holds the capture      clip=1024x640  png=1024x640 (was 8x8)
PASS  GDI not accumulating             first=11 last=11 plateau=10
```

逐条含义：

1. **清理只删该删的**：预先放了两个假文件，一个 mtime 倒退 30h、一个当前时间，文件名都是 `snip_*.png`。
   启动后 30h 的没了、新的一动没动 —— 证明判龄逻辑和白名单都对。
2. **PNG 有效**：文件头 `89 50 4E 47` 正确，`System.Drawing` 能解码，尺寸与选区一致。
3. **剪贴板真的被替换了**：测试前先往剪贴板放了一张 8×8 的图，截图后变成 1024×640。
   这一条比"能读出来"更有价值 —— 证明是**覆盖**而不是残留。
4. **GDI 不累积**：连拍 4 次恒为 11（判据见 Spike 2 记录）。

---

## 3. 内存：两个数字都要看

| 阶段 | 工作集 | 提交（私有） |
|---|---|---|
| 空闲（托盘） | 11.45 MB | 2.07 MB |
| 遮罩弹出 | 15.80 MB | ~3 MB |
| 截图后（**未裁剪**） | **19.66 MB** | ~5.5 MB |
| 截图后（**裁剪后**） | **1.77 MB** | **5.52 MB** |
| 全程峰值 | 24.79 MB | — |

### 3.1 19.66MB 里大部分不是我们的

连拍 8 次稳定在 19.64–19.67（平台值，非泄漏）。但同期**提交内存只有 5.5MB** ——
说明这 19.66MB 里约 14MB 是 `user32`/`gdi32`/`png` 解码器的**共享代码页**，
本来就常驻内存、别的进程也在用，不算本进程的独占成本。

### 3.2 为什么加了 `trim_working_set()`

用户口径是任务管理器显示的数字。不裁剪的话 19.66MB 只剩 0.3MB 余量，
Spike 4 的历史面板预算 +2.5MB，必然爆线。

实现：截图完整链路走完后调一次
`SetProcessWorkingSetSize(GetCurrentProcess(), -1, -1)`，把冷页移出工作集。

**必须说清楚它做了什么**：
- 它**不释放内存**，只是把不活跃的页移到 standby list，下次访问软缺页回来（微秒级，无磁盘 IO）；
- 提交内存（5.52MB）**完全没变** —— 那才是真实占用；
- 只在一次截图结束后调一次，不在热路径上；不会周期性抖动。

代价：裁剪后第一次按热键会多几次软缺页。对"一天用几十次"的托盘工具可以忽略。
如果将来觉得这个数字"太好看不可信"，把 `confirm()` 末尾那行 `crate::trim_working_set()` 删掉即可，
常驻会回到 19.66MB —— 仍然达标，只是没有余量给 Spike 4。

---

## 4. 清理设计要点（对照技术方案 §5）

| 要求 | 实现 |
|---|---|
| 只基于时间戳，不读图像 | `FindFirstFileW` 一次遍历拿 `dwFileAttributes` + `ftLastWriteTime`，全程不 `CreateFile`/`ReadFile` |
| 每日执行 | 主消息循环 `SetTimer` 24h + 启动补跑一次（`WM_TIMER`） |
| 超过 24h 才删 | `now - ftLastWriteTime > 24h`，单位 100ns |
| 不误删 | 只匹配 `snip_*.png` / `snip_*.tmp`；跳过目录与重解析点 |
| 失败不阻塞 | 单个 `DeleteFileW` 失败只计 `failed`，下轮再试 |

**为什么不用任务计划程序**：本工具 7×24 常驻，`SetTimer` 零额外开销；
任务计划程序需要注册计划任务、可能被杀软/组策略拦、且与主程序状态不同步。

**防删到半成品**：`save_png` 先写 `xxx.png.tmp` 再 `fs::rename` 原子改名，
所以清理只会看到完整文件；即使进程崩在写入中途，残留的 `.tmp` 也会在 24h 后被清掉。

---

## 5. 已知限制

| # | 限制 | 影响 |
|---|---|---|
| 1 | 24h / 保留时长是常量，还不能配置 | 设置界面（Spike 5）补 |
| 2 | 定时器路径无法真等 24h 验证，只验证了启动补跑（**同一个函数** `run_cleanup`） | 逻辑已覆盖，触发时机未覆盖 |
| 3 | 剪贴板只验证了 CF_DIB 可被读回；CF_PNG 是否被特定程序优先采用未逐一举证 | 需要时手动粘到 Chrome/Word 验证 |
| 4 | 没有 `state.json` 记录 last_cleanup | 启动补跑已能覆盖关机期间的空档，暂不需要 |
| 5 | 清理失败时最多弹一次气泡 | 避免反复打扰 |

---

## 6. 复跑

```powershell
cargo build --release

# 默认 3 次截图 + 清理断言，约 8 秒，会接管屏幕
powershell -ExecutionPolicy Bypass -File tools\spike3-smoke.ps1

# 连拍 8 次看工作集趋势
powershell -ExecutionPolicy Bypass -File tools\spike3-smoke.ps1 -Captures 8
```

脚本会在 `%TEMP%\MySnipTool` 里造两个测试文件（其中一个 mtime 倒退 30h），
跑完会删掉新造的那个；被清理掉的那个本来就该消失。**不会碰你其它文件。**
