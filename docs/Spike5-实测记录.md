# Spike 5 实测记录：动态即拍 + 文字残影修复 + 设置界面

日期：2026-10-31
构建：`cargo build --release`，零警告，`sniptool.exe` 399 KB，22 个 rs 文件 / 4566 行

---

## 1. 动态模式改为「松手即落盘」

用户要求：Alt+A 追求极致速度，不要待确认态、不要工具栏，松手就存盘+进剪贴板+关窗。

实现：`WM_LBUTTONUP` 里按模式分流。

```rust
Drag::New => {
    ...
    if s.mode == Mode::Dynamic { AfterUp::Commit } else { AfterUp::Pending }
}
```

- `commit()` 去掉了 `phase == Pending` 的前置条件（原来只有待确认态才能确认）。
- 冻结模式行为不变：仍进待确认、仍有 √/× 工具栏、仍可移动/缩放。
- 代码上顺手修了一个隐患：原来在 `WM_LBUTTONUP` 里持有 `state()` 返回的 `&mut` 时就调用
  `confirm()`，而它内部又取了一次 `state()` —— 两个 `&mut` 同时存在。现在先算出
  `AfterUp` 枚举、把借用放掉，再执行动作。

## 2. 文字残影：这次真的复现了，A/B 对照有据

### 症状
快速拖拽选区时，旧的尺寸数字和提示文字（「…拖边角缩放…」）被拉长、重影，
只出现在选区右侧。

### 根因
尺寸标签锚在 `sel.left`，往右铺开 `LABEL_W = 460 * scale`，**远比一个窄选区的右边界宽**。
而脏区是从选区矩形推出来的：

```
脏区 = union(上一帧选区, 当前选区) + 30 * scale
```

进入待确认态时做的是**整窗重绘**，所以标签是**完整画出来**的。
之后任何一次窄选区的增量重绘，脏区右缘只有 `sel.right + 30*scale`，
标签尾部（约等于 `sel.left + 500px` 之外那一段）**永远不在脏区里** ——
`DrawTextW` 只画脏区以内的部分，脏区以外那些上一帧画好的字，再也没人擦掉。

### 修法
把「这一帧到底画了什么」精确记录成一个包围盒，脏区按它算：

```rust
fn frame_bbox(sel, client, scale) -> RECT {
    let mut b = 选区外框;
    let l = label_rect(sel, client, scale);   // 与绘制共用同一个函数
    b = union(b, l);
    b
}
```

两个关键点：
1. **绘制和脏区计算必须走同一个 `label_rect()`**。两边各算一次，早晚会不一致。
2. `prev_frame` 存的是包围盒而不是选区，所以「标签翻到选区下方」这种位置跳变也覆盖到。

### A/B 实测（`tools/spike5-ghost.ps1`）

探针思路：冻结模式（底图静止，任何像素变化都只可能是我们自己画的）→
建一个窄选区 → 内部拖动让选区整体位移（标签跟着走，必然离开旧脏区）→
截图 A → `RedrawWindow(RDW_INVALIDATE|RDW_ERASE|RDW_UPDATENOW)` 强制整窗重绘 → 截图 B。

A 和 B 必须逐像素相同。

| 构建 | 结果 |
|---|---|
| 旧脏区（只按选区算） | **FAIL**：1864 个像素残留，bbox x 1136–1276 / y 438–466 |
| 修复后（按包围盒算） | **PASS**：像素完全一致 |

把残留区域放大，能直接读出「…边角缩放」的字形 —— 和用户描述的现象一致。

> 上一轮（Spike 3.8 → 4）我用 `mouse_event` 注入拖拽想复现蓝色竖条，没能复现，
> 只能声明「未验证」。这次换了思路：**不追竞态，找一个确定性的状态差异**
> （整窗重绘 vs 增量重绘），一次就抓到了。教训是：复现一个时序 bug 时，
> 去构造"两种绘制路径必然不同"的静态场景，比提高注入速率有效。

## 3. 设置界面 + 配置持久化 + 开机自启

### 配置
`%APPDATA%\MySnipTool\config.ini`，`key=value`，UTF-8 **带 BOM**。

默认值：

```ini
quick=Alt+A
frozen=Ctrl+Alt+A
history=Alt+V
retention_hours=24
autostart=1
```

为什么不用 JSON：只有 5 个字段。手写 JSON 解析要处理转义和嵌套，引 serde 又多
100KB 和一堆编译时间。`key=value` 每行一条，**任何一行写错只回退那一项**，
不会让程序起不来。写 BOM 是为了让记事本和 PS 5.1 都能正确显示中文注释。

### 热键
`Combo { mods, vk }` 一套结构贯穿配置、界面、注册三处：

- `Combo::parse("Ctrl+Alt+A")` —— 任何一处不合法就返回 `None`，配置写错不静默采用
- `Combo::label()` —— 规范顺序 Ctrl→Alt→Shift→Win→键名
- **必须至少含一个修饰键**，否则裸字母会吃掉全局输入

改绑是「注销旧的 → 注册新的 → 落盘 → 改自启 → 刷新托盘提示 → 重置清理定时器」
一次性做完，点保存立刻生效，不用重启。

### 设置窗口
原生子控件（STATIC / BUTTON / EDIT）+ 一个自绘的热键录入框。

- 为什么自绘热键框：没有任何原生控件能录组合键。`EDIT` 子类化会和输入法打架，
  还会吞掉 Tab / 方向键；独立窗口类最干净。
- 为什么其余控件用原生的：这个窗口一辈子打开几次，自绘按钮的命中测试和
  键盘/无障碍支持都得自己写，不划算。
- 校验失败、注册失败**都不弹窗**，写在窗口底部那行状态文字里。注册失败时窗口
  不关，方便当场换一个组合。

### 开机自启
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值名 `MySnipTool`，路径带引号。
每次启动对齐一次（幂等）。用户在「任务管理器→启动」里禁用由 `StartupApproved`
掌管，我们不碰那个键，所以不会和用户打架。

### 清理周期
原来固定 24h。保留时长可调到 1 小时后，24h 扫一次等于让文件多活一天。
改成 `保留时长的 1/4`，夹在 5 分钟到 1 小时之间。扫描只读目录项元数据，频率高没有代价。

## 4. 一个顺带修的质量问题：没有清单文件

设置窗口第一版截图里，复选框是 13px 的经典方块，和 30px 的文字完全不成比例。
原因不是 DPI 代码，而是**进程没有应用清单**，comctl32 退回 v5（经典控件，
不跟 DPI、不跟主题）。

加 `app.manifest`（声明 Common-Controls 6.0 + permonitorv2），用 `build.rs`
通过 `link.exe /MANIFESTINPUT` 链进去 —— 一条命令行参数，不引第三方
`embed-resource`。之后复选框是正常的蓝色勾选方框，按钮也有主题外观。

另外窗口类补了 `hbrBackground = COLOR_BTNFACE+1`（系统颜色刷，不额外占 GDI 对象）：
留 NULL 时窗口底色不会被擦除，移动/重绘会留下上一帧的残渣。

## 5. 实测结果

`tools/spike5-smoke.ps1`（15 项断言，全 PASS）：

```
PASS  config hotkeys registered          [quick=ok frozen=ok history=ok]
PASS  no toolbar in dynamic mode         never seen
PASS  overlay closes on mouse-up         ok
PASS  png written on mouse-up            9 -> 10
PASS  png matches the selection          873x545 expect~872x544 (scale 2)
PASS  clipboard filled on mouse-up       873x545
PASS  frozen mode keeps the toolbar      pending state ok
PASS  frozen confirm writes png          10 -> 11
PASS  settings window opens              boxes=3 size=401x328
PASS  settings closes after save         ok
PASS  settings leaks no GDI              before=30 after=26
PASS  config survives save round-trip    history=Alt+Shift+V
PASS  retention survives save            retention_hours=2
PASS  Run key points at the exe          "...\target\release\sniptool.exe"
```

几条断言的设计意图：

- **「no toolbar in dynamic mode」**：松手后 200ms 内每 20ms 查一次工具栏窗口，
  一次都不许出现。这是「极速模式」的核心语义，比"png 有没有生成"更能说明问题。
- **「config survives save round-trip」**：脚本先塞一个**非默认**组合
  `history=Alt+Shift+V`，再打开设置点保存。如果程序解析失败退回默认值，
  写回去就会变成 `Alt+V`，断言立刻失败。这比"配置文件存在"强得多。
- **「settings leaks no GDI」**：设置窗口会创建 1 个字体 + 9 个画刷（3 个热键框各 3 个），
  全都要在 `WM_DESTROY` 里 `DeleteObject`。

内存：空闲 6.3–7.6 MB（含主题引擎加载后的一次性增长），远低于 20 MB 红线。

## 6. 已知限制

| # | 限制 | 说明 |
|---|---|---|
| 1 | 设置窗口不响应 `WM_DPICHANGED` | 拖到另一块不同缩放的显示器上不会重排；用系统 DPI 创建 |
| 2 | 热键录入框未做「按下即注册测试」 | 要等点保存才知道有没有冲突；好处是不会边录边抢别人的热键 |
| 3 | `Alt+V` 也可能被某些编辑器占用 | 注册失败时菜单有灰字提示，设置里可换 |
| 4 | 动态模式没有撤销 | 松手即落盘；框错了只能重截（这是"极速"的代价，用户已确认） |
| 5 | 配置文件不带写锁 | 程序运行中手改文件不会即时生效，下次启动才读 |

---

## 7. 设置界面美化（GDI 重绘版）

用户给了参考图，要求「现代软件设置界面」的观感，但坚持纯 Win32 + GDI。
实机截图见 `docs/settings-ui.png`。

### 7.1 色板（从参考图采样，不是凭眼估）

| 用途 | 值 |
|---|---|
| 窗口底 | `#F5F5F5` |
| 卡片 | `#FFFFFF`，圆角 10，无边框 |
| 正文 / 标签 | `#1F1F1F`，14px，Semibold |
| 分区标题 / 说明 | `#8A8A8A`，12px |
| 主题蓝 / 悬停 / 按下 | `#0066FF` / `#3385FF` / `#0052CC` |
| 分割线 | `#EDEDED`，1px，左右内缩 24 |
| 输入框底 | `#F5F5F5`，聚焦时 `#FFFFFF` + 1px 蓝边 |
| 开关关态 | `#D9D9D9` |

采样方法：把参考图丢给 Python 取 `Counter` 前 12 色，主题蓝正好是 `(0,102,255)`。

### 7.2 自绘范围

**整窗自绘，只留一个原生 `EDIT` 输入数字。**

为什么不做「原生控件 + 改色」：一旦启用视觉主题，`BUTTON` / 复选框会**直接忽略**
`WM_CTLCOLORBTN`，想改色只能 `BS_OWNERDRAW` —— 那已经是自绘，只是多套了一层壳。

为什么数字框留原生：输入法、光标、选区、退格、右键菜单全是白送的，
自己实现要多 80 行且更容易出行为差异。它没有 `WS_BORDER`，父窗口在它下面画
圆角卡片，`EDIT` 四周内缩 10/6 逻辑像素，方块的四角正好落在圆角里，看不见。

同时删掉了 `ui/hotkey_box.rs`（原来自绘的热键子窗口）：
热键录制并进主窗口后，焦点、Tab、Esc 全部走同一套状态机，
少一个窗口类、少 9 个 GDI 对象，也少一层跨窗口消息转发。

### 7.3 GDI 预算

`Theme::new()` 建一次，`Drop` 统一删：

- 4 个字体（标题 20/600、标签 14/600、正文 13/400、说明 12/400）
- 11 个画刷（含 `BS_HOLLOW` 空心刷，用于只描边不填充）
- 4 个画笔（NULL / 蓝边 / 边框 / 关闭叉）

**合计 19 个，与交互次数无关。** 绘制路径上零创建：
`RoundRect` / `Ellipse` 只是 `SelectObject` 换对象，不分配句柄。
实测开/关一轮 GDI 27 → 24（下降，无泄漏）。

### 7.4 滑动开关 + 160ms 动画

```rust
// 点击 → 目标值切换 → 起 16ms 定时器，每帧 +0.1（10 帧 = 160ms）
let d = st.anim_target - st.anim;
if d.abs() <= ANIM_STEP { st.anim = st.anim_target; KillTimer(..) }
else { st.anim += ANIM_STEP * d.signum(); }
InvalidateRect(hwnd, &lay.toggle, 0);   // 只重画开关那一小块
```

滑块位置 = `left + pad + (right - left - 2*pad) * anim`，用 `Ellipse` 画。
轨道颜色按 `anim >= 0.5` 在灰/蓝之间切 —— 不做颜色插值是为了**每帧零分配**
（否则每帧都要 `CreateSolidBrush` + `DeleteObject`，160ms 内 10 次，虽然账能平，但没必要）。

### 7.5 无边框窗口

- `WS_POPUP` + 自绘标题栏；`WM_NCHITTEST` 在标题区返回 `HTCAPTION` 交给系统拖动
  （关闭按钮所在的矩形排除在外，否则点不到）。
- Win11：`DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE=33, DWMWCP_ROUND)`
  拿**系统级平滑圆角**（带抗锯齿）。
- Win10：该属性不存在、返回非 0，退回 `CreateRoundRectRgn` + `SetWindowRgn`（硬边）。

### 7.6 踩到的三个坑

1. **窗口没被置顶，点击全部落空。**
   托盘软件通过 `PostMessage` 打开设置窗时进程没有前台权限，`SetForegroundWindow`
   被系统拒绝，窗口就压在别的程序下面 —— 我的自动化点击全打在别人窗口上。
   修法：`SetWindowPos(TOPMOST)` → `SetWindowPos(NOTOPMOST)` → `SetForegroundWindow`。
   这在真实场景同样会发生（用户点托盘图标时不一定拿到前台权）。
2. **提示文字一个都不显示**：`hint` 矩形的 `left` 算成了 `f_hours.right + 10`、
   `right` 还是 `IN_R`，左右颠倒 → `DrawTextW` 什么都不画。改成「输入框左侧 + 右对齐」。
3. `DT_RIGHT` 忘了导入。这类常量散落在 `Graphics::Gdi` 里，编译期才报。

### 7.7 实测

```
PASS  settings window opens      hwnd=0xB310E4 edits=1 size=480x552
PASS  save button closes it      ok          # 自绘按钮，走真实鼠标点击
PASS  settings leaks no GDI      before=27 after=24
```

`tools/settings-toggle.ps1` 专门验证开关 → 配置 → 注册表的双向链路：

```
PASS  toggle off -> config=0 and Run value removed
PASS  toggle on  -> config=1 and Run value written
```

exe 403KB（比上一版 +4KB，多了 DWM 调用与自绘代码），21 个 rs 文件 / 5022 行。

---

## 8. Spike 5.1：闪烁 / 窗口"消失" / 冻结模式键盘操作

三个反馈里，有两个的**根因和用户猜的不一样**，先记结论。

### 8.1 设置窗口"失焦就消失" —— 不是 WM_KILLFOCUS

先查代码：`ui/settings.rs` 里**没有任何** `WM_KILLFOCUS` / `WM_ACTIVATE` 处理，
销毁路径只有 Esc、×、取消、保存四个。再实测：

```
settings hwnd = 0xCD1234
after clicking outside: isWindow=True visible=True foreground==settings: False
```

**窗口根本没被销毁，也没被隐藏。** 真正的问题是它当时用的是
`WS_EX_TOOLWINDOW` —— 这类窗口**既没有任务栏按钮、也不出现在 Alt+Tab 里**。
用户一点浏览器，它沉到后面，就再也捞不回来了，观感就是"消失了"。

修法：改成 `WS_EX_APPWINDOW`（0x40000），给它一个任务栏按钮和 Alt+Tab 条目。
冒烟脚本加了一条断言直接盯这个位：

```
PASS  settings reachable after blur      exstyle=0x40000
```

> 历史面板那个 `WM_ACTIVATE → DestroyWindow` 是**故意**的（点外面就收起），
> 但它只在 `ui/history.rs` 里，从来没有落到设置窗口上。

### 8.2 鼠标移动闪烁 —— 无缓冲的全窗重绘

窗口类当时带 `CS_HREDRAW | CS_VREDRAW`，悬停变化又 `InvalidateRect(整窗)`，
而 `paint()` 是**直接往窗口 DC 上画**：先 `FillRect` 铺满底色，再一层层画卡片、
文字、控件。一帧里几十个 GDI 调用，DWM 逐块合成时就能看到"只有底色、卡片还没上"
的那一瞬间 —— 就是闪烁。

三处一起改：

1. **双缓冲**：`CreateCompatibleDC` + `CreateCompatibleBitmap` 建一次（`BackBuf`），
   所有绘制先落离屏 DC，最后**一次 `BitBlt`** 贴上去。
2. **按脏区裁剪**：`SaveDC` + `IntersectClipRect(ps.rcPaint)`，只重画真正脏的那块，
   再 `BitBlt` 同一块。
3. **局部标脏**：悬停/按下/焦点变化只 `InvalidateRect` 相关控件（外扩 2 逻辑像素
   盖住聚焦描边），不再整窗重绘。窗口 960×1104 物理像素，切到按钮上原来是
   100 万像素，现在是 3 万。

顺带去掉 `CS_HREDRAW | CS_VREDRAW`（窗口尺寸固定，留着只会引入整窗重绘）。

> 闪烁没法用截图证明，但机制上已经没有中间态了：每个 `WM_PAINT` 只对窗口做一次
> `BitBlt`，屏幕上永远不会出现画了一半的帧。

### 8.3 冻结模式的 Enter / Esc —— 是前台权限，不是工具栏抢焦点

用户的猜测是"焦点被工具栏抢走"。工具栏和捕获窗都带 `WS_EX_NOACTIVATE` +
`SWP_NOACTIVATE`，实测它们确实不抢。真正的原因在日志里：

```
overlay hwnd = 0xC012C2   foreground==overlay: False   ← 刚拉起时
pending=True toolbar=True foreground==overlay: True    ← 拖过一次之后
```

**热键拉起遮罩时，`SetForegroundWindow` 被系统静默拒绝了。** 于是遮罩盖满全屏、
鼠标能拖（点一下就会激活），但**键盘还在上一个程序手里** —— 没拖就直接按 Esc，
按键就发给浏览器了。拖过一次之后焦点才过来，所以"有时候能按、有时候不能"。

修法：`grab_keyboard()` —— `AttachThreadInput` 到当前前台线程再抢焦点，
这是绕开前台锁的标准做法。两处调用：`open()` 之后、`enter_pending()` 之后
（后者是兜底，成本极低）。

实测：

```
PASS  Esc works before any drag     ← 之前必挂
PASS  real Enter confirms
PASS  real Esc cancels
```

### 8.4 本轮回归

`tools/spike5-smoke.ps1` 16 项全 PASS；新增 `tools/focus-keys.ps1`
（真实 `keybd_event`，不用 PostMessage，专门盯键盘链路）。

exe 405KB。设置窗口 GDI 27→24（双缓冲只多 1 DC + 1 位图，随窗口销毁释放）。

---

## 9. 回归发现的真问题：抢到前台 ≠ 看得见（Spike 5.1 引入）

准备开源时跑全量回归，`settings-toggle.ps1` 挂了：

```
after save click: window alive=True     ← 点了保存，窗口没关
config after : autostart=1              ← 开关状态也没变
```

窗口在、`visible=True`、`foreground==settings` —— 看上去一切正常。探针给出决定性证据：

```
foreground : class='MySnipTool.Settings'  isOurSettings=True
WindowFromPoint(保存按钮) -> class='Chrome_RenderWidgetHostHWND'  isOurs=False
```

**我们是前台窗口，但那个像素点上压着别的进程。**

原因是 Spike 5.1 的一个错误简化：把"TOPMOST→NOTOPMOST 提升 z 序"换成了
`AttachThreadInput`，并把提升 z 序降级成"只有抢不到前台才做"的兜底。
但桌面上只要存在 topmost 窗口（某些播放器、常驻工具、Electron 应用），
我们虽然成了前台，**z 序仍然在它下面** —— 条件判断为假，兜底永远不触发。

修法：把提升 z 序改成**无条件**执行（仍只改 z 序，不留置顶状态）：

```rust
SetForegroundWindow(hwnd);          // 抢焦点
SetWindowPos(hwnd, HWND_TOPMOST, ...SWP_NOACTIVATE);
SetWindowPos(hwnd, HWND_NOTOPMOST, ...);
BringWindowToTop(hwnd);
```

### 教训

**"抢到前台"和"用户能看到"是两件事**，topmost 层级不在前台锁的管辖范围内。
凡是托盘程序弹自己的窗口，z 序必须显式提升，不能指望 `SetForegroundWindow`。

顺带说明这个 bug 是靠 `settings-toggle.ps1` 抓到的 —— 冒烟脚本里
"save button closes it"那条当时是通过的，因为它的点击恰好落在没被遮挡的位置。
**同一行为有两条独立断言，才兜得住这种位置相关的偶发。**
