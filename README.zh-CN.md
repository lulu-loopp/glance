<img src="docs/images/banner-zh.webp" width="100%"
     alt="Glance：藏在屏幕边缘的系统监控。鼠标推到边缘，面板滑出；移开即收回。名字旁边是
     磨砂玻璃外观面板的 CPU 一栏，叠在桌面上。">


[![License: MIT](https://img.shields.io/badge/license-MIT-green)](#许可证)
[![Release](https://img.shields.io/github/v/release/lulu-loopp/glance?label=release&color=blue)](https://github.com/lulu-loopp/glance/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/lulu-loopp/glance/total?label=downloads&color=pink)](https://github.com/lulu-loopp/glance/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-8a2be2)](#安装)

Glance 是一个免费开源、不打扰你的 Windows 系统监控。把鼠标推到屏幕边缘，面板滑出，电脑此刻的状态一目了然；移开鼠标，面板收回。

[English](README.md) · [下载](https://github.com/lulu-loopp/glance/releases/latest) · [硬件支持](#硬件支持) · [构建](#构建)

![Glance 磨砂玻璃外观的面板，叠在桌面上](docs/images/desktop-zh.webp)

## 显示什么

- **CPU**：占用、每个线程、频率、温度、封装功耗、每个 CCD 的温度
- **GPU**：占用、显存、频率、温度、风扇，以及 NVIDIA 和 AMD 独立显卡的功耗
- **内存**：用量，以及每根内存条的温度
- **网络和磁盘**：流量，以及硬盘温度
- **进程**：按 CPU、内存、读写或 GPU 排序的最忙程序
- **存储**：每个分区的空间
- **主板**：各处温度和风扇转速
- **电池**：笔记本上显示

## 三种外观

记录纸、磨砂玻璃（每一块面板的边缘都会折射背后的桌面）和 Windows 11（与开始菜单相同的亚克力材质）。可选浅色、深色，或跟随壁纸亮度自动切换。界面支持中文和英文。

![三种外观（深色）：记录纸、磨砂玻璃、Windows 11](docs/images/looks-zh.webp)

## 小巧安静

程序只有一个约 1 MB 的可执行文件，用 Direct2D 和 DirectComposition 原生绘制。面板收起时约占 55 MB 内存，CPU 占用远低于一个核心的 1%。不需要账号，没有广告；除每天向 GitHub 查询一次新版本外不联网，这项检查可以在设置中关闭。

## 安装

1. 从[最新版本](https://github.com/lulu-loopp/glance/releases/latest)下载 `Glance_<版本>_x64-setup.exe` 并运行。
2. 安装程序已签名（发布者：Weiyi Shi）。由于签名刚开始使用，最初的一段时间里 Windows SmartScreen 仍可能提示“Windows 已保护你的电脑”，点击“更多信息”→“仍要运行”即可。
3. Glance 第一次启动时会请求一次管理员权限：读取温度、功耗和风扇需要它，通过安装程序装好的已签名驱动 [PawnIO](https://github.com/namazso/PawnIO) 完成。此后启动不再询问，也可以设置为开机时启动。

**安装位置。** 只有安装在普通程序无法修改的文件夹里，Glance 才会不经询问地以管理员身份启动：默认的 Program Files，或磁盘根目录下的新文件夹（例如 `D:\Glance`）。选择其他位置时，安装程序会说明影响；装在那里的 Glance 每次启动都需要确认管理员权限，也无法开机时启动。

## 使用

- **打开面板**：把鼠标推向屏幕右边缘（推向哪一侧、需要多大力度、面板出现在什么位置，都可以在设置中调整）。全屏程序中同样可用。需要有意识地推一下才会打开，所以屏幕边缘的滚动条照常可用。也可以按 **Ctrl+Alt+G**，在鼠标所在位置打开面板，再按一次收起。
- **收起面板**：把鼠标移开。
- **固定面板**：点面板底栏的图钉，面板会一直显示，方便边做别的事边看；再点一次图钉或按 Ctrl+Alt+G 即可收起。
- **随时一瞥**：鼠标悬停在托盘图标上，会显示 CPU、显卡和内存的简要读数。开启“过热提醒”（默认关闭）后，CPU 或显卡持续 30 秒达到温度警示值时，托盘会弹出提醒。
- **设置**：右键单击托盘图标，或再次启动 Glance。设置窗口也可以用键盘操作（Tab、方向键、空格）。
- **更新**：有新版本时，托盘会提示，设置中会出现更新按钮。Glance 下载安装程序后，只有确认其签名与当前 Glance 出自同一发布者，才会运行它。
- **卸载**：在“设置 → 应用 → 已安装的应用”中卸载，或在 Glance 的设置中卸载。卸载会删除 Glance、它的计划任务和设置，并询问是否一并卸载 PawnIO 驱动（其他监控软件可能也在使用它）。

## 硬件支持

| | 支持 | 已在真机上验证 |
|---|---|---|
| CPU | AMD Ryzen（Zen 及以后）、Intel Core | AMD Ryzen 9 9950X |
| 主板传感器 | ITE 和 Nuvoton Super I/O 芯片 | ITE IT8689E |
| 内存温度 | 带传感器的 DDR4 和 DDR5 内存条 | DDR5 |
| 显卡功耗 | NVIDIA（NVML）、AMD（ADL）独立显卡 | NVIDIA RTX 5070 Ti |
| 其他读数 | 通过 Windows 获取（与任务管理器相同的数据来源） | |

欢迎 Intel 和 Nuvoton 机器的用户反馈实际效果：在“设置 → 诊断信息”中点“复制”，Glance 会把识别到的硬件和读数情况复制到剪贴板，直接粘贴到 issue 即可（不包含路径或用户名）。笔记本的风扇和主板温度由笔记本自己的嵌入式控制器管理，Glance 暂不显示；核显的功耗包含在 CPU 封装功耗里，在 CPU 一栏中显示。

## 构建

```
cargo build --release
makensis installer\glance.nsi
```

`scripts\release.ps1` 会完成以上两步；加上 `-Sign` 时，还会为可执行文件、卸载程序和安装程序签名（`scripts\sign.ps1`，使用 Microsoft Artifact Signing）。调试版本启动时不请求管理员权限，因此也没有驱动提供的传感器读数。

本页的图片和宣传视频都由 Glance 自己绘制：`cargo build --release --features studio` 会加入 `glance --studio script.json out-folder` 命令，按镜头脚本在屏幕外逐帧渲染面板，再由 `studio\readme.py` 和 `studio\compose.py` 合成（见 `studio\`）。

## 许可证

Glance 以 MIT 许可证发布（[LICENSE](LICENSE)）。它附带或内置的第三方软件遵循各自的许可证：PawnIO 驱动安装程序（GPL-2.0，附例外条款；源代码随每个版本一并发布）、PawnIO 模块（LGPL-2.1，源代码位于 `pawnio-modules/source`）、Archivo 和 Inter 字体（OFL 1.1），以及 Rust 依赖库；详见 [licenses/THIRD-PARTY.txt](licenses/THIRD-PARTY.txt)。
