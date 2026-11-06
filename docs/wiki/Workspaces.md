### Overview

Neori has dynamic workspaces that can move between monitors.

Each monitor contains an independent set of workspaces arranged vertically.
You can switch between workspaces on a monitor with `focus-workspace-down` and `focus-workspace-up`.

Every workspace has a globally unique number that is assigned when it is created and never changes; see [Addressing workspaces by number](#addressing-workspaces-by-number).
Empty, unnamed workspaces are removed as soon as you switch away from them, so a monitor keeps at most one empty workspace: the active one.
There is no permanently-kept empty workspace at the bottom.
Navigating past the last workspace creates a new one on demand, and opening a window in the active empty workspace does not create another one below it.

You can move workspaces up and down on the monitor with `move-workspace-up/down`.
This changes their order on the monitor but not their numbers.
To put a window on a new workspace "in the middle", switch to it with `focus-workspace N` (which creates it if needed), or put the window on a workspace and then move that workspace up or down.

Here's a visual representation that shows two monitors and their workspaces.
The left monitor has three workspaces (two with windows, plus one empty), and the right monitor has two workspaces (one with windows, plus one empty).
An empty workspace only exists while it is the active workspace on its monitor.

<picture>
    <source media="(prefers-color-scheme: dark)" srcset="./img/workspaces-dark.png">
    <img alt="Two monitors. First with three workspaces, second with two workspaces." src="./img/workspaces-light.png">
</picture>

You can move a workspace to a different monitor using binds like `move-workspace-to-monitor-left/right/up/down` and `move-workspace-to-monitor-next/previous`.
A workspace keeps its number when you move it to another monitor.

When you disconnect a monitor, its workspaces will automatically move to a different monitor.
But, they will also "remember" their original monitor, so when you reconnect it, the workspaces will automatically move back to it.
To reduce surprising workspace movement, regular workspaces will "forget" their monitor when you open a new window there.
[Named workspaces](./Configuration:-Named-Workspaces.md), on the other hand, will remember their original monitor until you explicitly move them to a different one.

> [!TIP]
> From other tiling WMs, you may be used to thinking about workspaces like this: "These are all of my workspaces. I can show workspace X on my first monitor, and workspace Y on my second monitor."
> In neori, instead, think like this: "My first monitor contains these workspaces, including X and Y, and my second monitor contains these other workspaces. I can switch my first monitor to workspace X or Y. I can move workspace Y to my second monitor to show it there."

### Addressing workspaces by number

Several actions in neori address workspaces by number: `focus-workspace 2`, `move-column-to-workspace 4`.

Every workspace gets a globally unique number when it is created.
The number is a property of the workspace itself: it stays the same when the workspace is moved up or down, or moved to another monitor, and no two workspaces ever share a number.
Workspace 1 always exists.

`focus-workspace N` switches to workspace N wherever it is, focusing its monitor.
If no workspace with number N exists, it is created on the focused monitor.
Numbers are created on demand and gaps are not filled in: focusing workspace 6 when only 1 and 2 exist creates just 6, not 3, 4 and 5.
Likewise, `move-column-to-workspace N` creates workspace N on the current monitor if it doesn't exist yet.

Because empty workspaces disappear, the numbers in use can become sparse (for example 1, 2, 4, 7).
A removed number is not reused, so `focus-workspace N` never lands on a different workspace than the one you created earlier.

This is an important distinction from WMs where `focus-workspace 2` means "the second workspace from the top".
In neori, `focus-workspace 2` always means workspace number 2, no matter where it currently sits or which monitor it is on.
To move by position on the current monitor, use `focus-workspace-up/down`.

When you want to have a more permanent workspace in neori, you can create a [named workspace](./Configuration:-Named-Workspaces.md) in the config or via the `set-workspace-name` action.
You can refer to named workspaces by name, e.g. `focus-workspace "browser"`, and they won't disappear when they become empty.
Named workspaces also get a number, so you can address them either way.

> [!TIP]
> Stable numbers make it easy to set up a static-workspace-like workflow: bind `Mod+1`..`Mod+0` to `focus-workspace 1`..`focus-workspace 10`, like the default config does.
> Workspaces are still dynamic—they can be moved up and down and between monitors—but `Mod+N` will always take you to workspace N.
>
> That said, if you're coming from a static workspace WM, it's still worth trying the "neori way" with dynamic workspaces, focusing and moving up/down instead of by number.
> Thanks to scrollable tiling, you generally need fewer workspaces than on a traditional tiling WM.

### Example workflow

This is how I like to use workspaces.

I will usually have my browser on the topmost workspace, then one workspace per project (or a "thing") I'm working on.
On a single workspace I have 1–2 windows that fit inside a monitor that I switch between frequently, and maybe extra windows scrolled outside the view, usually either ones I need rarely, or temporary windows that I quickly close.
When I need another permanent window, I'll put it on a new workspace.

I actively move workspaces up and down as I'm working on things to make what I need accessible in one motion.
For example, I usually frequently switch between the browser and whatever I'm doing, so I always move whatever I'm currently doing to right below the browser, so a single `focus-workspace-up/down` gets me where I want.
