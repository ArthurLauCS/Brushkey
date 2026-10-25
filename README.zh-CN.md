# Brushkey

[English](README.md) | 简体中文

Windows 拼音输入法：打简体或繁体（台湾）中文，选一次就记住，全屏游戏里也能用。按键和微软拼音一样，可以直接替换它。

- **Ctrl+Shift+F 切换简繁。** 拼音不变。繁体用台湾字形，台湾用词排在逐字转换前面（打 ruanjian 先出“軟體”，再出“軟件”）。
- **选一次就记住。** 选过的词下次排第一，所有程序立刻都能用上新学的词。
- **游戏。** 全屏游戏里能用，自己画候选框的游戏也行。
- **轻快。** 每次按键约 0.3 ms，DLL 约 500 KB；20 MB 的词库所有程序共用一份，不是每个程序各载一份。
- **不联网。** Brushkey 从不连接网络。设置窗口有中文和英文，跟随系统深色 / 浅色。

## 安装

在 [Releases](https://github.com/ArthurLauCS/Brushkey/releases/latest) 下载 `Brushkey-Setup-<版本>.exe` 并运行。需要 64 位 Windows 10 或 11。

- 安装包没有商业代码签名证书，Windows 可能提示“Windows 已保护你的电脑”，点 **更多信息** → **仍要运行**。
- 安装要管理员权限：输入法是给整台电脑注册的。
- Brushkey 装在 **中文(简体，中国)** 下。Windows 里还没有这门语言的（比如只装了中文(繁体，台湾)），先在 **设置 → 时间和语言 → 语言和区域 → 添加语言** 里加上。
- 装好后按 **Win+空格** 选“Brushkey 拼音”。只想留 Brushkey 的话，在 **语言和区域 → 中文(简体，中国) → … → 语言选项 → 键盘** 里删掉微软拼音。
- 升级时，已经开着的程序要重开才会用上新版。

## 打字

整句一口气打完：第一个候选是最可能的整句，后面是开头几个音节的词。只打声母也行（`zg` 出“这个”“中国”）。

相邻两个字母打反了会自动纠正（`xainzai` 出“现在”），前提是打出的字母本身拼不成正确的拼音。第二个候选是按模糊音最可能的读法（`si` 也给出“是”，`jinglai` 给出“进来”）；哪些模糊音参与在设置里选。

输入拼音时：

- **空格** 或 **1–9** 选候选；**回车** 按原样上屏字母；**Esc** 清空。
- **-** / **=**、**,** / **.**、**PgUp** / **PgDn** 翻页。
- **[** / **]** 取候选词的第一个 / 最后一个字。
- **Shift+字母** 先上屏第一个候选，再上屏大写字母。标点同理：先上屏第一个候选，再上屏标点。
- **Ctrl+Shift+1–9** 删掉学过的词。
- **←** / **→** 逐词回到句子里改字：候选针对实线下划线的那个词，选一个就替换它并跳到下一个词。**空格** 或 **回车** 上屏整句；**Esc** 回到输入，再按一次清空；接着打字母或按退格，就回到句尾继续输入。
- 切到别的窗口时，没打完的拼音直接清空，不上屏。

模式对所有程序同时生效，重启后保留：

- **Shift**（单独按）切换中 / 英。输入拼音时按，字母原样上屏并切到英文。设置里可以改成 Ctrl 或关掉。大写锁定打开时打英文。
- **Ctrl+Shift+F** 切换简 / 繁。
- **Shift+空格** 切换全角 / 半角。全角时字母、数字、符号和空格也变成全角。
- 每次切换都会在光标旁显示新模式约 1 秒，全屏游戏里也看得到。任务栏托盘在 Brushkey 图标旁显示 简 / 繁 / 英：左键点切换中 / 英，右键打开设置。

标点和微软拼音一样：

| 键 | 出 | 键 | 出 | 键 | 出 |
|---|---|---|---|---|---|
| `,` | ， | `\` | 、 | `^` | …… |
| `.` | 。 | `( )` | （ ） | `_` | —— |
| `;` | ； | `< >` | 《 》 | `$` | ￥ |
| `:` | ： | `[ ]` | 【 】 | `~` | ～ |
| `?` | ？ | `!` | ！ | `` ` `` | · |

引号开、闭交替：简体“”‘’，繁体「」『』。紧跟数字的 `.` `,` 保持半角，`3.14` 照常打。

## 设置

右键任务栏托盘里的 简 / 繁 / 英，或从开始菜单打开 **Brushkey 设置**。改动立即保存；已经开着的程序在下次获得焦点时生效。

- 中 / 英切换键：Shift、Ctrl 或关闭
- 候选窗：横排或竖排、每页 3–9 个、字号、简体和繁体字体
- 模糊音（第 2 个候选），11 对（z = zh、n = l、an = ang……，默认开 8 对）
- 清空记忆
- 设置窗口的语言

## 你的数据

学过的词、设置和模式存在 `%APPDATA%\Brushkey`；简体和繁体共用学过的词。数据不会离开你的电脑。

- 密码框里能打字，但选词不记忆。
- 沙盒程序（开始菜单的搜索框、部分商店应用）里能打字，但选词不记忆。
- 在 **设置 → 应用 → 安装的应用** 里卸载。卸载时会问要不要一起删掉学过的词和设置，默认保留。开着的程序还在用的文件，下次重启后删除。

## 从源码构建

需要 Rust（stable，MSVC 工具链）和 32 位目标：`rustup target add i686-pc-windows-msvc`。

```powershell
cargo test --release --workspace                                     # 引擎、按键逻辑、设置文件
cargo run --release -p engine --bin dictgen -- data target/dict.bin  # 词库文件
powershell -File scripts\installer.ps1                               # target\installer\Brushkey-Setup-<版本>.exe
```

- `crates/engine`：拼音引擎，没有依赖：词库、整句转换、记忆、繁体转换。`dictgen` 把 `data/` 编译成 `dict.bin`，输入法以内存映射方式打开它；`cargo run --release -p engine --example bench` 测按键耗时。
- `crates/ime`：TSF 文本服务 `brushkey.dll`（x64 和 x86 各一份）：按键、组合串、Direct2D 候选窗、给游戏用的 UI-less 候选列表、托盘按钮、注册。
- `crates/settings`：设置程序 `brushkey-settings.exe`。
- `sh scripts/dev-install.sh`（以管理员身份运行 Git Bash）编译、复制到 `C:\Program Files\Brushkey` 并注册；`-u` 注销。
- `cargo run -p ime --example host -- tsf "nihao{space}"` 在自己的窗口里通过已安装的输入法打字并打印结果。先把 `APPDATA` 设成临时目录，免得改动你学过的词。`scripts/drive-settings.ps1` 用同样的方式操作设置窗口并截图。
- `scripts/installer.ps1` 需要 [Inno Setup 6](https://jrsoftware.org/isinfo.php)，只保留当前版本的安装包。`scripts/icon.py` 把 `assets/icon.svg` / `icon-small.svg` 渲染成 `app.ico` 和 `app.res`（需要装了 Pillow 的 Python、Google Chrome 和 Windows SDK）。

## 许可

GPL-3.0，见 [LICENSE](LICENSE)。词库来自 [rime-ice](https://github.com/iDvel/rime-ice)（GPL-3.0），繁体转换数据来自 [OpenCC](https://github.com/BYVoid/OpenCC)（Apache-2.0），详见 [data/README.md](data/README.md)。
