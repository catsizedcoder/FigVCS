# Hosting your own server

The `fvcs-server` binary hosts repos for many users over HTTP:
```
fvcs-server --dir /srv/fvcs --port 8080            # add --closed to disable open registration
```

## Contents
- [Flags](#flags)
- [Server data](#server-data)
- [Garbage collection](#garbage-collection)
- [Anti-spam (Proof of Work)](#anti-spam-proof-of-work)

## Flags

| Flag | Default | Purpose |
| --- | --- | --- |
| `--dir <path>` | `.` | Folder that holds all repos and `server.db` |
| `--port <n>` | `8080` | Port to listen on |
| `--closed` | off | Disable open registration |
| `--pol-difficulty <bits>` | `18` | Base proof-of-work puzzle difficulty (`0` disables it) |
| `--pol-adaptive-max <bits>` | `8` | Maximum extra puzzle bits for repeat registrants |
| `--use-proxy-headers` | off | Trust `X-Forwarded-For`. Enable this **only** behind a proxy such as Cloudflare or nginx. |
| `--rate-register <n>` | `5` | Registrations per IP per hour |
| `--rate-login <n>` | `20` | Logins per IP per hour |
| `--rate-push <n>` | `600` | Pushes per IP per hour |
| `--max-pushes-per-day <n>` | `200` | Pushes per account per day (`0` = no limit) |
| `--max-repo-size-mb <n>` | `0` | Maximum size of one repo in MiB, commits plus the pool objects it refers to (`0` = no limit). A push that goes over this limit is refused. |
| `--gc` | off | Run garbage collection one time, then exit |
| `--gc-interval-hours <n>` | `0` | Run garbage collection every N hours (`0` = off) |
| `--gc-grace-hours <n>` | `1` | Never collect objects younger than N hours |

## Server data
The server keeps its data in a SQLite database (`server.db`). Accounts, hashed passwords, and hashed tokens stay on the server. All repos share one object pool. If X amount of users upload the same file, the server stores it one time. Run the server behind an HTTPS reverse proxy for encryption.

## Garbage collection
Object pools grow when branches change and commits become unreachable. The garbage collector cleans this up:
- It walks every branch in every repo and marks the reachable history.
- It deletes commits that no branch can reach.
- It deletes pool objects that no commit refers to.
- It skips objects younger than `--gc-grace-hours`. This keeps in-flight pushes safe.

Run it one time with `fvcs-server --dir /srv/fvcs --gc`. Or let the server run it on a schedule with `--gc-interval-hours 24`. For an external schedule (Task Scheduler or cron), use the `--gc` flag.

Note: when a user deletes a repo, the shared pool objects are not removed immediately. However when the garbage collector is ran it removes objects that no repo refers to. (eg. A lib that is referenced in other repos won't be deleted but a custom script you made that nobody else uses will be deleted from the backend when the garbage collector is ran.)

## Anti-spam (Proof of Work)
The server defends itself against bot floods out of the box:
- **Signed proof-of-work puzzles.** Registration requires the solution of a small sha256 puzzle. The server signs each challenge. Each challenge works one time and expires after 10 minutes. Bots cannot forge, reuse, or pre-compute challenges. The client solves the puzzle automatically in a second or two. Mass registration is resource intensive for each account.
- **Adaptive difficulty.** Each recent registration from the same IP makes the next puzzle harder (up to `--pol-adaptive-max` extra bits). Most users wouldn't notice however bot floods are quite inefficient.
- **Per-IP rate limits.** The server limits registrations, logins, and pushes/IP/hour (see the flag table). Behind Cloudflare or another proxy, enable `--use-proxy-headers`. The limits then use the real client IP from `X-Forwarded-For`, not the proxy IP preventing what would be essentially self DoS against your own server.
- **Per-account daily push quota.** This stops a compromised or spam account before it floods the server's storage.
- **Per-repo size limit.** Set `--max-repo-size-mb` to stop one repo from using too much storage.
