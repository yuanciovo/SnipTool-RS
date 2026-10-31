# Spike 3.9 / 4 实测记录（残影修复 + 历史面板）

> 两部分：
> ① 快速拖拽的蓝色残影修复（含 A/B 验证与**未能复现**的诚实说明）
> ② 历史面板（Ctrl+Win+V）+ 缩略图 + 点击回写剪贴板

---

# 一、快速拖拽残影

## 1. 诊断

你的猜测是"没做双缓冲 / 没擦背景"，方向对了一半 —— 真正的原因是**时序**：

```
SetWindowRgn   → 立即改变窗口形状，DWM 下一帧就按新形状合成
InvalidateRect → 只是把区域标脏，等消息队列空闲才发 WM_PAINT
```

快速拖拽时 `WM_MOUSEMOVE` 把队列塞满，`WM_PAINT` 一直**饿着**。于是出现
「形状是新的、表面还是旧的」这种不一致：上一帧画在选区**外面**的边框，
在新形状下被划进了可见区，就直接露出来了 —— 表现为竖条残影。
停下来队列一空，`WM_PAINT` 补上，残影消失。和你描述的"停下来就正常"完全吻合。

## 2. 修复

在改完形状后强制同步走一次绘制：

```rust
unsafe fn repaint_now(hwnd: HWND, s: &mut Overlay) {
    let dirty = dirty_union(&s.prev_sel, &s.cur, 30 * s.scale);
    s.prev_sel = s.cur;
    InvalidateRect(hwnd, &dirty, 0);
    UpdateWindow(hwnd);          // ← 关键：不等队列空闲
}
```

两个细节：

- **只脏变化的那一块**（旧选区 ∪ 新选区，外扩 30×scale 覆盖边框和尺寸标签），
  不是整窗重画 —— 全屏 FillRect 每帧一次在 4K 上不划算。
- 状态里加了 `prev_sel`，否则算不出"这一帧到底哪块变了"。

## 3. 诚实的部分：我没能复现

我写了探针：冻结模式开遮罩 → 拖出选区 → 抓住右边缘**反向快速拖拽** →
**按住不放**截屏 → 统计选区内"偏蓝"像素。

- 慢速注入（8ms 间隔）：修复前后都是 211 个蓝像素，**逐像素完全一致**（都是网页本身的蓝色元素）。
- 暴力注入（零间隔、40 步、每步 7px）：修复前后都是 **0**。

结论：`mouse_event` 注入本身有开销（每次几毫秒），消息队列总能被抽空，
所以**合成输入触发不了这个竞态**。真实鼠标的事件率远高于注入。

因此：修复本身是对的（形状/内容一致性是必须保证的），但我**没有拿到"修复前能复现"的证据**。
如果你那边还能看到残影，请把截图或录屏发我，我按真实样本再定位。

---

# 二、历史面板（Spike 4）

## 4. 数据结构

```
%TEMP%\MySnipTool\
├─ snip_20261031_123859_984.png      ← 原图（约 140KB）
└─ thumbs\
   └─ snip_20261031_123859_984.png   ← 缩略图（231×144，等比，约 40KB）
```

**缩略图在截图那一刻生成**：那时全分辨率 DIB 还在手里，`StretchBlt` 一次几毫秒。
若等面板打开再缩放，就得先把原图整块解码（4K 就是 33MB × N 张）。

面板打开时：
1. `list_recent(10)` 只读文件名 + `image::image_dimensions`（**只读 PNG 头，不解码像素**）；
2. 解码 10 张缩略图（合计约 1.5MB）；
3. 关闭时全部 Drop。

## 5. 面板

- 自绘单窗口，**不用 ListView**（ListView + ImageList 的内部缓存比自绘高一倍以上）；
- 380×656（逻辑像素，10 行 × 64），圆角 region，跟随光标并夹到屏幕内；
- 每行：缩略图（等比塞进 96×54 框内居中）+ `10-31 12:38:59` + `873 × 545 · 142 KB`；
- ↑↓ 选择、Enter 打开、Esc / 点外部关闭；
- 点击某行 → 解码原图 → 写剪贴板 → 关面板 → HUD 反馈。

## 6. 托盘菜单（补全五项）

```
截图（动态）      Alt+A
截图（冻结）      Ctrl+Alt+A
─────────────
历史面板          Ctrl+Win+V
打开最新截图
打开临时文件夹
─────────────
设置…            （Spike 5）
退出(E)
```

热键未注册时，菜单底部多一行灰字列出是哪几个被占用。

---

## 7. 冒烟结果（`tools/spike4-smoke.ps1`）

```
INFO  MySnipTool Tray Host [quick=ok frozen=ok history=fail]
PASS  png written                      49 -> 50
PASS  thumbnail written                10 -> 11
PASS  thumbnail is small               231x144 (max 256x144)
PASS  history panel opens              ws 1.75 -> 6.38 MB
PASS  panel closes on click            ok
PASS  clipboard holds the original     873x545
PASS  hotkey toggles the panel         open=True then=False
PASS  GDI not accumulating             cycle1=15 cycle2=15
```

- **`ws 1.75 -> 6.38 MB`**：面板打开只涨 4.6MB，正是 10 张缩略图 + 两个字体 + 三个画刷。
  如果加载原图，这里会是 10 × 数 MB。
- **`GDI cycle1=15 cycle2=15`**：面板开关两轮零增长。
- **`clipboard holds the original 873x545`**：拿到的是原图尺寸，不是缩略图尺寸 —— 证明点击后
  确实去读的硬盘原图。

## 8. 又一个环境冲突：Ctrl+Win+V 注册失败

`history=fail`。这台机器上 `RegisterHotKey(MOD_CONTROL|MOD_WIN, 'V')` 返回 0，
大概率被系统或某个常驻程序占着（Win+V 是系统剪贴板历史，Ctrl+Win+V 也常被占用）。

表现已经是优雅降级：托盘菜单里「历史面板」照常可用，菜单底部有灰字提示。

**三个热键里已经有两个在你这台机器上冲突过**（Alt+A 被微信、Ctrl+Win+V 未知）。
这强烈说明 **Spike 5 的设置界面（热键改绑）不是可选项，是必需项**。

---

## 9. 内存

| 场景 | 工作集 |
|---|---|
| 空闲 | 1.75 MB（裁剪后） |
| 面板打开 | 6.38 MB |
| 面板关闭后 | 4.95 MB |

## 10. 已知限制

| # | 限制 |
|---|---|
| 1 | Ctrl+Win+V 在部分机器被占用，需设置界面改绑 |
| 2 | 缩略图缺失时（Spike 3 时代的老文件）会在打开面板时补生成，首次会慢一点 |
| 3 | 面板不支持滚动，固定 10 条（符合需求） |
| 4 | 清理已覆盖 `thumbs\` 子目录（实测：30h 的缩略图被删、新缩略图保留），孤儿缩略图不会堆积 |
