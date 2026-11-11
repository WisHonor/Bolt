# 142 — Bolt sends nothing, and each build proves it

- **Status:** Accepted
- **Date:** 2026-11-11
- **Agenda topic:** none (the v0.1 release: the package)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 141](141-the-user-agreement.md) (what the agreement says Bolt sends); [decision 022](022-engineering-rules.md) (the engineering rules, which gain this check)

## Context

**In plain words.** The user agreement tells people that Bolt sends nothing anywhere: their projects, file names and habits stay on their computer. A promise like that is worth only what stands behind it. Today nothing does: v0.1 happens to have no networking code, but a new dependency could bring some without anyone noticing, and a closed program can't be read by its users to check.

**Technically.** No source file of Bolt's uses `std::net`; no crate in its build is a networking crate; and the release `bolt.exe` of 2026-11-09 imports no network library and no call that starts another program (read with `dumpbin /IMPORTS`). The program never runs another program either: every `Command::new` in the crates' sources is in tests, and Bolt never runs Git ([decision 039](039-first-save-after-import.md)).

## Options

- **A. Say it in the agreement, and trust it.** Nothing stops a dependency update from bringing networking code.
- **B. Rules on the source:** clippy refuses `std::net`'s types in Bolt's code, and cargo-deny refuses known networking crates. Catches the likely ways in, on every push, but not code a dependency carries without a telling name.
- **C. B, plus a check of each program that is built for release:** the program's own list of what it imports from the system is read, and the build fails if it holds anything that reaches a network or starts another program. The check looks at what the program can do, whatever source it came from.

## Decision

**C.**

- **Clippy** (`clippy.toml`, `disallowed-types`) refuses `std::net::TcpStream`, `TcpListener` and `UdpSocket` anywhere in the workspace, tests included, with the reason "Bolt sends nothing (decision 142)".
- **cargo-deny** (`deny.toml`, `[bans] deny`) refuses the crates whose purpose is the network: HTTP clients and servers, TLS, async runtimes' network layers, DNS resolvers, Git's network transports and error-reporting services (the list is in `deny.toml`, each with that reason). Both run in CI on every push and pull request.
- **Each release program** is checked by `.github/release/no-network.sh` in the release workflow, right after it is built, and its build fails when the program imports:
  - **on Windows,** anything from a network library (`ws2_32`, `wsock32`, `mswsock`, `winhttp`, `wininet`, `urlmon`, `dnsapi`, `iphlpapi`, `httpapi`, `websocket`, `webio`, `wldap32`, `mpr`, `netapi32`), or a call that starts a program (`CreateProcess…`, `ShellExecute…`, `WinExec`);
  - **on Linux and macOS,** a socket or name-lookup call (`socket`, `connect`, `bind`, `listen`, `accept`, `accept4`, `send`, `sendto`, `sendmsg`, `sendmmsg`, `recv`, `recvfrom`, `recvmsg`, `recvmmsg`, `getaddrinfo`, `getnameinfo`, `gethostbyname…`, `gethostbyaddr…`, `res_…query`, `res_…search`), or a call that starts a program (`fork`, `vfork`, `exec…`, `posix_spawn…`, `system`, `popen`); and on macOS a network framework (`CFNetwork`, `Network`, `SystemConfiguration`).
- **The check proves itself first.** Before it reads Bolt, it is run on two tiny programs built on the spot, one that opens a TCP connection and one that starts a program, and must refuse both; so a check that has stopped seeing anything fails instead of passing. The full list of what Bolt imports is printed in the log.
- **Starting another program counts.** Bolt could otherwise send data through one (a `curl` or a `git push`), and a tool that never runs a program can't be made to run one by a project it was given ([design.md](../design.md) §2).

## Consequences

- The agreement's sentence "Bolt sends nothing" rests on a check that runs on every program the owner publishes.
- **Known limit:** a program can load a library by name while it runs (`LoadLibrary`, `dlopen`) and reach the network through it without importing anything the check knows. Rust's standard library itself looks up some of the system's own functions that way, so the check can't refuse the lookup; clippy and cargo-deny guard the source against it, and nothing in Bolt does it.
- **A feature that needs the network** (syncing, planned for v0.3), or that starts a program (an editor for messages, say), first needs a decision that changes this one, a new user agreement that says what is sent, and a change to the check's lists.
- Reading a project on a network drive or in a synced folder moves data over a network, but the operating system does that, not Bolt; the agreement says so.
