# Spike 2 实测记录（框选遮罩 + 全局热键）

> 结论先行：**「不预抓全屏、region 挖洞、确认后只抓选区」这条路成立。**
> 遮罩弹出只花 +4.44MB，没有 33MB 全屏 DIB；GDI 连拍 6 次零累积。
> 唯一的代价是**抓图瞬间的峰值 24.19MB 超过 20MB 红线**（见 §4）。

---

## 1. 新增内容

| 文件 | 内容 |
|---|---|
| `src/platform/hotkey.rs` | `RegisterHotKey` 封装，失败只返回 false（不弹窗） |
| `src/platform/gdi.rs` | `Dib` RAII：`CreateDIBSection` + `Drop` 里自动 `DeleteObject`/`DeleteDC` |
| `src/capture/overlay.rs` | 遮罩窗口：`SetWindowRgn` 挖洞、拖拽、尺寸标签、确认/取消 |
| `src/capture/bmp.rs` | **临时** 24bpp BMP 落盘（Spike 3 换成 PNG，本文件删除） |
| `src/platform/tray.rs` | 新增右键菜单项「截图」与气泡提示 |
| `tools/spike2-smoke.ps1` | 自动冒烟：开遮罩、模拟拖拽、验证 BMP 与 GDI |

行为：`Ctrl+Alt+A` → 全屏半透明遮罩 → 拖拽 → 松开即落盘 + 气泡提示。
`Esc` / 右键 / 只点不拖 → 取消。托盘菜单也有「截图」，热键被占用时仍可用。

---

## 2. 内存实测（2560×1600 @200% 缩放，选区 1024×640）

| 阶段 | 工作集 | 增量 | GDI |
|---|---|---|---|
| 空闲（托盘） | 11.35 MB | — | 0 |
| **遮罩弹出** | **15.79 MB** | **+4.44 MB** | 10 |
| 截图后稳定 | 16.69 MB | +5.34 MB | 11 |
| **全程峰值** | **24.19 MB** | +12.84 MB | — |

连续 6 次截图的工作集：16.61 → 16.67 → 16.68 → 16.68 → 16.69 → 16.69（**平台化，零增长**）。

**+4.44MB 的构成**（这就是省下来的 33MB）：
1 个全屏分层窗口（无位图）+ 2 个画刷 + 1 个字体 + region + 首次绘制的 GDI 缓存。
如果按主流做法预抓全屏 DIB，单是一张 2560×1600×4 就是 **16MB**，双屏翻倍。

---

## 3. 三个关键验证

### 3.1 抓到的图里没有遮罩 ✅

把落盘的 BMP 转 PNG 后统计：**平均亮度 239.7**（明亮）。
如果遮罩被拍进去，整幅图会是 ~40–60 的暗色。目视确认：无暗色蒙版、无蓝色选框、无尺寸标签。

原理：`SetWindowRgn` 挖洞后，洞内像素**不属于我们的窗口**，`BitBlt` 从屏幕 DC 取到的是真实桌面；边框和标签都画在洞外。

### 3.2 GDI 零累积 ✅

连拍 6 次，GDI 恒为 11。进程内打点确认了收支平衡：

```
12  confirm: enter            ← 10(基线) + 2 画刷（字体已在更早计入）
14  confirm: dib alive        ← +2：CreateCompatibleDC + CreateDIBSection
13  confirm: overlay destroyed ← -1（窗口销毁连带释放）
11  confirm: dib dropped      ← -2：Drop 里 DeleteObject + DeleteDC
```

### 3.3 那个 "GDI 回不到 0" 是伪问题 ✅

首轮打开遮罩后 GDI 从 0 变 10，且**三轮开关始终是 10**——不是逐次泄漏，是进程级一次性缓存
（窗口站/GDI 初次初始化）。判定标准因此从「回到 0」改为「**不随操作次数增长**」。

---

## 4. 已知问题

### 4.1 抓图峰值 24.19MB > 20MB（唯一超标项）

峰值来源：选区 DIB（1024×640×4 = 2.6MB）+ `CAPTUREBLT` 触发的桌面合成拷贝 + 首次触页。
**它随选区面积线性增长**：全屏 2560×1600 = 16MB DIB，峰值会到 ~35MB。

- 若 20MB 只约束**常驻**（当前口径）：达标，稳态 16.69MB。
- 若要求**峰值**也 ≤ 20MB：需要改造，两条路
  - 分块编码：不整块建 DIB，按行条带 BitBlt → 直接喂 PNG encoder（要换 WIC 流式编码器）；
  - 分块 BitBlt + 先落盘再编码，始终只持有 N 行。

现阶段先记录，等 Spike 3 上 PNG 时一并评估。

### 4.2 边框和标签是半透明的

`LWA_ALPHA` 是整窗统一透明度，边框也跟着 47% 透明。视觉可接受。
若要清晰边框，需把边框画到独立的非分层小窗口上（额外一个窗口，成本可控）。

### 4.3 遮罩背后是实时画面，不是冻结帧

这是省 16–33MB 的直接代价。`freeze_screen` 开关按之前的决定留到设置界面再做。

### 4.4 多屏 / 混合 DPI 未验证

当前用 `GetSystemMetrics(SM_*VIRTUALSCREEN)` 覆盖虚拟屏。单屏 200% 缩放下坐标正确
（抓出的 1024×640 与拖拽比例吻合）。混合 DPI 多屏需另测。

### 4.5 Esc 依赖焦点

`SetForegroundWindow` 在部分场景（前台锁定）会失败，此时 Esc 可能无效，但右键和点击外部仍可取消。
实际测试中首次点击后焦点稳定，Esc 正常。

---

## 5. 冒烟脚本用法

```powershell
# 默认：校准一轮 + 连拍 4 次，全程约 8 秒，会接管屏幕
powershell -ExecutionPolicy Bypass -File tools\spike2-smoke.ps1

# 连拍 6 次
powershell -ExecutionPolicy Bypass -File tools\spike2-smoke.ps1 -Captures 6

# 额外真实按一次 Ctrl+Alt+A（若该组合被 QQ 占用，会拉起 QQ 的截图，慎用）
powershell -ExecutionPolicy Bypass -File tools\spike2-smoke.ps1 -TestHotkey
```

脚本只在**确认遮罩窗口已存在**后才发送鼠标事件，所以不会误操作到你的桌面上。
热键是否注册成功从窗口标题读取（`MySnipTool Tray Host [hotkey=ok|fail]`），不需要真按键。

---

## 6. 踩坑记录

| # | 坑 | 解 |
|---|---|---|
| 1 | `RegisterHotKey` / `MOD_*` / `VK_*` 不在 `WindowsAndMessaging`，在 `UI::Input::KeyboardAndMouse` | 加 feature `Win32_UI_Input_KeyboardAndMouse` |
| 2 | `BeginPaint` / `SetWindowRgn` / `UpdateWindow` / `InvalidateRect` 虽是 user32 函数，却被归到 `Graphics::Gdi` | 从 Gdi 导入 |
| 3 | `GetStockObject(DEFAULT_GUI_FONT)` **每调用一次进程 GDI +1 且永不回收** | 改 `CreateFontW` 自建 + `DeleteObject` |
| 4 | PowerShell 把 `$null` 传给 `string` 参数会变成空串，`FindWindowW(class, $null)` 变成「找空标题窗口」 | P/Invoke 声明改成 `IntPtr`，传 `[IntPtr]::Zero` |
| 5 | 工作集是「粘性」的：释放内存后不会立刻下降，要等系统裁剪 | 判定用「多次操作后是否平台化」，别看单次回落 |
