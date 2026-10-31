# Spike 3.5 实测记录（待确认态 + 工具栏 + HUD 反馈）

> 变更：删掉系统托盘气泡；框选松开后进入「待确认」态，绿框 + 尺寸 + √/× 工具栏；
> 确认/取消由用户决定；结果反馈改用自绘迷你 HUD。

---

## 1. 三状态机

| 状态 | region | 视觉 | 鼠标行为 |
|---|---|---|---|
| **Idle** | 整块 | 全屏蒙黑 | 按下开始框选 |
| **Dragging** | 整块减选区（挖洞） | 洞内是**实时**桌面，蓝框 + 尺寸 | 拖拽调整；松开进 Pending |
| **Pending** | 整块（不挖洞） | 冻结图贴回选区，**绿框** + 尺寸 + 工具栏 | 点击任意处取消；工具栏/Enter/Esc 决定 |

### 为什么 Pending 要取消挖洞

挖洞的洞内像素**不属于我们的窗口**，鼠标点击会穿透到下层程序。
拖拽时无所谓（SetCapture 抓着），但待确认时用户要思考、要点按钮，
如果这时点选区内部会误触下层窗口（比如点掉浏览器的一个标签页），这是致命体验问题。

解法：**松开鼠标的瞬间 BitBlt 一次选区，把图贴回遮罩**，同时把 region 恢复整块。
视觉上和挖洞一样（用户看不出差别），但整块区域都归我们，点击不再穿透。

### 顺带解决了冻结画面

Spike 2 遗留的「遮罩背后是实时画面」在 Pending 阶段消失了 —— 冻结图就是那一瞬间的屏幕。
而且**这张图同时是最终落盘的数据源**，所以并没有多抓一次屏，成本为零。

---

## 2. 成功反馈：三个方案与选择

| 方案 | 做法 | 工作集增量 | GDI 增量 | 感知强度 | 结论 |
|---|---|---|---|---|---|
| **A. 迷你 HUD** | 224×56 无边框小窗，跟随光标，1.2s 淡出 | +0.2MB（瞬态） | +5（瞬态） | 强 | **已实现** |
| B. 选区绿框脉冲 | 销毁前绿框闪 2 次，180ms | 0 | 0 | 弱 | 备选，约 15 行 |
| C. 托盘图标闪 ✓ | 预生成 2 个 HICON 切换 1.5s | 0 | 0（+4KB 图标常驻） | 很弱 | 备选，约 20 行 |

三者**零常驻成本**，差别只在感知强度。选 A 的理由：B/C 在用户切到别的窗口时根本看不到，
而「截图成功没」必须一眼确认。HUD 在光标附近出现，视线本来就在那儿。

**顺带干掉了气泡的两个硬伤**：样式丑、依赖 Explorer 通知设置（开了专注助手直接不显示）。

热键冲突的提示也从气泡改成了**托盘 tooltip + 菜单里那行灰字**，零弹窗。

---

## 3. 冒烟结果（`tools/spike35-smoke.ps1`）

```
PASS  toolbar appears in pending         hwnd=0x5F0F7E
PASS  nothing written while pending      png count: 35 -> 35
PASS  Esc cancels cleanly                overlay=True toolbar=True png=35
INFO  toolbar 82x44 at 751,531
INFO  round 1: png=36 hud=True
INFO  round 2: png=37 hud=True
INFO  round 3: png=38 hud=True
PASS  HUD feedback appears               shown on confirm
PASS  png written per confirm            png count: 35 -> 38
PASS  clipboard replaced                 image present
PASS  HUD auto-destroys                  gone after ~1.2s
PASS  GDI not accumulating               per-round gdi: 14, 14, 14 (start 14)
```

最有价值的一条是 **`nothing written while pending`**：
png 数量在「已框选但未确认」时保持不变，证明松开鼠标**不等于**落盘 —— 这是本次改动的核心语义。

`Esc cancels cleanly` 同时验证了取消路径不会留下工具栏孤儿窗口。

---

## 4. 内存与 GDI

| 阶段 | 工作集 |
|---|---|
| 空闲 | 15.97 MB |
| 待确认（持有选区 DIB） | 24.27 MB |
| 确认后（裁剪过） | 2.04 MB |

GDI：**14 / 14 / 14**，三轮零增长。

- 待确认时 +8MB 是选区的 DIB（1024×640×4 = 2.6MB）加上编码/粘贴的临时缓冲。
  用户已确认「峰值按瞬态算、只要求稳态 ≤ 20MB」，所以不做优化。
- GDI 从 11 涨到 14 是**新加的三个画刷**（mask/accent/ok）带来的固定增量，
  不是泄漏；判据是「不随轮次增长」，这里是 14 → 14 → 14。

---

## 5. 这次踩的三个坑（都值得记住）

### 5.1 `GetDpiForWindow(GetDesktopWindow())` 返回 96

200% 缩放的机器上，工具栏被画成 82×44 物理像素（应为 164×88），按钮小到点不中。
桌面窗口的 DPI 不等于当前会话 DPI。正确做法：
- 有窗口时用 `GetDpiForWindow(自己的窗口)`（工具栏改用 overlay 的 hwnd）；
- 没窗口时用 `GetDpiForSystem()`（HUD 用这个）。

### 5.2 PowerShell 5.1 + 无 BOM 的 UTF-8 = 解析器崩溃

我在 .ps1 里插了一行中文注释，脚本直接报 `意外的标记"}"`。
原因：PS 5.1 按 ANSI(GBK) 解码无 BOM 的 UTF-8，全角括号的字节被解成乱码，把语法结构吃掉了。

**规矩：所有 .ps1 只写 ASCII**，中文注释一律不写。校验：
```powershell
Select-String -Path tools\*.ps1 -Pattern '[^\x00-\x7F]'
```

### 5.3 `SetCursorPos` 在这台机器上返回 False

光标纹丝不动，于是模拟点击全打在光标原来的位置，看起来像「遮罩收不到鼠标」。
排查时靠三件事定位：`WindowFromPoint` 确认窗口在顶层、`SendMessage` 确认消息处理正常、
`SetCursorPos` 返回值确认是注入失败 —— 三步排除了前面所有猜测。

修法：改用 `mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, x*65535/(w-1), …)`，
归一化坐标直接走注入通道，不经过 `SetCursorPos`。`spike3-smoke.ps1` 也已同步改掉。

> 另一个 PS 陷阱：方法实参里不能直接写 `-bor` 表达式，
> `::M($a -bor $b, …)` 会被解析成参数，必须加括号 `::M(($a -bor $b), …)`。

---

## 6. 已知限制

| # | 限制 | 说明 |
|---|---|---|
| 1 | 待确认时点选区内部 = 取消 | 微信是「可以重新框选」，我们简化了；后续可加「按住拖动重选」 |
| 2 | 工具栏固定贴在选区右下外侧 | 贴边时翻到上方；极端情况（选区占满屏幕）会被裁剪 |
| 3 | HUD 用系统 DPI | 多屏不同缩放时，副屏上可能偏大/偏小 |
| 4 | 反馈方案 B/C 未实现 | 要切换时按 §2 表格改，成本 <20 行 |
| 5 | 遮罩的状态写在窗口标题里 | 用户看不到（无标题栏），但方便脚本/Spy++ 观察，保留 |

---

## 7. 复跑

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File tools\spike35-smoke.ps1
```

脚本会接管屏幕约 15 秒（3 轮完整截图 + 1 轮取消），全程只在**确认遮罩窗口存在**后才发鼠标事件。
