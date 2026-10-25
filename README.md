# Brushkey

English | [简体中文](README.zh-CN.md)

A pinyin input method for Windows that types Simplified or Traditional (Taiwan) Chinese, learns your words from a single pick, and works in fullscreen games. The keys work like Microsoft Pinyin, so it can replace it as is.

- **Simplified or Traditional with Ctrl+Shift+F.** The pinyin stays the same. Traditional uses Taiwan character forms and offers Taiwan vocabulary first, ahead of the character-by-character conversion.
- **Learns from one pick.** Pick a word once and it comes first next time. Every app sees new words at once.
- **Games.** Works in fullscreen games, including those that draw the candidate list themselves.
- **Light.** About 0.3 ms per keystroke and a ~500 KB DLL; the 20 MB dictionary is shared by every app rather than loaded into each one.
- **Offline.** Brushkey never connects to the internet. The settings window comes in English and Chinese and follows the system light / dark theme.

## Install

Download `Brushkey-Setup-<version>.exe` from [Releases](https://github.com/ArthurLauCS/Brushkey/releases/latest) and run it. Requires 64-bit Windows 10 or 11.

- The installer is not signed with a commercial code-signing certificate, so Windows may show "Windows protected your PC". Click **More info** → **Run anyway**.
- It asks for administrator permission: input methods are registered for the whole PC.
- Brushkey is added under **Chinese (Simplified, China)**. If Windows doesn't have that language yet (for example, only Chinese (Traditional, Taiwan) is installed), add it first in **Settings → Time & language → Language & region → Add a language**.
- After installing, press **Win+Space** and pick Brushkey. To keep only Brushkey, remove Microsoft Pinyin in **Language & region → Chinese (Simplified, China) → … → Language options → Keyboards**.
- When updating, apps that are already open keep the old version until you restart them.

## Typing

Type a whole sentence in one go: the first candidate is the likeliest sentence, followed by words for its first syllables. Initials work too (`zg` for a two-character word whose syllables start with z and g).

Two adjacent letters typed in swapped order are corrected (`xainzai` reads as `xianzai`), as long as the letters as typed don't spell valid pinyin. The second candidate is the likeliest reading through fuzzy pinyin (`si` also offers the word for `shi`); settings choose which pairs count.

While typing pinyin:

- **Space** or **1–9** picks a candidate; **Enter** types the letters as they are; **Esc** clears.
- **-** / **=**, **,** / **.** and **PgUp** / **PgDn** turn the page.
- **[** / **]** type the first / last character of a candidate.
- **Shift+letter** types the first candidate, then the capital letter. Punctuation likewise types the first candidate, then the punctuation.
- **Ctrl+Shift+1–9** forgets a learned word.
- **←** / **→** go back over the sentence word by word: the candidates are for the word with the solid underline, and a pick replaces it and moves to the next word. **Space** or **Enter** types the sentence; **Esc** goes back to typing, a second **Esc** clears; letters and **Backspace** carry on at the end.
- Switching to another window clears what you were typing; nothing is entered.

Modes apply to every app at once and are kept after a restart:

- **Shift** (pressed alone) toggles Chinese / English. Pressed while typing pinyin, it enters the letters and switches to English. Settings can change this key to Ctrl, or turn it off. With Caps Lock on you type English.
- **Ctrl+Shift+F** toggles Simplified / Traditional.
- **Shift+Space** toggles full / half width. Full width also makes letters, digits, symbols and the space full-width.
- Every switch shows the new mode next to the caret for a second, which you can see in fullscreen games too. The taskbar tray shows the mode next to the Brushkey icon: click it to toggle Chinese / English, right-click it for settings.

Punctuation follows Microsoft Pinyin: full-width marks, Chinese brackets, and quotes that alternate between opening and closing (corner brackets in Traditional). Right after a digit, `.` and `,` stay half-width, so `3.14` comes out as typed.

## Settings

Right-click the mode button in the taskbar tray, or open **Brushkey Settings** from the Start menu. Changes are saved at once; apps that are already open pick them up the next time they get focus.

- Chinese / English toggle key: Shift, Ctrl or off
- Candidate window: horizontal or vertical, 3–9 per page, font size, Simplified and Traditional fonts
- Fuzzy pinyin for the second candidate, 11 pairs (z = zh, n = l, an = ang, …; 8 on by default)
- Clear memory
- Language of the settings window

## Your data

Learned words, settings and modes are kept in `%APPDATA%\Brushkey`; Simplified and Traditional share the learned words. Nothing leaves your PC.

- In password fields you can type, but picks are not learned.
- In sandboxed apps (the Start menu search box, some Store apps) you can type, but picks are not learned.
- Uninstall in **Settings → Apps → Installed apps**. It asks whether to delete the learned words and settings too; the default keeps them. Files that open apps still use are removed at the next restart.

## Build from source

Needs Rust (stable, MSVC toolchain) with the 32-bit target: `rustup target add i686-pc-windows-msvc`.

```powershell
cargo test --release --workspace                                     # engine, key handling, settings file
cargo run --release -p engine --bin dictgen -- data target/dict.bin  # the dictionary file
powershell -File scripts\installer.ps1                               # target\installer\Brushkey-Setup-<version>.exe
```

- `crates/engine` is the pinyin engine, with no dependencies: dictionary, sentence conversion, memory, Traditional conversion. `dictgen` compiles `data/` into `dict.bin`, which the IME memory-maps; `cargo run --release -p engine --example bench` times keystrokes.
- `crates/ime` is the TSF text service, `brushkey.dll` (built for x64 and x86): keys, composition, the Direct2D candidate window, the UI-less candidate list for games, the tray button and registration.
- `crates/settings` is the settings app, `brushkey-settings.exe`.
- `sh scripts/dev-install.sh` (Git Bash, as administrator) builds, copies to `C:\Program Files\Brushkey` and registers; `-u` unregisters.
- `cargo run -p ime --example host -- tsf "nihao{space}"` types into its own window through the installed IME and prints the text. Set `APPDATA` to a scratch folder first so your learned words stay untouched. `scripts/drive-settings.ps1` drives the settings window the same way and takes screenshots.
- `scripts/installer.ps1` needs [Inno Setup 6](https://jrsoftware.org/isinfo.php) and keeps only the current version's installer. `scripts/icon.py` renders `assets/icon.svg` / `icon-small.svg` into `app.ico` and `app.res` (needs Python with Pillow, Google Chrome and the Windows SDK).

## License

GPL-3.0, see [LICENSE](LICENSE). The dictionary comes from [rime-ice](https://github.com/iDvel/rime-ice) (GPL-3.0) and the Traditional conversion data from [OpenCC](https://github.com/BYVoid/OpenCC) (Apache-2.0); see [data/README.md](data/README.md).
