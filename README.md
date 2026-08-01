<img src="assets/banner_large.png" alt="Large Banner" height="400">

[![CC BY-NC-SA 4.0][cc-by-nc-sa-shield]][cc-by-nc-sa]

# What is FigVCS anyway?
FigVCS is a (VCS) Version Control System built specifically for managing [FiguraMC](https://github.com/FiguraMC/Figura) avatars.

## Installation
You do **not** need to know how to code or install any developer tools. Pick your system below.

### Windows (easiest)
1. Press `Win + X` and choose **Terminal** (or **Windows PowerShell**).
2. Paste this one line and press Enter:
   ```powershell
   irm https://raw.githubusercontent.com/catsizedcoder/FigVCS/main/install.ps1 | iex
   ```
3. Close the terminal and open a new one. Done! Try it: `fvcs --help`

the script only downloads the FigVCS binary from this repository and adds it to your PATH.

### Linux
```sh
curl -sSfL https://raw.githubusercontent.com/catsizedcoder/FigVCS/main/install.sh | sh
```

### Prefer clicking?
Grab `fvcs-windows-x86_64.zip` (or the Linux tarball) from the [Releases page](https://github.com/catsizedcoder/FigVCS/releases), unzip it anywhere, and run `fvcs` from that folder.

### Building from source (devs/contributors)
Requires a [Rust toolchain](https://rustup.rs):
```
cargo build --release
```
The binary is built to `target/release/`.

## Quick start
```
cd path/to/your/avatar     # the folder containing avatar.json
fvcs init                  # create the repository (.fvcs/)
fvcs add .                 # stage all files
fvcs commit -m "initial"   # record the first version
```
Everyday commands:
- `fvcs status` | staged / unstaged / untracked changes
- `fvcs diff` | workdir vs staged; `fvcs diff --cached` | staged vs HEAD; `fvcs diff <a> <b>` | between commits/branches/tags
- `fvcs log [--oneline]` | history
- `fvcs branch`, `fvcs checkout <branch|commit>`, `fvcs tag <name>` | branches and tags
- `fvcs restore <paths>` / `fvcs restore --staged <paths>` | discard workdir changes / unstage

The storage is content-addressed and zlib-compressed (`.fvcs/objects/`), preventing the same file from being stored twice which means versioning an avatar costs a fraction of its size.

Files listed in `.fvcsignore` (Formatted in the same way as git.) are excluded from tracking.

## Plans for this project
- Establish a system that is low on storage usage but effective.
- Establish an efficient system so that if a hub is created for avatars the local versioned repos can be uploaded with ease.
- Add easy library linking to have libraries update automatically with something like `fvcs pull`
- Eventually a Desktop GUI with Linux and Windows support(Possibly even MacOS support) since I know not everyone prefers CLI
- A Minecraft Companion mod for FigVCS that allows for dynamically saving versions of avatars and managing versions within [FiguraMC](https://github.com/FiguraMC/Figura)
- Add the support for automatic backups to a server if configured, allowing for avatars to be restored even if the user encounters hardware failure.
- Create a system that is similar to git except more user friendly while also not compromising usability and allowing for in depth usage for automation scripts and such.

## Copyright

This work is licensed under a
[Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International License][cc-by-nc-sa]. In other words you **may** share and modify the source code, however you may **not** use the code for commercial purposes, you must distribute your contributions under the same license as the original, you may not apply legal terms or [technological measures](https://creativecommons.org/licenses/by-nc-sa/4.0/#ref-technological-measures) that legally restrict others from doing anything the license permits, and you may not use the code without giving [proper credit to me](https://creativecommons.org/licenses/by-nc-sa/4.0/#ref-appropriate-credit), you must indicate if changes were made. You may do so in any reasonable manner, but not in any way that suggests that I endorse you or your use. For more details please [read the page on creativecommons.org](https://creativecommons.org/licenses/by-nc-sa/4.0/) 

[cc-by-nc-sa]: http://creativecommons.org/licenses/by-nc-sa/4.0/
[cc-by-nc-sa-shield]: https://img.shields.io/badge/License-CC%20BY--NC--SA%204.0-lightgrey.svg
