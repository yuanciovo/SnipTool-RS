# Spike 1 运行说明（空壳托盘程序）

> 目的：测出「Rust + 原生 Win32」的**空闲内存基线**。
> 这一步只验证技术栈能不能过 20MB，所有功能都在这个基线上做加法。
> 验收通过 → 进 Spike 2（框选遮罩）。不通过 → 立刻换 Qt，损失最小。

---

## 1. 环境

本机已确认就位：

| 组件 | 状态 |
|---|---|
| MSVC 14.51（VS Build Tools 2026） | ✅ `C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools` |
| Windows SDK 10.0.26100 | ✅ |
| Rust | ❌ 未安装 |

装 Rust（推荐 rustup，选 `stable-x86_64-pc-windows-msvc`）：

```powershell
winget install --id Rustlang.Rustup -e
```

或者手动下载 <https://win.rustup.rs/x86_64> 运行。安装时如果提示需要 C++ 构建工具，**不用再装**——本机已有 MSVC。

装完**必须新开一个终端**（PATH 才会生效），验证：

```powershell
rustc --version    # 期望 >= 1.75
cargo --version
```

---

## 2. 构建与运行

```powershell
cd <仓库目录>

cargo build --release
```

首次编译约 20–40 秒（只有一个依赖，`windows-sys` 不需要 build script）。产物：

```
target\release\sniptool.exe      # 预计 250–400 KB
```

运行（双击也行，无控制台窗口）：

```powershell
.\target\release\sniptool.exe
```

**预期现象**：任务栏无窗口、无闪烁，托盘区出现一个图标（系统默认应用图标），悬停提示 `MySnipTool · Spike 1（右键退出）`。

**退出**：右键托盘图标 → 退出。

> 重复双击不会产生第二个托盘图标（`FindWindowW` 单实例检查），第二份进程会立即静默退出。

---

## 3. 测内存（三种方式，任选）

### 方式 A：任务管理器（最贴近你定的口径）

`Ctrl+Shift+Esc` → **详细信息** 页 → 找 `sniptool.exe`：

- **「内存(专用工作集)」** ← 这是验收口径，看这一列，目标 ≤ 20MB
- 「内存(专用工作集)」右侧的「提交大小」也顺手看一眼

### 方式 B：脚本（推荐，能看趋势和泄漏）

```powershell
# 采样 120 秒，每 2 秒一次，结束后自动关掉进程
powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Launch -Seconds 120

# 或者先自己启动程序，再挂上去采样
powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Seconds 60
```

输出示例：

```
 T+sec   WS_MB   PRIV_MB   PEAK_MB     DELTA    HND    THR   GDI
 ------   -----   -------   -------     -----    ---    ---   ---
      0    7.84      2.61      7.84      0.00     98      4    18
      2    7.84      2.61      7.84      0.00     98      4    18
...
==== summary ====
working set  max : 7.84 MB   <-- budget 20 MB
RESULT: PASS  (max working set 7.84 MB <= 20 MB)
```

脚本给三样任务管理器看不到的东西：
- **drift**（首末差值）：跑完 120 秒如果涨了 > 1.5MB，说明有泄漏，这个基线不可信；
- **PEAK**：启动瞬间的峰值（动态链接器加载、图标解码都发生在这一瞬间）；
- **GDI 对象数**：后面画遮罩、画面板时，这是比内存灵敏得多的泄漏指标，现在先记下**基线值**。

### 方式 C：VMMap（想看得更细）

微软 Sysinternals VMMap → 选进程 → 看 `Private Data` / `Working Set` / `Total`。
不必装，方式 A/B 够用。

---

## 4. 验收标准

| 指标 | 目标 | 说明 |
|---|---|---|
| 工作集最大值 | **≤ 20 MB** | 硬指标，你定的口径 |
| 工作集首末漂移 | ≤ 1.5 MB | 排除泄漏 |
| GDI 对象数 | 稳定，不随运行时间增长 | 记录基线值，Spike 2 起对照 |
| 句柄数 | 稳定 | 同上 |

参考预期：**7–9MB**（工作集）。因为只链了 kernel32/user32/shell32/gdi32 等系统 DLL，且 `opt-level="z" + lto + strip`。

> 如果数字落在 15–20MB 之间，先别急着换栈：多半是杀软注入（Windows Defender 实时防护会往进程里塞 DLL），把 `target\release` 加进排除项再测一次。
> 如果超过 25MB，那是真有问题，把 `measure.ps1` 的输出发我。

---

## 5. 本阶段的已知限制（都是故意的）

| 项 | 现状 | 何时补 |
|---|---|---|
| 托盘图标 | 系统默认图标 `IDI_APPLICATION` | 正式版用自身 exe 资源里的多尺寸 ico |
| 全局热键 | 没有 | Spike 2 |
| 框选截图 | 没有 | Spike 2 |
| 文件/清理 | 没有 | Spike 3 |
| 历史面板 | 没有（左键双击为空实现，留了位置） | Spike 4 |
| 开机自启 | 没有 | 功能开发阶段 |
| 日志 | 没有（`panic` 会弹对话框，因为无控制台） | 功能开发阶段 |

**为什么先不装热键**：热键注册会多一个 user32 的窗口消息注册项，但更重要的是——如果第一版就带热键，测出来的数字会掺进"注册热键"的成本，而这一层的目标是拿到**干净的空壳基线**。

---

## 6. 代码结构对照

`src/main.rs` 里的函数已经按最终目录结构标注了去向，Spike 2 直接搬进 `platform/`：

| 本文件函数 | 最终位置 |
|---|---|
| `run()` / `main()` | `main.rs` |
| `wnd_proc()` | `app.rs` |
| `register_class()` / `create_host()` | `platform/window.rs` |
| `tray_add()` / `tray_del()` / `show_menu()` | `platform/tray.rs` |
| `to_wide()` / `error_box()` | `platform/win_util.rs` |

一处对技术方案的偏离：宿主窗口用**隐藏的顶层窗口**而非 `HWND_MESSAGE`。原因在 `create_host()` 的注释里——message-only 窗口无法成为前景窗口，`SetForegroundWindow` 会失败，右键菜单就会"点外面不消失"（技术方案 §9 第 3 条）。

---

## 7. 实测记录（2026-10-30，本机）

环境：rustc / cargo **1.99.1**，release 构建，采样 45 秒 × 15 次。

```
编译耗时        3.03 s（windows-sys 0.61.2 + 本 crate）
exe 体积        108 KB
工作集          12.02 – 12.07 MB   ← 验收口径，budget 20 MB
私有内存        2.00 – 2.09 MB
峰值（启动）    12.07 MB
首末漂移        -0.05 MB
句柄            156，全程零增长
线程            4 → 1（初始化后收敛，非泄漏）
GDI 对象        0，全程零增长
```

**结论：PASS，余量约 8 MB。**

三个数字的正确解读：

1. **私有内存 2MB 才是真实成本。** 工作集 12MB 里约 10MB 是 user32/gdi32/shell32/comctl32 的**共享页**——这些页本来就常驻内存（explorer、其他 GUI 程序都在用），本进程只是映射进来，对系统的边际成本远低于 12MB。所以「12MB」看着离 20MB 只剩 8MB，实际余量比数字更大。
2. **GDI 对象 0 是对的，不是测量失败。** 托盘图标是 USER 对象（`GetGuiResources(h, GR_USEROBJECTS)`），不是 GDI 对象。这个程序至今没有创建过任何 DC/Bitmap/Font。Spike 2 开始画遮罩，这一列才会非零——届时以 0 为基线，只增不减就是漏了。
3. **线程 4 → 1 是正常的。** 多出来的三个是 DLL 初始化/杀软扫描线程，完成后自行退出。这不是线程泄漏（泄漏的特征是单调增长）。

含 8MB 余量的后续预算：Spike 4 的面板预算 +2.5MB，config/热键 +0.5MB，合计约 15MB，仍在红线内。真正的风险点是 Spike 2 框选时的**瞬时**峰值（选区 BitBlt，4K 全屏约 33MB），那不在常驻口径内，但要单独测。
