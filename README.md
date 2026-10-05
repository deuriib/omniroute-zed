# omniroute-zed

[![ci](https://github.com/deuriib/omniroute-zed/actions/workflows/ci.yml/badge.svg)](https://github.com/deuriib/omniroute-zed/actions/workflows/ci.yml)
[![release](https://github.com/deuriib/omniroute-zed/actions/workflows/release.yml/badge.svg)](https://github.com/deuriib/omniroute-zed/releases)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](./LICENSE-MIT)

Wire [OmniRoute](https://github.com/diegosouzapw/OmniRoute) into [Zed](https://zed.dev) as an LLM provider, and keep the model list in sync automatically.

```
1192 models -> updated C:\Users\deuri\AppData\Roaming\Zed\settings.json [44 curated kept, 1155 added]
```
---

## Hybrid layout: host syncer + companion extension

Root is a native host binary (timers + file writes — things a WASM guest
cannot do). `extension/` is a snippets-only companion Zed can install
locally via `Install Dev Extension`. It ships no Rust/WASM: provider sync
stays in the binary, the extension only carries editor snippets.

Why hybrid: **extensions cannot register LLM providers, run timers, or write
settings.** The binary owns the provider entry and the 5-minute refresh;
the extension owns what Zed lets extensions own.

## How it works

Two verified Zed behaviours do the actual wiring:

| Zed behaviour | Consequence |
|---|---|
| Zed appends `/chat/completions` to `api_url` | We write the `/v1` **root**, not the full endpoint |
| Zed derives the key env var as `format!("{}_API_KEY", id).to_upper_snake()` | Provider id `omniroute` ⇒ Zed reads **`OMNIROUTE_API_KEY`** on its own |

The second one matters most: **Zed reads the API key by itself**. This tool never has to put a secret in your config. It only needs the key to fetch `/v1/models`.

## Install locally

### 1. Host syncer (does the real work)

```sh
mise install
mise run build
mise run install   # cargo install --path . --root ~/.local --force
```

Requires Rust (mise handles it) and a running OmniRoute gateway on `http://localhost:20128`.

Set the key in Zed's environment so Zed can call the provider:

```sh
# PowerShell, for the current user
[Environment]::SetEnvironmentVariable("OMNIROUTE_API_KEY", "sk-...", "User")
```

Then restart Zed.

### 2. Companion extension (snippets only)

In Zed: Extensions -> `Install Dev Extension` -> select `./extension`.
No build step — there is no WASM crate, just `extension.toml` + snippets.

## Usage

```sh
mise run status   # is OmniRoute reachable? how many models?
mise run sync     # one-shot sync into settings.json
mise run watch    # loop, refreshing every 5 minutes (Ctrl-C to stop)
```

Direct binary, if you prefer:

```sh
./target/release/omniroute-zed sync
./target/release/omniroute-zed watch --interval 300
./target/release/omniroute-zed --settings ./my-settings.json sync
```

### Options

| Flag | Env | Default |
|---|---|---|
| `--base-url` | `OMNIROUTE_BASE_URL` | `http://localhost:20128` |
| `--provider-id` | `OMNIROUTE_ZED_PROVIDER_ID` | `omniroute` |
| `--settings` | `OMNIROUTE_ZED_SETTINGS` | per-user Zed path |
| `--write-api-key` | — | **off** |
| `--interval` (watch) | `OMNIROUTE_SYNC_INTERVAL` | `300` |

### Sync interval

Two options, and they serve different purposes:

- **Per-cycle:** `mise run watch -- --interval 60`
- **Task default:** change `OMNIROUTE_SYNC_INTERVAL` in your shell, or edit the `watch` task in `mise.toml`

## Your settings file is treated as precious

Zed's `settings.json` is **JSONC** — real files carry comments, trailing commas and hand-written notes. A strict-JSON round-trip would silently eat all of it.

This tool edits the file's **concrete syntax tree**, so:

- **Comments survive.** Verified on a 790-line file: an untouched parse re-prints byte-for-byte (24 632 bytes in, 24 632 out).
- **Your curation survives.** Models already in your file are never rewritten, so hand-written `display_name`s and the comments explaining them stay put.
- **Nothing is ever deleted.** If a model disappears from `/v1/models`, your entry is left alone. Wiping a config line because of a transient upstream hiccup is far worse than a stale entry Zed simply fails to call.
- **A malformed file is refused, not clobbered.**
- **Atomic writes** — temp file plus rename, so a crash cannot truncate your settings.
- **`.bak` is kept once**, holding the pristine original.

On this machine that meant: your 44 curated models were kept untouched and 1 155 new ones appended.

### Secrets

By default **no secret is written to disk** — verified by grep after a real sync. Zed reads `OMNIROUTE_API_KEY` from its own environment.

`--write-api-key` opts into persisting the key in `settings.json`. That file is plaintext: it ends up in dotfile repos, backups and screen shares. The default is off for that reason.

Pre-existing secrets you wrote yourself are left exactly as they are — this tool only manages what it owns.

## The `localhost` quirk

On Windows, `localhost` resolves to `::1` **first**. If OmniRoute listens on IPv4 only, that first connect stalls about two seconds before refusing.

A single global HTTP timeout let that dead address family eat the entire request budget, which made `localhost` look unreachable. Fixed with per-phase timeouts (resolve 5 s, connect 3 s, total 15 s).

A ~2 s cost per sync remains — inherent to Windows dual-stack resolution, harmless at a 5-minute cadence. To avoid it:

```sh
mise exec -- ./target/release/omniroute-zed --base-url http://127.0.0.1:20128 sync
```

## Design notes

**Determinism.** OmniRoute returns models in a varying order between calls. Writing that order through would rewrite `settings.json` every cycle, and Zed *watches* that file — so a 5-minute loop would trigger pointless reloads forever. Output is sorted, so Zed only reloads when the model *set* actually changes.

**Ordering.** Provider families group alphabetically; within a family, ids sort descending (`auto/reasoning` before `auto/chat`).

**Zed's model schema.** Read from Zed's source, not guessed — and it is **snake_case**, not camelCase. A first draft assumed camelCase; the tests caught it.

**`max_tokens` is required by Zed**, but 340 of 1 192 OmniRoute models report no context window. Those get a conservative default rather than being hidden.

**Refusing to write on an empty model list.** If `/v1/models` returns nothing, the sync fails and your file is left alone.

**This tool does not write `context_servers`.** It does not touch MCP at all. That is not an oversight — an earlier version did register the MCP server, and it cost a real bug. Zed's schema had moved on from the `source`-tagged form I had read, so the entry it wrote was deprecated. Zed's migrator stripped the key on every load and raised a *"Your settings file uses deprecated settings"* banner. A tool that writes one deprecated key gets blamed for every future schema change, so it now writes nothing it does not own.

## Development

```sh
mise run test        # 41 tests
mise run lint-fmt    # rustfmt
mise run lint-clippy # clippy -D warnings
mise run check       # all of the above
mise run clean
```

CI runs fmt, clippy and tests on Linux, Windows and macOS.

Layout:

| File | Role |
|---|---|
| `src/client.rs` | HTTP, timeouts, deterministic sort |
| `src/models.rs` | OmniRoute payload → Zed model schema |
| `src/jsonc_merge.rs` | CST editing — the part that preserves your file |
| `src/settings.rs` | Locate, back up, atomic write |
| `src/sync.rs` | Orchestration |

## Endpoints

| Purpose | URL |
|---|---|
| Models | `http://localhost:20128/v1/models` |
| Chat | `http://localhost:20128/v1/chat/completions` |

If you want OmniRoute's MCP tools inside Zed, add them yourself under `context_servers`. See the note below for why this tool stays out of that file.

## Download (no Rust needed)

Grab the binary for your OS from [Releases](https://github.com/deuriib/omniroute-zed/releases/latest):

| OS | File |
|---|---|
| Windows x64 | `omniroute-zed-x86_64-pc-windows-gnu.zip` |
| Linux x64 | `omniroute-zed-x86_64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `omniroute-zed-aarch64-apple-darwin.tar.gz` |

Then:

```sh
omniroute-zed status
omniroute-zed sync
```

## License

Licensed under either of [MIT](./LICENSE-MIT) or [Apache-2.0](./LICENSE-APACHE), at your option.