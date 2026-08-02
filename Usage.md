# Usage

Just finding this project? Do the [Quick start](README.md#quick-start) first.

## Contents
- [Command reference](#command-reference)
- [Syncing & backups (remotes)](#syncing--backups-remotes)
- [Accounts & permissions](#accounts--permissions)
- [Library linking](#library-linking)
- [The central registry](#the-central-registry)
- [How storage works](#how-storage-works)

## Command reference

| Command | What it does |
| --- | --- |
| `fvcs init` | Create a repository in the current folder |
| `fvcs add <paths>` | Stage files (also stages deletions) |
| `fvcs status` | Staged / unstaged / untracked changes |
| `fvcs commit -m "msg"` | Record a new version |
| `fvcs log [--oneline] [-n N]` | Show history |
| `fvcs diff [--cached] [a] [b]` | Differences between workdir, index, commits |
| `fvcs checkout <branch\|commit>` | Switch branches or inspect an old commit |
| `fvcs restore [--staged] <paths>` | Discard changes / unstage |
| `fvcs branch` / `fvcs tag` | Manage branches and tags |
| `fvcs remote ...` | Link folder or server remotes |
| `fvcs push` / `fvcs pull` / `fvcs clone` | Sync with a remote |
| `fvcs login <server> [--register]` | Log in on a FigVCS server |
| `fvcs config user.name/email` | Set your commit identity |
| `fvcs share <user> [--remove]` | Grant/revoke push access on your repo |
| `fvcs remote delete <name> [--yes]` | Delete the repo on the server (owner only) |
| `fvcs lib ...` | Link external libraries |
| `fvcs registry-url` + `fvcs sync` | Fetch libraries from the central registry |
| `fvcs registry <dir>` | (Hosts) rebuild `registry.json` |

Everyday usage:
- `fvcs status` | staged / unstaged / untracked changes
- `fvcs diff` | workdir vs staged; `fvcs diff --cached` | staged vs HEAD; `fvcs diff <a> <b>` | between commits/branches/tags
- `fvcs log [--oneline]` | history
- `fvcs branch`, `fvcs checkout <branch|commit>`, `fvcs tag <name>` | branches and tags
- `fvcs restore <paths>` / `fvcs restore --staged <paths>` | discard workdir changes / unstage

## Syncing & backups (remotes)
A remote can be a plain folder (shared drive, USB stick) or a FigVCS server over HTTP. Point FigVCS at it once, then push/pull:
```
fvcs remote add origin "D:/backups/my-avatar"          # folder remote
fvcs remote add origin "https://fvcs.example.com/my-avatar"   # server remote
fvcs push                                       # upload your commits
fvcs pull                                       # download + update libraries
fvcs clone "https://fvcs.example.com/my-avatar" restored   # restore onto a new machine
```
Rules for `push` and `pull`:
- `push` stops if the remote has commits that you do not have. Run `fvcs pull` first.
- `pull` moves your branch forward when the remote is ahead.
- `pull` stops if the two sides have diverged. FigVCS cannot merge yet.

On every push, FigVCS reads `avatar.json` and generates a `README.md` (avatar name, description, authors) on the remote so repos stay browsable. Don't want that? `fvcs push --no-readme` once and it's remembered in `.fvcs/config.json` (`no_readme`) for all future pushes.

## Accounts & permissions
- Users self-register and log in: `fvcs login https://your-host --register -u name` (password is prompted). Tokens are stored in `~/.fvcsconfig`.
- Set your identity once (derived from gits method.): `fvcs config user.name "Your Name"` and `fvcs config user.email "you@example.com"` | it becomes the author on every commit.
- New repos are **private**. Only the owner can read them and push to them. To let everyone read and clone a repo, run `fvcs remote visibility origin public`.
- To let another user push to your repo, run `fvcs share their-username`. To remove that permission, run `fvcs share their-username --remove`.
- Only the owner can delete a repo from the server: `fvcs remote delete origin`. FigVCS asks you to type the repo name before it deletes. Use `--yes` to skip this question. The remote stays in your local config; a later `push` makes the repo again.

## Library linking
Keep shared Lua libraries in their own repos and drop the latest version into any avatar:
```
fvcs lib add FOXAPI path/to/foxapi-repo            # links the whole repo
fvcs lib add FOXAPI path/to/repo --subdir modules  # or just one subfolder
fvcs pull                                          # libraries update automatically
```
Links live in `.fvcslibs` (a plain JSON file you can commit and share). `fvcs lib` lists them, `fvcs lib update` refreshes on demand, `fvcs lib remove <name>` unlinks.

## The central registry
Instead of linking every library by hand, point FigVCS at a registry and sync everything in one command:
```
fvcs registry-url https://fvcs.example.com
fvcs sync
```
The registry is a `registry.json` listing libraries, their sources, and every known version hash. `fvcs sync` links new libraries and updates existing one, However **if you edited a library locally** (your files match no known registry hash) it leaves your copy alone and tells you, instead of overwriting your work which could've solved an issue. `fvcs sync --force` overwrites your edits when you actually want to.

For registry hosts: put your library repos in one folder (mirrored from GitHub etc., e.g. [FOX's Figura APIs](https://github.com/Bitslayn/FOX-s-Figura-APIs)), then rebuild the registry daily:
```
fvcs registry /srv/fvcs        # writes /srv/fvcs/registry.json, keeping old hashes
fvcs-server --dir /srv/fvcs    # serves repos + registry.json together
```
Schedule `fvcs registry` with Task Scheduler/cron and the registry stays fresh on its own.

## How storage works
- FigVCS names each file version with the sha256 hash of its content. It compresses the content with zlib. It stores the result in `.fvcs/objects/`.
- If two files have the same content, FigVCS stores that content one time. Versioning an avatar costs a fraction of its size.
- Each commit is a small JSON file. It points to a tree of blob hashes.
- On a server, all repos share one object pool. Run the garbage collector to remove objects that no commit uses (see [Garbage collection](HOSTING.md#garbage-collection)).
- `.fvcs/config.json` holds repo settings: remotes, `no_readme`, and the registry URL. `~/.fvcsconfig` holds your identity and your server tokens.
