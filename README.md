# prog-scanner

A **read-only** command-line tool that lists the programs installed on a Windows PC.
It never installs, removes, edits, moves, or deletes anything.

## What it lists
- Installed programs from the Windows registry (the "Apps & Features" list, 64-bit, 32-bit and per-user)
- Microsoft Store / MSIX apps
- Folders in `C:\Program Files`, `C:\Program Files (x86)`, `%LOCALAPPDATA%\Programs`
- Chocolatey and Scoop packages (by listing their folders)
- Terminal-installed tools via read-only list queries: `winget list`, `pip list`, `npm ls -g`, `cargo install --list`
- Optional (`--path`): executables found in your PATH folders

## Build (one time)
1. Install Rust from https://rustup.rs (accept the defaults)
2. Open a terminal in this folder and run:

    cargo build --release

3. The program is at `target\release\prog-scanner.exe`

## Use
    prog-scanner.exe                      show everything
    prog-scanner.exe --no-commands        skip winget/pip/npm/cargo queries
    prog-scanner.exe --path               also list executables on PATH
    prog-scanner.exe --save report.txt    also save to a NEW file (never overwrites)
    prog-scanner.exe --pause              wait for Enter before closing

**Double-clicking the .exe:** the window stays open until you press Enter, and a report
is saved automatically as `prog-scanner-report-<date>-<time>.txt` on your Desktop
(or next to the .exe). The exact path is printed on screen.

Tip: `prog-scanner.exe > programs.txt` also works from a normal terminal.
Run it as a normal user; Administrator is not needed.

## Safety notes
- Registry keys are opened with read-only access only.
- Folders are only listed, never touched.
- The only possible write is the optional `--save` report, and it refuses to overwrite existing files.
- You can verify this: the whole tool is one file, `src/main.rs`.

## Run on any Windows PC with one command (needs Node.js)
    npx github:YOUR_USERNAME/prog-scanner
Extra options work too, e.g. `npx github:YOUR_USERNAME/prog-scanner --no-commands`.
It downloads the checked, prebuilt exe to a temp folder, runs it, and deletes it. Nothing is installed.

## Publishing (maintainer)
1. Put this project in a PUBLIC GitHub repo; set `repository.url` in package.json.
2. `git tag v1.1.0 && git push --tags` - GitHub Actions builds the exe and creates the Release.
   (Tag, Cargo.toml version and package.json version must match.)
