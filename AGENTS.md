# Agent Notes

This repository is primarily useful to agents as a source of BYOND runtime signatures, raw type layouts, and function prototypes for populating Ghidra projects. Most agent work here is expected to happen through the Ghidra MCP tools rather than through code edits.

## Ghidra Programs

The Ghidra project contains these programs:

- `byondcore.dll` (this is 516.1681)
- `byondcore_515.1602.dll`
- `libbyond_516.1681.so`
- `libbyond_516.1667.so`
- `libbyond_515.1647.so`
- `libbyond_515.1602.so`

`byondcore.dll` is the main reference database for naming conventions, data type names, comments, and general layout style. `byondcore_515.1602.dll` is also populated enough to use as a 515-era reference. `libbyond_515.1602.so` has been populated with the auxtools Linux runtime signatures that matched uniquely, but should still be treated as partial. The auxtools codebase is currently a better reference for data structures and function definitions themselves, as the ghidra programs are not fully verified.

Always pass `program_name` explicitly to Ghidra MCP calls. Do not rely on the active program, because the project contains many similar BYOND binaries. It is expected that the current program reported by the mcp server can change arbitrarily, don't worry about it.

If a requested program is not open in Ghidra, some MCP calls can silently operate on the active program even when `program_name` was supplied. Always start with `list_binaries` and confirm the target is listed before trusting search, stats, or rename results.

## Version Conventions

Use the BYOND build minor number when discussing signatures and ranges:

- `1602` means BYOND `515.1602`.
- `1647` means BYOND `515.1647`.
- `1648` means BYOND `516.1648`.
- `1667` means BYOND `516.1667`.
- `1681` means BYOND `516.1681`.

Important boundaries:

- Last 515 build: `515.1647`.
- First 516 build: `516.1648`.
- In auxtools signature ranges, the major version is usually implicit and the minor build number drives the branch selection.

Public BYOND changelogs are in:

- `changelogs/515.md`
- `changelogs/516.md`

Use those changelogs when a structure, signature, or behavior changes between versions. They can explain why a signature moved, why an execution context field changed, or why a function stopped matching. They do not include every change.

## Platform Status

Windows builds should be reasonably well supported by the signatures in this repo. Prefer starting with the Windows DLLs when porting names and types across versions.

Linux support is much weaker:

- `libbyond_515.1602.so` is the last Linux build expected to have useful support.
- Linux support started breaking after `515.1602`.
- Auxtools is not currently functional on Linux for the version immediately before the compiler break, but that version may still contain useful migration clues.
- The Linux compiler changed at some point, probably around `516.1667`.
- Assume every signature is invalid on the latest Linux builds until verified.

For Linux work, do not bulk-apply Windows-derived names just because a function looks similar. Verify with byte patterns, call graph context, decompiler behavior, and version changelogs.

Current `libbyond_515.1602.so` status:

- Verified and named from unique auxtools Linux signatures: `BYOND_get_proc_array_entry`, `BYOND_get_string_id`, `BYOND_call_proc_by_id`, `BYOND_get_variable`, `BYOND_set_variable`, `BYOND_get_string_table_entry`, `BYOND_inc_ref_count`, `BYOND_dec_ref_count`, `BYOND_get_assoc_element`, `BYOND_set_assoc_element`, `BYOND_create_list`, `BYOND_append_to_list`, `BYOND_remove_from_list`, `BYOND_get_length`, `BYOND_get_misc_by_id`, and `BYOND_runtime`.
- Verified and named globals: `BYOND_suspended_procs`, `BYOND_suspended_procs_buffer`, and `BYOND_variable_names`.
- The auxtools signatures for `BYOND_call_datum_proc_by_name`, `BYOND_to_string`, and `BYOND_current_execution_context` did not match exactly during population. Leave them unresolved unless a future agent verifies them semantically.
- A shortened `to_string`-like pattern matched `FUN_00419980`, but decompilation showed key-file/fopen behavior, not BYOND value stringification. Do not use that candidate.

## Source Files To Use

NOTE: This is just a suggestion.

Core signature and initialization data:

- `auxtools/src/lib.rs`
- `auxtools/src/sigscan.rs`
- `instruction_hooking/src/lib.rs`
- `debug_server/src/ckey_override.rs`

Raw BYOND type layouts:

- `auxtools/src/raw_types/values.rs`
- `auxtools/src/raw_types/procs.rs`
- `auxtools/src/raw_types/misc.rs`
- `auxtools/src/raw_types/lists.rs`
- `auxtools/src/raw_types/strings.rs`
- `auxtools/src/raw_types/variables.rs`
- `auxtools/src/raw_types/funcs.rs`
- `auxtools/src/raw_types/funcs.cpp`

Signature macro behavior:

- `signature_struct!(call, "...")` means search for the pattern, then resolve the relative `CALL` target from the first byte of the match.
- `signature_struct!(N, "...")` means search for the pattern, then read a pointer-sized value at `match + N`.
- A plain signature resolves to the match address.
- `version_dependent_signature!` ranges are selected using the BYOND minor build number.

## Ghidra Population Workflow

NOTE: This is just a suggestion.

For each unpopulated program:

1. Start by checking `list_binaries`, `get_binary_info`, and `get_function_statistics`.
2. Confirm the image base, architecture, and program name.
3. Select the correct signatures from `auxtools/src/lib.rs` for that platform and minor version.
4. Use `search_bytes` and require unique matches before renaming functions.
5. Resolve `call` and integer-offset signatures exactly as implemented in `auxtools/src/sigscan.rs`.
6. Use `byondcore_516.1681.dll` as the naming and typing reference, but adjust layouts for the target version.
7. Apply structures before prototypes so decompiler output can recover field references.
8. Apply function names with the `BYOND_` prefix for runtime functions resolved from auxtools signatures.
9. Add comments for hook sites and non-obvious signature-derived globals.
10. Re-decompile representative callers to verify the result.

Ghidra MCP quirks observed while populating `libbyond_515.1602.so`:

- `batch_rename` did not find functions by bare address or `0x` address, but did find them by current symbol name such as `FUN_00364d30`.
- `search_bytes` reports match addresses, not resolved signature targets. For `signature_struct!(call, ...)`, read the call bytes and compute `target = call_address + 5 + rel32` before renaming.
- The struct C parser often treats forward declarations or referenced structs as extra structure definitions. If it reports multiple structures, create the referenced type first or inline fields with primitive types for the live target layout.
- Types created under `/auxtools` may not be found by `create_data_var` or `types set` using the bare type name. Creating simple root-category aliases can be faster when applying data variables through MCP.
- Do not include a trailing semicolon when using `variables set_prototype`; the parser rejects it as trailing text.

Prefer exact names already present in the populated reference program:

- `BYOND_call_proc_by_id`
- `BYOND_call_datum_proc_by_name`
- `BYOND_get_proc_array_entry`
- `BYOND_get_string_id`
- `BYOND_get_variable`
- `BYOND_set_variable`
- `BYOND_get_string_table_entry`
- `BYOND_inc_ref_count`
- `BYOND_dec_ref_count`
- `BYOND_get_assoc_element`
- `BYOND_set_assoc_element`
- `BYOND_create_list`
- `BYOND_append_to_list`
- `BYOND_remove_from_list`
- `BYOND_get_length`
- `BYOND_get_misc_by_id`
- `BYOND_to_string`
- `BYOND_runtime`
- `BYOND_execute_instruction`

Known global labels from the populated `1681` Windows database:

- `BYOND_current_execution_context`
- `BYOND_suspended_procs`
- `BYOND_suspended_procs_buffer`
- `BYOND_variable_names`
- `BYOND_guest_ckey_format_string`

For auxtools-populated BYOND functions, prototype the native BYOND function, not the Rust/C++ wrapper. Use `auxtools/src/raw_types/funcs.cpp` for the native function pointer definitions and `funcs.rs` for wrapper-facing types.

If a byte pattern is non-unique but one candidate is strongly implied, verify with decompiler semantics before naming it. For example `get_length` should switch on `ByondValue` tag and return string/list/content lengths.

## Struct And Type Conventions

Use `Byond`-prefixed type names in Ghidra. Keep the reference database's style:

- `ByondValue`
- `ByondValueTag`
- `ByondProcEntry`
- `ByondProcInstance`
- `ByondExecutionContext`
- `ByondStringEntry`
- `ByondList`
- `ByondAssociativeListEntry`
- `ByondSuspendedProcs`
- `ByondSuspendedProcsBuffer`
- `ByondVariableNameIdTable`

For versioned layouts, encode the boundary in the name when both variants are useful, for example:

- `ByondExecutionContextPre1668`
- `ByondBytecodePre1630`
- `ByondBytecodePost1630`
- `ByondProcInstancePre516Tail`
- `ByondProcInstancePost516Tail`

Apply only the target version's real layout to live variables and function prototypes. Keep older layouts available as reference types.

Known layout boundaries from the Rust sources:

- `ProcEntry.metadata` uses the post-1630 bytecode layout for `1630+`.
- `ProcInstance` uses the post-516 argument tail for BYOND 516 builds.
- `ExecutionContext` uses the post-1668 layout for `516.1668+`.
- `ExecutionContext` uses the pre-1668 layout for 515 builds and `516.1667`.

## Signature Repair Guidelines

When a signature breaks between versions:

- Check whether the version range in `auxtools/src/lib.rs` is too broad.
- Compare the populated `1681` Windows target with the closest older populated Windows target.
- Check BYOND changelogs for relevant runtime, list, proc, compiler, exception, or debugger changes.
- For Windows, inspect nearby calls and string references before changing a pattern.
- For Linux, assume compiler output changed enough that Windows pattern logic may not transfer.
- Use shorter patterns only when they still match exactly once.
- Prefer stable semantic anchors: calls to already identified helpers, accesses to known globals, distinctive runtime strings, jump-table setup, and structure field offsets.
- Record repaired signatures in code comments or Ghidra comments when the reason is not obvious.

Do not rename or prototype a function from a non-unique byte match. If a match is ambiguous, add a comment explaining the candidates and leave the symbol unresolved until there is stronger evidence.

## Repository Work

Use `rg` for searching. Keep code edits scoped to the requested task. The repo contains Rust crates plus C++/assembly hook shims; avoid broad formatting churn.

Reasonable verification commands when editing code:

- `cargo check`
- `cargo test`
- Target-specific checks for 32-bit builds only when the required toolchain and system libraries are installed.

Network access and dependency downloads may not be available in agent environments, so report when verification could not be run.

## BYOND builds
Builds of BYOND can be found under the `./byond` directory. For example: ./byond/linux/515.1602_byond_linux/... or ./byond/windows/515.1602_byond/...

## Auxtest Runner

Use `tools/run-auxtest.sh` to run the auxtest integration world against local BYOND builds. The script is WSL-oriented and supports these targets:

- `tools/run-auxtest.sh all`
- `tools/run-auxtest.sh linux-515`
- `tools/run-auxtest.sh linux-516`
- `tools/run-auxtest.sh windows-515`
- `tools/run-auxtest.sh windows-516`
- `tools/run-auxtest.sh linux-515.1647`
- `tools/run-auxtest.sh windows-515.1647`
- `tools/run-auxtest.sh linux-1647`
- `tools/run-auxtest.sh windows-1647`
- `tools/run-auxtest.sh 1647`

The current target mappings are:

- `linux-515`: `./byond/linux/515.1602_byond_linux/byond`
- `linux-516`: `./byond/linux/516.1681_byond_linux/byond`
- `windows-515`: `./byond/windows/515.1602_byond/byond`
- `windows-516`: `./byond/windows/516.1681_byond/byond`
- `linux-<version>` and `windows-<version>` map to `./byond/<platform>/<version>_byond.../byond`
- Bare minor builds such as `1602`, `1647`, and `1681` are mapped to their BYOND major version and run on both Windows and Linux.

The runner builds and runs `tests/test_runner`, which builds `tests/auxtest`, compiles `tests/auxtest_host/auxtest_host.dme` with the selected DreamMaker, then launches DreamDaemon with `AUXTEST_DLL` pointing at the built auxtest library. It deletes the old `.dmb` before compiling so the selected BYOND version always produces the world being run.

Linux requirements:

- Rust target: `i686-unknown-linux-gnu`
- Usual 32-bit build/runtime packages from the README, including multilib C/C++ support.
- Newer Linux BYOND builds may also need runtime libraries such as `libcurl4:i386`.

Windows-from-WSL requirements:

- Windows Rust target: `i686-pc-windows-msvc`
- Visual Studio C++ x86 tools discoverable via `vswhere.exe`.
- The script uses PowerShell, initializes the VS developer shell, copies the source tree to `%TEMP%\auxtools-wsl-<version>`, and runs Windows Cargo from there. This avoids Rust/Cargo and BYOND issues with WSL UNC paths.
- The script intentionally excludes `.git`, `target`, and the `byond` symlink from the temp copy.

Auxtest output and diagnostics:

- `auxtest_out()` writes markers like `SUCCESS: Finished` and `FAILED: ...` to stderr and, when `AUXTEST_OUT` is set, to `auxtest_output.log` beside the `.dmb`.
- DreamDaemon is launched with `-log auxtest_host.txt`; the runner includes that log in failure output. This is important for failures before auxtest can emit markers, such as `auxtools_init` returning `FAILED (Couldn't find <signature>)`.
- A missing `SUCCESS:` marker should be treated as a real test failure. Inspect the included `world log:` first; it often contains the root cause.

When asked to run auxtest against a BYOND version and fix issues:

- Start with the closest runner target above, using exact-version syntax when the requested minor build matters. For example, use `linux-515.1647` or `linux-1647` to test Linux BYOND `515.1647` independently from `515.1602`.
- If DreamMaker or DreamDaemon fails before loading the world, fix environment/runtime prerequisites first. Examples include missing 32-bit Linux shared libraries or Windows path handling.
- If the world log says `init_result = FAILED (Couldn't find <name>)`, this is a signature issue. Repair only the missing signature first, keep the version range as narrow as justified, and rerun the same target before moving on.
- For signature repair, require unique byte matches. If using a semantic cross-reference, document the reasoning in the conversation and prefer version-bounded signatures. Do not broaden a signature just because it works on one build.
- After fixing a Windows 515-era signature, rerun `windows-515` and `windows-516` to check both sides of the 515/516 boundary. After fixing a Linux 515-era signature, rerun `linux-515`; only run `linux-516` if the task explicitly includes latest Linux or if the signature range could affect it.
- Latest Linux support is known to be weak. If `linux-516` fails on missing signatures, do not bulk-apply Windows-derived names or patterns; verify with Ghidra semantics and version-bound every fix.

# Ghidra Tips

When searching for new symbols in a BYOND program, prioritising using x-refs within the program of a better-supported BYOND version as part of the search. For example, if version 1602 and 1647 both have a known "A" function, and we know that "A" calls "B", but "B" is only known in 1602, we can use the fact that "A" x-refs "B" to find "B" in 1647.
