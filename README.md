# auxtools

auxtools is a Rust-based successor to the [extools](https://github.com/MCHSL/extools) project.

Currently, it implements some of the features such as the debug server, but is planned to supplant most functionality (maptick, etc.).

## Features
Code Coverage - A tool to generate cobertura code coverage XML reports for a codebase. Note that, due to a [BYOND issue](http://www.byond.com/forum/post/108025) it requires some additional work to have properly un-mangled file names in the report.

Debug Server - Debug DM worlds from [VS Code](editors/vscode) or the command line, with experimental hot reload.

## The debugger

The debugger supports line, function, instruction and data breakpoints; conditional breakpoints and logpoints; runtime error breakpoints; line and instruction stepping; sleeping proc stacks; variable inspection and assignment; expression and proc-body evaluation; and disassembly. A journal records runtime errors, reloads, output and reboots.

No changes to the world's source are required. `auxtools` launches a compiled `.dmb` in DreamSeeker, DreamDaemon or `dd.exe` with the debug library loaded. Build the world with your project's usual tools. After `world.Reboot()`, the launcher reconnects and restores breakpoints and reloads.

Multiple debuggers can connect to the same world. Each session has independent breakpoints and runtime error settings. Pausing stops the world for every session; any session can resume it.

Use `auxtools list` to find running worlds. Worlds using an incompatible debugger protocol are listed but cannot be attached to.

For VS Code, install the extension and press F5 in a workspace containing a built world. See the [extension README](editors/vscode/README.md) for configuration and debugging features.

For command-line use, run `auxtools help COMMAND` for options, behavior, output and exit codes. `--pid PID` selects a world when several are running. Requests time out after 30 seconds by default, with exit code 2. Use `--response-timeout SECONDS` to change the limit. Results go to stdout and diagnostics to stderr. `--json` returns a JSON value, or JSON Lines for `log` and `trace`.

```sh
auxtools launch tgstation.dmb --detach
auxtools wait pause --break code/game/foo.dm:120
auxtools vars
auxtools eval 'src.health'
auxtools step-over
auxtools continue
auxtools trace /mob/proc/Life --by caller --for 5
auxtools log --runtimes --follow
auxtools wait reload
auxtools reboot
auxtools stop
```

Exit codes are 0 for success, 1 for failure, 2 for timeout, 3 if the world exits or reboots before completion, and 4 if a world launched in the foreground exits unsuccessfully.

Editors use `auxtools dap` as the debug adapter.

## Hot reload (experimental)

The launcher watches source files and reloads changes when they are saved. Hot reload is enabled by default for launched worlds, including **Run Without Debugging**. Supply the same `defines` used to build the `.dmb`.

Hot reload supports proc bodies, proc and verb additions, proc removals, new types and variables, variable types and defaults, `static`, `const` and `tmp` declarations, parameters and `set` statements. Changes to parent types or proc declarations, such as converting a proc to a verb, require a rebuild.

Existing calls finish on their original bytecode. Existing instances retain their verbs and built-in values; computed defaults apply to new instances. After reboot, reloads are restored before world code runs. Reloads introducing resources absent from the build wait until `world/New`, so global initializers and map atoms run the built code.

Proc-body edits can use a single-file parse. Other edits trigger a full project parse. See the [extension documentation](editors/vscode/README.md#hot-reload-experimental) for reload limits and the [hot reload tests](tests/hot_reload_test) for supported cases.

## Supported BYOND versions

Windows and Linux: 515.1637–515.1647 and 516.1648–516.1688.

Symbol patterns were checked throughout these ranges. Newer builds are attempted but may fail initialization if a pattern no longer matches. Older builds and unsupported major versions are rejected with a version error.

The patterns use [byond-scan](https://github.com/Absolucy/byond-scan) and are stored in `auxtools/src/symbols/`.

CI tests the debugger, hot reload and CLI on 515.1637, 515.1647, 516.1659, 516.1685 and 516.1688. The integration suites cover the debug server, source reloads, launcher lifecycle, CLI output and DAP behavior. Linux tests run on Ubuntu 22.04; Windows tests run in `dd.exe` on Windows Server 2022 and under Wine.

The VS Code extension has separate integration coverage on Linux 516.1688 using a harness outside this repository. It has no automated Windows coverage. DreamSeeker and windowed DreamDaemon were checked manually with the first part
of `launcher_test` before the CLI was added; they are outside these automated runs. The `auxtest` suite and coverage reports were last checked on 516.1687.

## Dependencies

*These instructions were taken directly from tgstation's [rust-g](https://github.com/tgstation/rust-g) documentation.*

The [Rust] compiler:

1. Install the Rust compiler's dependencies (primarily the system linker):

   * Ubuntu: `sudo apt-get install gcc-multilib`
   * Windows (MSVC): [Build Tools for Visual Studio 2017][msvc]

1. Use [the Rust installer](https://rustup.rs), or another Rust installation method,
   or run the following:

    ```sh
    curl https://sh.rustup.rs -sSfo rustup-init.sh
    chmod +x rustup-init.sh
    ./rustup-init.sh
    ```

1. Add the **32-bit** compilation target:

    ```sh
    # Clone the `auxtools` repository to a directory of your choice
    git clone https://github.com/auxtools/auxtools
    # in the `auxtools` directory...
    cd auxtools
    # Linux
    rustup target add i686-unknown-linux-gnu
    # Windows
    rustup target add i686-pc-windows-msvc
    ```

System libraries:

* Ubuntu and Debian users run:

    ```sh
    sudo dpkg --add-architecture i386
    sudo apt-get update
    sudo apt-get install build-essential g++-multilib libc6-i386 libstdc++6:i386
    ```

* Other Linux distributions install the appropriate **32-bit development** and **32-bit runtime** packages.

## Compiling

Cargo downloads and builds Rust dependencies. The lockfile pins two git dependencies:
[auxtools/dmasm](https://github.com/auxtools/dmasm) for proc compilation and hot reload, and
[auxtools/SpacemanDMM](https://github.com/auxtools/SpacemanDMM) for parser fixes.

Build the workspace in release mode:

Linux:

```sh
cargo build --workspace --release --target i686-unknown-linux-gnu
```

Outputs in `target/i686-unknown-linux-gnu/release`: `libauxtools.so`, `libdebug_server.so`, `libauxcov.so`, `auxtools`.

Windows:

```sh
cargo build --workspace --release --target i686-pc-windows-msvc
```

Outputs in `target/i686-pc-windows-msvc/release`: `auxtools.dll`, `debug_server.dll`, `auxcov.dll`, `auxtools.exe`.

To cross-compile Windows builds on Linux, install clang 19 or newer, lld and LLVM, then fetch the SDK (about 800 MB):

```sh
ci/msvc/fetch-sdk.sh ~/.cache/msvc-sdk
export MSVC_SDK=~/.cache/msvc-sdk
. ci/msvc/env.sh
cargo build --workspace --release --target i686-pc-windows-msvc
```

## Continuous integration

`.github/workflows/ci.yml` builds, tests and packages both platforms. It runs on pull requests and pushes to `master` in the willox and auxtools organizations; other forks can run it manually.

Linux binaries target glibc 2.35 and later. Windows binaries target Windows 10 and later and link the C runtime statically. `ci/check-linux.sh` and `ci/check-windows.sh` check these requirements.

CI uses pinned Rust and action versions. Format code with `cargo +nightly-2026-10-24 fmt --all`; the rustfmt configuration requires nightly options.

BYOND test archives come from the [BYOND build mirror](https://byond-builds.dm-lang.org), verified against `ci/byond.sha256`. To run the integration suites locally, use `ci/run-tests.sh PROGRAMS BYOND`.

| Artifact | Contents |
| --- | --- |
| `auxtools-linux.tar.gz` | Launcher and debug library |
| `auxtools-linux-symbols.tar.gz` | `auxtools.debug`, `libdebug_server.so.debug`; place beside the binaries for gdb |
| `auxtools-windows.zip` | Launcher and debug library |
| `auxtools-windows-symbols.zip` | `auxtools-cli.pdb`, `debug_server.pdb` |
| `auxtools-VERSION.vsix` | VS Code extension with Linux and Windows binaries |

## Releasing

Run **Actions > Release > Run workflow** on `master` and enter a version such
as `3.0.0`. The workflow builds and tests that version before publishing the
GitHub release and `sohpea.auxtools` extension.

It creates a `vVERSION` tag and advances `master` to the next patch's `-dev`
version. Publishing uses the `release` environment and Marketplace Trusted
Publishing. See [release.yml](.github/workflows/release.yml) for setup.

[Rust]: https://rust-lang.org
[Cargo]: https://doc.rust-lang.org/cargo
[rustup]: https://rustup.rs
[msvc]: https://visualstudio.microsoft.com/thank-you-downloading-visual-studio/?sku=BuildTools

## License

Auxtools is licensed under the [MIT license](https://en.wikipedia.org/wiki/MIT_License).
See [LICENSE](./LICENSE) for more details.

The debug server, launcher and Auxcov coverage tool are licensed under GPL-3.0.
See [debug_server/LICENSE](./debug_server/LICENSE), [auxtools-cli/LICENSE](./auxtools-cli/LICENSE) and [auxcov/LICENSE](./auxcov/LICENSE).
