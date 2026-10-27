# dunk

A Mason-inspired Rust terminal UI for Homebrew. Browse formulae and casks in one list, filter installed or outdated packages, search by name, inspect package details, and install, upgrade or remove packages.

## Install with Homebrew

```sh
brew install lesliesrussell/tap/dunk
dunk
```

The formula lives in [lesliesrussell/homebrew-tap](https://github.com/lesliesrussell/homebrew-tap). Prebuilt bottles are available for Apple Silicon macOS and x86_64 Linux. Systems without a compatible bottle build the checksummed release source with Rust. To update or remove:

```sh
brew update
brew upgrade lesliesrussell/tap/dunk
brew uninstall dunk
```

## Run from source

Requires Rust and Homebrew on your PATH.

```sh
just run
```

Install the executable with `just install`. Run `just brew-check` for a read-only check of the Homebrew catalog. See setup instructions below.

## Keys

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Select package |
| `/` | Search; Enter or Escape finishes editing |
| `1` / `2` / `3` | All / installed / outdated |
| Enter | Load package information and caveats |
| `i` | Install selected package |
| `Space` | On Installed: mark/unmark the focused package |
| `u` | On Installed: upgrade marked packages, or the focused package if none are marked |
| `U` | On Installed: upgrade all installed packages |
| `X` | Remove selected installed package |
| `y` | Confirm displayed Brew command |
| `r` | Reload catalog and installed state |
| `q`, Escape, Ctrl-C | Quit when idle |

On **Installed** (`2`), use `Space` to mark multiple packages and `u` to upgrade them. With no marks, `u` upgrades the focused row. Marks persist when searching or switching tabs; the marked count appears above the list. Press `Space` on a marked row to unmark it. Marked upgrades include hidden marks and group formulae/casks into separate Brew commands; execution stops on the first failed command and refreshes installed state. Outside Installed, `u` still upgrades only the focused installed package.

Press `U` on Installed to run `brew upgrade` for all installed packages eligible under [Homebrew's standard upgrade rules](https://docs.brew.sh/Manpage#upgrade-options-installed_formulainstalled_cask-). This ignores marks and search filters, respects pinning and normal cask exclusions, and does not force upgrades. Both upgrade actions show the scope and commands for confirmation (`y`); any other key cancels. Brew may also upgrade required dependencies. Output streams in the Brew pane and installed state refreshes afterward.

Every package change requires confirmation. Brew runs in the background with output shown in the UI; actions are serialized and quitting is disabled during active commands. Installed state is refreshed after each action. Brew auto-update is disabled for these commands; use `brew update` separately to update metadata.

Formulae and casks are explicitly distinguished when invoking Brew. Custom-tap installed packages are included. Package details come from `brew info`. Search matches package names. The output pane shows the most recent lines (up to 1,000 retained in memory); details are clipped to the terminal height.

Casks requiring interactive password entry cannot prompt inside dunk. Run those commands in a normal terminal. Homebrew errors are shown in the output pane.

`DUNK_BREW` can point to another Brew executable, including a test double.

## Development and maintenance

The top-level `justfile` is the command interface (validated with just 1.59.0). It delegates to Cargo and the existing Python smoke test; no Python packages are needed. Run `just` to list commands. Recipes run from the repository root even when invoked from a subdirectory.

### Prerequisites and fresh checkout

Supported workflow: macOS or Linux with Homebrew and a Unix shell. GitHub Actions validates the Rust checks, PTY test, and release build on macOS and Linux. Homebrew casks are macOS-specific. Native Windows is unsupported by the PTY test.

Install prerequisites explicitly, outside setup:

- Install Rust/Cargo using [rustup](https://rust-lang.org/tools/install/), then run `rustup component add rustfmt clippy`.
- Install Homebrew and Python 3 if absent. With Homebrew available, `brew install just python` provides the command runner and test interpreter. Alternative just installation: `cargo install just --locked`; see the [just installation guide](https://github.com/casey/just#installation).
- Keep the installed tools on `PATH`. A native compiler/linker is needed by Cargo (on macOS, install Apple Command Line Tools if missing).

From a fresh checkout:

```sh
just setup           # checks prerequisites, fetches Cargo.lock dependencies
just check           # formatting, Clippy, type-checking, Rust unit tests
just e2e             # builds debug binary, runs fake-Brew PTY smoke test
just run             # starts the interactive TUI
```

`setup` never installs global tools or changes the lockfile. There are no local configuration templates, hooks, submodules, generated sources, or supporting services to initialize. `just deps` repeats only the locked dependency fetch. Fetching may require network access; once dependencies are cached, use `CARGO_NET_OFFLINE=true just ci` for offline verification. Help, diagnostics, and formatting checks do not require network access.

### Commands

| Group | Commands |
| --- | --- |
| Discovery | `just`, `just doctor`, `just versions` |
| Setup | `just setup`, `just deps` |
| Development | `just run`, `just run --help`, `just brew-check` |
| Quality | `just fmt`, `just fmt-check`, `just lint`, `just typecheck`, `just check` |
| Tests | `just test`, `just test filters_distinguish`, `just test -- --nocapture`, `just e2e` |
| Build | `just build`, `just release`, `just docs` |
| Full validation | `just ci` |
| Installation | `just install`, `just uninstall` |
| Cleanup | `just clean` |

`fmt` and `fmt-check` cover both the justfile and Rust sources. `check` is the fast pre-commit gate. `ci` reuses `check` and `e2e` without real Brew access, interaction, or installation. GitHub Actions runs `just ci` and `just release` on macOS and Linux. The end-to-end test builds explicitly into this checkout's `target/` because the existing smoke script expects `target/debug/dunk`. It uses a temporary fake Brew executable and never changes installed packages.

`run` forwards application arguments; `test`, `build`, and `release` forward Cargo arguments as individually quoted positional arguments. For example, `just build --offline` or `just test renders_empty -- --nocapture`. App arguments beyond `--help` and `--check` are not currently supported by dunk. No watch or coverage tool is configured, so none is implicitly installed.

### Build, install, uninstall, and cleanup

```sh
just release                   # target/release/dunk under normal Cargo settings
just docs                      # target/doc/dunk/index.html under normal Cargo settings
just install                   # Cargo's normal user-local installation root
just uninstall                 # remove only Cargo-tracked dunk
just install "$HOME/.local"     # custom Cargo root; binary goes in ~/.local/bin
just uninstall "$HOME/.local"   # use the identical root when removing
just clean                     # remove only checkout target/ artifacts
```

Installation uses `cargo install --locked --path . --bin dunk`; uninstallation uses Cargo's own installation records and removes only its tracked `dunk` binary. It does not remove the shared root or user configuration/data. No `--force` is used; Cargo refuses unrelated binary collisions. Repeating uninstall after removal reports Cargo's not-installed error. There is no separate packaging or staging convention, so `PREFIX`/`DESTDIR` are not invented: the optional `root` parameter is Cargo's native installation prefix and includes its tracking metadata. Add its `bin` directory to `PATH` yourself.

Configuration:

- `just install ROOT` / `just uninstall ROOT`: optional matching Cargo installation roots, including paths containing spaces. With no argument, Cargo chooses its root using `CARGO_INSTALL_ROOT`, Cargo configuration, `CARGO_HOME`, or the default `~/.cargo`, in Cargo's normal precedence.
- `CARGO_TARGET_DIR` and other Cargo environment/configuration settings apply to ordinary Cargo recipes. `e2e` and `clean` explicitly use checkout `target/`; custom build directories are not cleaned.
- `CARGO_NET_OFFLINE=true`: prevent Cargo network access when dependencies are already cached.
- `DUNK_BREW`: runtime Brew executable override; `doctor` checks standard Homebrew on `PATH`, while `e2e` sets its own test double.

**Consequences:** `install` writes to the selected Cargo root and `uninstall` removes the tracked executable there. `clean` deletes reproducible artifacts in `target/`, including generated documentation, but preserves Cargo's global download cache, configuration, and user data. Keep non-build files out of `target/`. There is no destructive reset command. Running the app can change Homebrew packages only after confirmation; `brew-check` only reads the catalog but Homebrew may fetch metadata. No recipes publish, deploy, update dependencies, or install system prerequisites.

There are no database, container, benchmark, code-generation, or deployment workflows in this repository, so no recipes for these operations are provided.

## Releases and Homebrew maintenance

Versions are recorded in `Cargo.toml` and `Cargo.lock`. Run `just ci` and `just release` before tagging a release. Publish a GitHub release from the matching immutable `vVERSION` tag, with a `dunk-VERSION.tar.gz` source archive produced by `git archive` from that tag (including a `dunk-VERSION/` prefix). Calculate its SHA-256 and update the tap's `Formula/dunk.rb` URL and checksum together. Never replace assets or move tags for an already published version; publish a new version instead.

The tap has its own Homebrew validation workflow. Source releases do not require a registry token or crates.io publication. Publishing a release and changing the tap affect all users and are explicit maintainer actions, separate from `just ci`. The `just release` recipe only builds locally; it does not publish.

