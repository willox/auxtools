# auxtools

auxtools is a Rust-based successor to the [extools](https://github.com/MCHSL/extools) project.

Currently, it implements some of the features such as the debug server, but is planned to supplant most functionality (maptick, etc.).

Code Documentation: https://auxtools.willox.dev

## Features
Code Coverage - A tool to generate cobertura code coverage XML reports for a codebase. Note that, due to a [BYOND issue](http://www.byond.com/forum/post/108025) it requires some additional work to have properly un-mangled file names in the report.

Debug Server - Working with SpaceManiac's [SpacemanDMM](https://github.com/SpaceManiac/SpacemanDMM), Auxtools interfaces with the debugger frontend to provide debugger information and management of breakpoints.

*Dissassembly*  - When currently stopped at a breakpoint, enter the `#dis` command into the Debug Console in VSC to see the DM bytecode for the current proc. You can also provide it any arbitrary proc path like `#dis /mob/proc/Life`.

## Supported BYOND versions

516.1659 through 516.1688 on Windows, and 516.1664 through 516.1688 on Linux. Those are the builds the patterns that find BYOND's functions were checked against. A newer build is still tried, and fails at init with the name of whatever stopped matching. An older build, or another major version, is refused at init with an error that names the build.

The patterns are run by [byond-scan](https://github.com/Absolucy/byond-scan) and live in `auxtools/src/symbols/`. They are copies of the ones in the byond-re repo's `byond_catalog`, which checks each against every local build, so a change to one has to land in both.

What was run on Linux (516.1687): the `auxtest` suite, and the debug server starting up. The code coverage tool's report was checked on Windows and Linux 1687 against a world with a known set of lines. Stepping and breakpoints with a real debugger attached were not tested on either platform.

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
    git clone https://github.com/willox/auxtools
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
    sudo apt-get install build-essential g++-multilib libc6-i386 libstdc++6:i386 pkg-config libssl-dev libssl-dev:i386
    ```

* Other Linux distributions install the appropriate **32-bit development** and **32-bit runtime** packages.

## Compiling

The [Cargo] tool handles compilation, as well as automatically downloading and
compiling all Rust dependencies. To compile in release mode (recommended for speed):

Linux:
```sh
export PKG_CONFIG_ALLOW_CROSS=1
cargo build --release --target i686-unknown-linux-gnu
# output: target/i686-unknown-linux-gnu/release/libauxtools.so
```

Windows:

```sh
cargo build --release --target i686-pc-windows-msvc
# output: target/i686-pc-windows-msvc/release/auxtools.dll,debug_server.dll,auxcov.dll
```

[Rust]: https://rust-lang.org
[Cargo]: https://doc.rust-lang.org/cargo
[rustup]: https://rustup.rs
[msvc]: https://visualstudio.microsoft.com/thank-you-downloading-visual-studio/?sku=BuildTools

## License

Auxtools is licensed under the [MIT license](https://en.wikipedia.org/wiki/MIT_License).
See [LICENSE](./LICENSE) for more details.

The Auxtool Debug Server (located @ ./debug_server) and Auxcov Code Coverage tool (located @ ./auxcov) is licensed under the [GPL 3.0 license](https://www.gnu.org/licenses/gpl-3.0.en.html).
See [debug_server/LICENSE](./debug_server/LICENSE) or [auxcov/LICENSE](./auxcov/LICENSE) for more details.
