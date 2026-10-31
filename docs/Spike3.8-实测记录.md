# Spike 3.8 实测记录（双模式 + 移动/缩放）

> 变更：**动态**（Alt+A，不抓全屏）+ **冻结**（Ctrl+Alt+A，全屏 DIB 垫底）双模式；
> 待确认态支持拖动移动选区、拖边角缩放，尺寸文字与工具栏实时跟随。

---

## 1. 先说我诊断出的真正病根

你观察到的「发灰 + 重影 + 视频还在动」，不是半透明遮罩本身的锅，是 **Spike 3.5 的实现错了**：

我把截图 BitBlt 到遮罩窗口上做"冻结"，而遮罩是 `WS_EX_LAYERED` + `LWA_ALPHA=120`。
画在分层窗口上的任何像素都会**和它身后的实时桌面再混合一次**（47%）。
于是：截图被压暗 → 发灰；实时画面又透过 47% → 重影；视频继续动 → 因为底子还是活的。

**正确做法：永远不要把内容画到分层遮罩上。** 洞内要么放行实时桌面（动态模式），
要么由一个**不透明**窗口垫在遮罩下面（冻结模式）。

---

## 2. 两种模式的窗口结构

### 动态模式（Alt+A）：1 个窗口

```
  遮罩（分层 47%，挖洞）
  ────────────────────────
  洞内 → 实时桌面（不混合，干净）
```

只抓确认时的选区，常驻内存不变。

### 冻结模式（Ctrl+Alt+A）：2 个窗口

```
  ┌───────────────────────────────┐
  │ 遮罩（分层 47%，挖洞）          │  ← z 序在上
  ├───────────────────────────────┤
  │ 底图（不透明，全屏静态照片）     │  ← z 序在下
  └───────────────────────────────┘
  洞内 → 底图窗口的像素（不混合、绝对静止）
```

热键按下瞬间先 `BitBlt` 整个虚拟屏（**必须在创建任何窗口之前**，否则会抓到自己的遮罩），
再创建底图窗口和遮罩。确认时直接从这张 DIB 里裁选区，**不再碰屏幕**。

---

## 3. 洞内点击穿透：这次的真问题

待确认态保留洞（内容才干净），但洞内像素不属于我们的窗口 ——
**第一次按下的点击会穿透到下层程序**。`SetCapture` 救不了，它是在收到按下之后才生效的。

解法：在选区正上方盖一个 `alpha = 1` 的分层小窗（`catcher`），0.4% 透明度肉眼不可见，
但能接住点击，把坐标换算成遮罩客户区坐标后 **`SendMessage` 同步转发**给遮罩。

> 为什么用 SendMessage 而不是 PostMessage：同步转发保证「按下」先于「抬起」被处理。
> 异步的话，快速点击时抬起会落到捕获窗自己身上，遮罩就卡在一个没有 up 的拖拽里。

捕获窗必须**先于工具栏创建**，因为工具栏在空间不够时会翻到选区内部，
那时候它必须压在捕获窗上面，否则按钮点不到。

---

## 4. 命中测试

待确认态下，鼠标按下位置决定行为：

| 位置 | 判定 | 行为 | 光标 |
|---|---|---|---|
| 8 个角/边（±5×scale px） | `Hit::Edge` | 缩放 | 双向箭头 |
| 选区内部 | `Hit::Inside` | 整体移动 | `IDC_SIZEALL` |
| 选区外部 | `Hit::Outside` | 取消 | 箭头 |

- 边角判定优先于内部判定（先角、再边、再内部）。
- 缩放会把矩形规范化，允许拖过对边翻转；最小尺寸 `4×scale`。
- 移动时整体夹在虚拟屏范围内。
- 移动/缩放过程中同步做三件事：重建 region 洞、`InvalidateRect`、重定位捕获窗与工具栏。

---

## 5. 冒烟结果（`tools/spike38-smoke.ps1`）

```
INFO  hotkeys: MySnipTool Tray Host [quick=fail frozen=ok]
PASS  dynamic mask opens                 ws=16 MB (idle 11.42)
PASS  pending toolbar appears            hwnd=0x3F0EC4
PASS  hit test: inside = move            title='MySnipTool Overlay [moving]'
PASS  move keeps size                    png=1025x641 expect~1024x640
PASS  hit test: corner = resize          title='MySnipTool Overlay [resizing]'
PASS  corner resize grows size           png=1185x761 expect~1184x760
PASS  frozen backdrop exists             hwnd=0x140CCE  ws 1.81 -> 19.39 MB
PASS  frozen capture written             png=1025x641 expect~1024x640
PASS  backdrop destroyed with mask       ok
PASS  Esc leaves no orphan window        ok
PASS  GDI not accumulating               per-round: 19, 19, 19
```

几条值得看的：

- **`hit test: inside = move` / `corner = resize`**：直接读窗口标题确认命中分类，
  比"看尺寸变了没"更能定位问题（尺寸不变也可能是"根本没动"）。
- **`move keeps size`**：移动后 PNG 尺寸不变，证明是"整体平移"而不是"重新框选"。
- **`corner resize grows size`**：尺寸正好多了拖拽距离（±1px 归一化误差）。
- **`frozen backdrop exists … 1.81 -> 19.39 MB`**：冻结模式一次性 +17.6MB，
  正好是 2560×1600×4 = 16.4MB 的全屏 DIB 加开销 —— 数字对得上，说明抓的是整屏。
- **`GDI 19/19/19`**：捕获窗、工具栏、底图、HUD 的 GDI 全部回收，不累积。

---

## 6. 实测发现：这台机器上 Alt+A 注册失败

`[quick=fail frozen=ok]` —— Alt+A 已被本机某个程序占用（`RegisterHotKey` 返回 0）。
Ctrl+Alt+A 正常。程序按约定优雅降级：托盘菜单里两项都能用，菜单里多一行灰字提示，
tooltip 也改成"部分未注册"。

**这不是 bug，但会影响默认体验**。三个选项供你定：
1. 保持 Alt+A（用户机器上可能没冲突），失败时走菜单；
2. 换一个更冷门的组合（如 `Alt+Shift+A`）；
3. 等 Spike 5 的设置界面，允许用户自己改。

---

## 7. 内存

| 场景 | 工作集 |
|---|---|
| 空闲 | 11.4 MB |
| 动态模式拖拽中 | 16.0 MB |
| 冻结模式打开瞬间 | +17.6 MB（全屏 DIB，闭合即释放） |
| 确认后（裁剪过） | 4.3 MB |

按你定的"稳态 ≤ 20MB、瞬态不管"标准，两种模式都达标。

---

## 8. 已知限制

| # | 限制 |
|---|---|
| 1 | Alt+A 在部分机器上会被占用，需在设置界面提供改绑 |
| 2 | 冻结模式下若期间屏幕内容变化（如视频），确认时得到的是热键按下那一刻的画面 —— 这是设计意图，但用户可能误以为"实时" |
| 3 | 多屏混合 DPI 未验证，底图/遮罩都按虚拟屏矩形铺 |
| 4 | 捕获窗 alpha=1 在极暗背景下理论上会压暗 0.4%，肉眼不可见 |
| 5 | 移动选区时若拖到屏幕外会被夹住（不会跑丢），但没有橡皮筋式回弹 |

---

## 9. 复跑

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File tools\spike38-smoke.ps1
```

脚本约 25 秒，会接管屏幕、模拟移动与缩放。鼠标事件只在**确认遮罩窗口存在**后才发送。
