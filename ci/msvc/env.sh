# Source from bash with MSVC_SDK pointing to the fetched SDK:
#
#     export MSVC_SDK=DIRECTORY
#     . ci/msvc/env.sh
#
# Cross-compile i686-pc-windows-msvc on Linux using clang, lld-link and
# Microsoft libraries in MSVC_SDK. Requires clang 19 or newer. Then:
#
#     cargo build --target i686-pc-windows-msvc
#
# This file's settings take the place of the target's in .cargo/config.toml.

: "${MSVC_SDK:?set MSVC_SDK to the SDK directory created by ci/msvc/fetch-sdk.sh}"
MSVC_SDK=$(cd "$MSVC_SDK" && pwd)
export MSVC_SDK

# Find an unversioned LLVM tool or the newest installed version.
msvc_llvm_tool() {
	command -v "$1" || ls /usr/lib/llvm-*/bin/"$1" 2>/dev/null | sort -V | tail -n 1
}

MSVC_CLANG=$(msvc_llvm_tool clang)
export MSVC_CLANG
# bin/ has clang-cl, and an ml.exe for the assembly MASM would build
PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/bin:$PATH"
export PATH
export CC_i686_pc_windows_msvc=clang-cl
export CXX_i686_pc_windows_msvc=clang-cl
AR_i686_pc_windows_msvc=$(msvc_llvm_tool llvm-lib)
export AR_i686_pc_windows_msvc
CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER=$(msvc_llvm_tool lld-link)
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER
# Link the C runtime statically and omit build-machine paths from PDB names.
export CARGO_TARGET_I686_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -C link-arg=/PDBALTPATH:%_PDB% -Lnative=$MSVC_SDK/crt/lib/x86 -Lnative=$MSVC_SDK/sdk/lib/um/x86 -Lnative=$MSVC_SDK/sdk/lib/ucrt/x86"
