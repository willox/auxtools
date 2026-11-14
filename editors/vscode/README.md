# auxtools Debugger

Debug BYOND worlds in VS Code with [auxtools](https://github.com/auxtools/auxtools).
Supports breakpoints, stepping, runtime errors, sleeping proc stacks, variable
inspection, expression evaluation, disassembly and experimental hot reload.

The launcher loads the debugger without changes to your world's source. Windows
supports DreamSeeker and DreamDaemon; Linux supports DreamDaemon. Install a DM
language extension alongside it for syntax highlighting and language features.

## Start debugging

Build your world, then press F5. The extension runs the `.dmb` in DreamSeeker on
Windows or DreamDaemon on Linux. If several worlds are available, select one.
You can also use **auxtools: Debug in DreamSeeker** or **auxtools: Debug in
DreamDaemon** from the command palette.

**Run Without Debugging** (Ctrl+F5) disables breakpoints and stepping while keeping
hot reload enabled. You can also set `"noDebug": true` in a launch configuration.

To save a configuration, add it to `.vscode/launch.json`. **Add Configuration...**
provides templates:

```json
{
    "type": "auxtools",
    "request": "launch",
    "name": "Debug in DreamDaemon",
    "dmb": "${workspaceFolder}/tgstation.dmb",
    "server": "dreamdaemon"
}
```

| Option | Description |
| --- | --- |
| `dmb` | Compiled world to run. |
| `dme` | Project environment; defaults to the `.dme` beside the `.dmb`. Its directory is the source root. |
| `server` | `dreamseeker`, `dreamdaemon` or `dd`. `dd` uses Windows' console DreamDaemon. |
| `defines` | Build defines as `NAME` or `NAME=VALUE`. Use the same `-D` values as DreamMaker so hot reload parses the same source. |
| `noDebug` | Disable debugging. Default: `false`. |
| `hotReload` | Reload source changes on save. Default: `true`. |
| `sleepingThreads` | Maximum sleeping procs shown in the call stack. Default: `100`. |
| `byondPath` | Optional BYOND installation path. |
| `args` | Arguments after the `.dmb`, replacing the defaults: `-trusted`, plus `-close` for windowed DreamDaemon on Windows. |
| `env` | Environment variables for the world. |
| `debugLibrary` | Path to a custom debug library. |

Linux DreamDaemon and Windows `dd.exe` send output to the Debug Console.
DreamSeeker and windowed DreamDaemon display output in their own windows.
The debugger reconnects after `world.Reboot()` and restores breakpoints and reloads.

**Stop** requests shutdown through `world/Del()`, allowing cleanup such as saving.
After five seconds, the launcher terminates the process. Press **Stop** again or
use **Disconnect** to terminate a launched world immediately. Attached worlds
follow the [attach behavior](#attach-to-a-running-world) below.

Windows DreamDaemon may prompt before running in trusted mode. Select
"Don't ask me this again" to suppress that prompt.

## Breakpoints

Click the gutter for a line breakpoint. Use **Add Function Breakpoint** for a proc
path: `/mob/proc/Login`, or `/mob/Login#1` for its second definition.

- **Conditions** are DM expressions. A false result skips the hit. Runtime errors
  also skip the hit and are reported once in the Debug Console. A condition that
  fails to compile prevents the breakpoint from being set; hover over it for details.
- **Hit counts** support `5` (fifth hit onward), `== 5` (fifth hit only), `% 5`
  (every fifth hit), and `>`, `>=`, `<`, `<=`. Only hits passing the condition count.
  Counts reset when the breakpoint changes, its file is reloaded, or the world
  restarts. Function breakpoint counts survive hot reload.
- **Logpoints** print instead of pausing. Use `{expression}` interpolation, such as
  `health is {health} for {src}`. Failed expressions display their errors in place.
  Write `{{` and `}}` for literal braces.
  Logpoints can also have conditions and hit counts.

With hot reload enabled, breakpoints added to unsaved text remain unverified
until the file is saved and reloaded. Existing breakpoints continue to refer to
the running code. After reload, they follow their statements as lines move.

Without hot reload, breakpoints refer to the source used to build the world.
If the file has changed since the build, the displayed line may no longer match;
the Debug Console warns about this.

**Loaded Scripts**, available from the Run and Debug view's **...** menu, lists
files named in the world's debug info and BYOND's `stddef.dm`. Files containing
only types, variables or defines are absent because they have no procs.

## Sleeping procs

While paused, the Call Stack view lists the running proc followed by sleeping
procs. Inspect their frames and variables or evaluate expressions as usual.
Stepping a sleeping proc waits until it wakes. The view lists up to 100 sleeping
procs by default; change `sleepingThreads` to adjust the limit.

## Data breakpoints

While paused, right-click a variable and choose **Break on Value Change**.
Execution pauses just after its value changes, usually on the next line.
The Debug Console reports the old and new values.

You can watch object variables, list entries, globals, locals and arguments.
Frame variables are watched only for that call. Watching a variable containing a
list detects reassignment of the list; watch an entry to detect changes within it.
`src`, `usr` and expressions without a storage location, such as `a + b`, cannot
be watched. Assigning the same value does not trigger a breakpoint.

Data breakpoint conditions use `value` and `old` for the new and previous values:
`value < old`, for example. Hit counts and condition errors behave like ordinary
breakpoints. Watches expire when their object is deleted or their proc returns.
After reboot, only global watches are restored; the Breakpoints view explains
which watches expired.

Watches are checked between instructions, including engine changes such as atom
movement. They slow execution while enabled. Remove them when no longer needed.

## Runtime errors

In the Breakpoints view, **Runtime errors** pauses on uncaught runtime errors and
uncaught throws. It is enabled by default. **Caught runtime errors** also pauses
inside `try` blocks.

Use the filter's pencil icon to match message substrings. Separate alternatives
with commas and prefix exclusions with `!`: `bounds, null` accepts either word;
`!libdreamluau, !rust_g` excludes both.

Errors are still printed when no breakpoint pauses for them. Source locations in
the Debug Console link to the corresponding file and line.

## Variables and evaluation

The Variables view shows the selected frame's locals and arguments, including
`src` and `usr`, and loads globals when expanded. Objects expand to variables;
lists expand to entries, paginated in groups of 100. Hover a variable to see its type.

**Set Value** accepts an expression evaluated in the selected frame. **Copy as
Expression** and **Add to Watch** use an access expression such as
`mob.contents[3].name`. Use **Format > Hex** for hexadecimal numbers.

Watches, conditions and the Debug Console accept DM expressions such as
`health / maxHealth` or `get_dist(src, usr) < 3`. Name resolution follows the
selected proc: locals and arguments, then `src` variables, then globals. Bare calls
resolve to `src` procs where available. While running, evaluation has only global
scope. `/obj/item::force` reads a type's initial value. In launched worlds,
`auxtools_kept()` stores values between evaluations until reboot. For example,
`auxtools_kept()["m"] = m` saves a value for a later `auxtools_kept()["m"]`.

Expressions can assign variables and call procs. Watch expressions run at every
stop, so their side effects can change the world.

The Debug Console also accepts proc bodies with declarations, loops and `return`:

```dm
var/n = 0
for (var/mob/M in world)
    n++
n
```

The result is the returned value or final expression. A body ending with a
statement, including an assignment or increment, returns `.`, as a proc does.
Bodies can modify frame variables. Compiler warnings accompany the result.
Evaluation is aborted if it sleeps or calls a sleeping proc.

Source `#define`s are available in evaluations and logpoints when the launcher has
parsed the project for hot reload. Definitions update on reload; undefined macros
are removed. They are unavailable when the project has not been parsed, including
with `"hotReload": false`.

Hovering source code evaluates a single expression in the selected frame without
calling procs or assigning values. The Debug Console completes visible variables
and procs, including members after `.`.

`#dis` shows the selected proc's bytecode; `#dis /proc/name` selects another proc.

## Disassembly

While paused, right-click a stack frame and select **Open Disassembly View**.
The view shows dmasm instructions, hexadecimal words, source lines and the current
instruction. A frame still running pre-reload bytecode shows that bytecode without
source lines, since its source text is no longer current.

Instruction addresses encode the proc ID, retained bytecode buffer ID and word offset as
`0xPPPPPPPPCCCCOOOO`. For example, `0x0000002A00000013` is offset `0x13` in proc
`0x2A`'s current bytecode.

Click beside an instruction address to set a breakpoint. It must be an instruction
boundary in the proc's current bytecode. Instruction breakpoints support conditions
and hit counts and survive world reboots.

With this view focused, **Step Over** advances one instruction in the frame,
running calls to completion. **Step Into** advances to the next executed
instruction, entering a callee if needed. **Step Out** returns to the caller.

## Hot reload (experimental)

Saving a source file reloads supported changes, including during **Run Without
Debugging**. The Debug Console reports results and changes requiring a rebuild.

After reboot, reloads are restored before world code runs. Reloads introducing
resources absent from the build wait until `world/New`, so map atoms run the
built code.

Supported changes include proc bodies, new procs and verbs, removals, new types
(including mobs), new variables and static locals, variable types and defaults,
changes to `static`, `const` and `tmp` variable declarations,
parameters (including `as` input types), `in` lists, prefabs, `set` statements
and parent types (`parent_type`).
Proc, verb, world proc and global proc definitions can be inserted or removed
between existing overrides.

Changes requiring a rebuild include proc declaration changes (such as switching
between proc and verb or adding `final`), unsupported built-in defaults such as
`see_infrared`, and removing a built-in type's own override (such as
`/mob/layer`). The last case needs BYOND's original default, which the running
world no longer stores.

Reloaded and evaluated code preserves DreamMaker's field caching: after
`a.b.f()`, a later `a.b.x` in the same flow can use the old object even if
`f()` replaced `a.b`. This common pattern does not warn. Read the field into
a local when a call may replace it. Other compatibility differences and
copied quirks are reported as warnings.

Active calls finish on their original code. Existing instances retain their
verbs and built-in values. Computed defaults apply to new instances; new
computed vars are null on existing instances, including their `initial()`
values, until restart. Running globals and statics keep their values when
initializers change. Changed world settings and client settings stored in the
world record, such as `client.control_freak`, take effect after restart.

Breakpoints follow statements after edits from VS Code, other editors or checkouts.
The VS Code extension reports unsaved files to the launcher. Other DAP clients
that do not report this state use on-disk line numbers for new breakpoints.

## Attach to a running world

Use **auxtools: Attach to a running world** to select a world started by the
launcher. Multiple VS Code windows can attach simultaneously, each with independent
breakpoints. Any session can resume a paused world.

Disconnecting leaves an attached world running. If it is paused, it stays paused
while other debugging sessions remain. The Debug Console also reports reloads
performed by other launchers.

```json
{
    "type": "auxtools",
    "request": "attach",
    "name": "Attach to a world",
    "processId": "${command:auxtools.pickWorld}"
}
```

Attach options are `processId`, `dme`, `defines`, `hotReload` and `sleepingThreads`.
If `processId` is omitted, select a running world. Hot reload defaults to `false`
for attach sessions to avoid competing with the launching session's reloads.

## Settings

- `auxtools.byondPath`: BYOND installation directory. Discovery checks
  `BYOND_SYSTEM`, the registry, `Program Files`, `~/byond` and `/usr/local/byond`.
- `auxtools.launcherPath`: custom launcher path.
- `auxtools.debugLibrary`: custom debug library path.

## License

GPL-3.0. See [LICENSE](LICENSE). The extension bundles the launcher and debug server.
