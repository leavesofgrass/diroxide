# ADR-0014: Software drawing, and one console program on Windows

Status: Accepted, Tuesday, November 3, 2026 (alpha 3, Wave 6, lanes G0 and G3, from research/gui-stack.md, R26, research/alpha3-gui-plan.md, R27, spikes S4 and S8, and the owner's answer 106).

## Context

The window draws with Vello, which renders on the graphics card through wgpu and needs an adapter with compute shaders. Many of the machines a screen-reader user sits at have none that works: a Remote Desktop session, a virtual machine, a bare install with no driver, and CI's Windows runner, where textweaver's own workflow found the software adapter crashing on Vello's shaders. The owner chose (answer 106) that the window should still open there, drawn on the processor, and that the terminal is the last resort, never a blank window.

On Windows a program is built for one of two subsystems. A console program gets a console of its own when Explorer starts it, which shows as a window; a GUI program has none, so `diro ls | findstr` would print nothing unless the program attached its parent's console first, and a shell would get its prompt back before the program ended. diroxide is one binary for the terminal and the window (ADR-0007), so one of these costs had to be paid.

## Decision

- **Four drawing roads, tried in order,** chosen by the vendored `masonry_winit` runner before it makes the window's surface (diroxide's third patch to the vendored tree, `third_party/xilem/diroxide.patch`, the module `drawing.rs`): the graphics card, through wgpu, when an adapter that is not a software one exists; the processor, Vello's CPU renderer presenting whole frames through softbuffer with no graphics driver at all, when there is no adapter or only a software one; the hybrid renderer on a software adapter when the processor's road cannot start; the terminal, with one sentence, when nothing draws. The sentence is "Could not open the GUI: " then the cause, then "the terminal opens instead."
- **The choice is the owner's too.** `--graphics NAME` for one start and the `graphics` setting keep it: `auto` takes the order above, `software` asks for the processor, and `vulkan`, `dx12`, `metal` and `gl` name a programming interface for the card. A road other than the card is said once in the status line ("Drawing on the processor: no graphics adapter was found."), and `diro --log FILE` records the road taken with the average frame time, for a report from a machine the orchestrator never sees.
- **`diro.exe` is a console program** that lets go of its console only when it is alone on it and the window opens. Started from a shell, the shell's console is kept: the shell waits, messages print there, Ctrl+C ends the window. Started from Explorer, a shortcut or the Run box, the console Windows gave it closes when the window opens, and may show for a moment first. If the window cannot open after the console was let go, a message box says why and the terminal opens in a new console. `--background` opens the window off screen, never activated, for tests and reports.

## Alternatives

- **A GUI-subsystem program attaching its parent's console,** textweaver's road: no console ever shows, but a pipe needs the attach to succeed first, the prompt returns before the program ends, and redirected handles need care on every start. Spike S8 measured the console program's cost at 51 to 163 ms of console on a busy machine in a debug build, and the owner's hard rule is no flashing console; the moment the console shows is the price of pipes that work, and the first release build measures it again.
- **Two binaries,** `diro` for the terminal and a windowed one: two things to install, name and document; refused by ADR-0007's one core and one program.
- **The processor always:** spike S4 measured a 1080p frame of a hundred thousand rows at 6.7 ms median on the processor, faster than the card's first frames, but the card is the stack's own tested road, draws large and high-density windows with less load, and keeps the processor free for the screen reader; the processor is the fallback, chosen first only when asked for.
- **The hybrid renderer as the first fallback,** which R27 expected: it ran at 270 ms a frame on Windows' software adapter, too slow for a list that moves on every key; it stays as the road after the processor's.

## Fallback

When every road fails the terminal opens with the one sentence, so the owner is never left in a window that does not draw. A road that fails on one platform is passed over with `--graphics`, and the developer switch `DIROXIDE_DRAWING` forces each road for a check. If a platform's software adapter proves better than the processor's road, the order changes in `drawing.rs` alone.

## Consequences

- Three new crates on the processor's road (softbuffer, tiny-xlib on X11, and ctor and dtor beneath them), all pure Rust; the native inventory and cargo-deny see them.
- The vendored `masonry_winit` differs from textweaver's by this patch and the clipboard patch; the sync rule in `docs/dev/vendoring.md` names them, and the patch is offered to textweaver.
- CI's GUI workflow runs the window on the Windows runner, which has no graphics card, and fails on error now; the Linux AT-SPI check in the container runs under Xvfb with llvmpipe as the only adapter and reports the road.
- The macOS road is what CI's harness proves; no person has seen the window there yet.
- Archive and hidden rows may draw upright on the processor's road, since no italic face is compiled in; their words say what they are, and the tree is the same on every road.

## See also

- [The GUI](../gui.md), the guide, and [Vendoring](../dev/vendoring.md).
- [ADR-0007: Frontends over one core](0007-frontends-over-one-core.md).
