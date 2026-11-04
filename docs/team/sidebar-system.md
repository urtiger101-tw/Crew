# System status sidebar

Crew adds a compact system section below the existing sidebar content. It appears in the
Aurora workspace tree on the left and in the classic agents sidebar on the right. The
workspace tree and agents rows keep their existing navigation and pane targets.

## What the values mean

`Launch dir` is the selected pane's launch directory: the working directory Crew used when
it started that pane. It follows pane focus. A shell command such as `cd` does not change
this value; Crew does not inspect or claim to track the shell's current directory. The disk
free/total sample is for the volume containing the selected pane's launch directory.

RAM used/total and the OS name describe the local host. The pane count includes all open
panes in this daemon. Native system information is sampled outside the UI state lock and cached for
up to five seconds. A disk sample that has not arrived yet is shown as pending. A failed
disk or RAM sample and an unknown launch directory are shown as `N/A` (`無資料`). The
panel does not launch shell commands or add a runtime dependency.

## Short terminals

The panel is hidden below 12 terminal rows. At 12 rows it keeps the launch directory,
disk and RAM rows visible. At 16 rows and above it has room for the OS and pane count too.
Its rows are reserved inside the sidebar and do not take space from panes or move existing
mouse targets. The terminal status row remains at the bottom.

## Isolated native smoke

Run against the shipping executable after the sidebar changes are integrated:

```powershell
python scripts/system_status_smoke.py --binary .\target\release\crew.exe
```

The smoke creates a private temporary configuration, socket, state file and two pane
launch directories. It checks the Aurora left tree and classic right sidebar at 120x30,
80x12 and 60x18, in English and Traditional Chinese. It waits for actual cached disk and
RAM values, compares disk values with the host filesystem, checks that pane dimensions
stay stable across rendering and refresh, verifies a click on the system section is inert,
and verifies that the normal pane body still changes focus. It stops only the server owned
by the run and removes only its temporary directory.

To retain representative frames and the sampled values for review, add a destination:

```powershell
python scripts/system_status_smoke.py --binary .\target\release\crew.exe `
  --evidence-dir .\target\system-status-evidence
```

Each invocation creates a uniquely named evidence subdirectory containing a JSON summary
and representative ANSI and plain frames. The test uses no agent provider or model calls.
