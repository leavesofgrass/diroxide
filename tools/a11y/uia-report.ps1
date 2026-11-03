<#
.SYNOPSIS
  Opens diroxide's window out of the way and reports what assistive
  technology sees through UI Automation: every element's control type,
  name and patterns; each panel's list box with its options, positions
  and set size; the status bar; the live-region events with their text;
  the process's top-level windows (winit's helper window among them);
  and whether the window ever took the foreground.

.DESCRIPTION
  diroxide's window draws its own widgets; AccessKit gives them to UI
  Automation as one provider per window. NVDA and JAWS read the window
  through UI Automation, so this report reads it the same way.

  - The window starts with --gui --background (never activated, off
    screen, no taskbar button) and SW_SHOWNOACTIVATE, from a process with
    no console window (CREATE_NO_WINDOW); the report samples the
    foreground window every 100 ms and says whether it ever changed. No
    key is sent and nothing is clicked, since either would need the
    focus; the report reads, and moves nothing but a list's scroll
    through ScrollItemPattern.
  - It listens for LiveRegionChanged events desktop-wide before the
    window starts, and polls for new Text elements in the window, so the
    start's held messages show up when the report's client first asks
    for the tree (they are held until then).
  - It lists every top-level window of the process with its class, its
    title, whether it is visible, and whether UI Automation's control
    view of the desktop shows it, which is what a screen reader's object
    navigation and its window list meet.
  - It closes the window with WM_CLOSE (the app's quit rule) and reports
    the exit code.

  Plays no audio. Writes the report as Markdown (to -Out, or the
  console). Adapted from textweaver's
  crates/textweaver-xilem/tools/uia-report.ps1 (the owner's own project).

.EXAMPLE
  powershell -NoProfile -File tools/a11y/uia-report.ps1 -Folder D:\dirocheck -Out uia-diroxide.md
#>
param(
    [string] $Exe = '',
    # The folder the left panel opens; a sample folder is made under the
    # build folder when none is given.
    [string] $Folder = '',
    # The right panel's folder; the left one's when not given.
    [string] $Other = '',
    # The portable configuration folder diro writes into (beside the
    # program by default; --portable).
    [string] $Out = '',
    [int] $SettleMs = 4000
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
if (-not $Exe) { $Exe = Join-Path $target 'debug\diro.exe' }
if (-not (Test-Path -LiteralPath $Exe)) { throw "Not built: $Exe (cargo build -p diro)." }
$Exe = (Resolve-Path -LiteralPath $Exe).Path
if (-not $Folder) {
    $Folder = Join-Path $target 'uia-sample'
    $backup = Join-Path $Folder 'Backup'
    New-Item -ItemType Directory -Force -Path $backup | Out-Null
    foreach ($name in @('notes.txt', 'report.pdf', 'budget.xlsx', 'raccoon.txt')) {
        $file = Join-Path $Folder $name
        if (-not (Test-Path -LiteralPath $file)) { Set-Content -LiteralPath $file -Value 'diroxide' -Encoding ascii }
    }
    $old = Join-Path $backup 'old.txt'
    if (-not (Test-Path -LiteralPath $old)) { Set-Content -LiteralPath $old -Value 'old' -Encoding ascii }
    if (-not $Other) { $Other = $backup }
}
$Folder = (Resolve-Path -LiteralPath $Folder).Path
if (-not $Other) { $Other = $Folder }
$Other = (Resolve-Path -LiteralPath $Other).Path

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, WindowsBase

$source = @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;

public static class DiroUia
{
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")]
    static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")]
    static extern bool IsWindowVisible(IntPtr hwnd);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct STARTUPINFO
    {
        public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
        public int dwX; public int dwY; public int dwXSize; public int dwYSize;
        public int dwXCountChars; public int dwYCountChars; public int dwFillAttribute;
        public int dwFlags; public short wShowWindow; public short cbReserved2;
        public IntPtr lpReserved2; public IntPtr hStdInput; public IntPtr hStdOutput; public IntPtr hStdError;
    }
    [StructLayout(LayoutKind.Sequential)]
    struct PROCESS_INFORMATION { public IntPtr hProcess; public IntPtr hThread; public int dwProcessId; public int dwThreadId; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool CreateProcess(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit,
        uint flags, IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);
    [DllImport("kernel32.dll")]
    static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")]
    static extern uint WaitForSingleObject(IntPtr h, uint ms);
    [DllImport("kernel32.dll")]
    static extern bool GetExitCodeProcess(IntPtr h, out uint code);

    delegate bool EnumProc(IntPtr hwnd, IntPtr lParam);
    [DllImport("user32.dll")]
    static extern bool EnumWindows(EnumProc proc, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassName(IntPtr hwnd, StringBuilder text, int max);

    public static readonly List<string> Events = new List<string>();
    static readonly Stopwatch Clock = Stopwatch.StartNew();
    static int listenPid;
    static IntPtr process = IntPtr.Zero;

    /// Starts diro without activating it and with no console window: the
    /// first ShowWindow of a process started with STARTF_USESHOWWINDOW
    /// uses wShowWindow, and CREATE_NO_WINDOW gives it no console.
    public static int LaunchInactive(string exe, string args, string dir)
    {
        var si = new STARTUPINFO();
        si.cb = Marshal.SizeOf(typeof(STARTUPINFO));
        si.dwFlags = 0x1;          // STARTF_USESHOWWINDOW
        si.wShowWindow = 4;        // SW_SHOWNOACTIVATE
        PROCESS_INFORMATION pi;
        var cmd = new StringBuilder("\"" + exe + "\" " + args);
        if (!CreateProcess(exe, cmd, IntPtr.Zero, IntPtr.Zero, false, 0x08000000 /* CREATE_NO_WINDOW */,
                IntPtr.Zero, dir, ref si, out pi))
            throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        CloseHandle(pi.hThread);
        process = pi.hProcess;
        return pi.dwProcessId;
    }

    /// Waits for the process to end; its exit code, or -1 if it did not
    /// end in time.
    public static long WaitExit(int ms)
    {
        if (process == IntPtr.Zero) return -1;
        if (WaitForSingleObject(process, (uint)ms) != 0) return -1;
        uint code;
        GetExitCodeProcess(process, out code);
        CloseHandle(process);
        process = IntPtr.Zero;
        return code;
    }

    /// Every top-level window of the process: handle, class, title,
    /// visible.
    public static List<IntPtr> WindowsOf(int pid)
    {
        var found = new List<IntPtr>();
        EnumWindows(delegate (IntPtr hwnd, IntPtr l)
        {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == (uint)pid) found.Add(hwnd);
            return true;
        }, IntPtr.Zero);
        return found;
    }

    public static string TitleOf(IntPtr hwnd)
    {
        var sb = new StringBuilder(256);
        GetWindowText(hwnd, sb, 256);
        return sb.ToString();
    }

    public static string ClassOf(IntPtr hwnd)
    {
        var sb = new StringBuilder(256);
        GetClassName(hwnd, sb, 256);
        return sb.ToString();
    }

    public static bool Visible(IntPtr hwnd) { return IsWindowVisible(hwnd); }

    public static IntPtr FindFrame(int pid)
    {
        foreach (var hwnd in WindowsOf(pid))
            if (TitleOf(hwnd).EndsWith(" - diroxide")) return hwnd;
        return IntPtr.Zero;
    }

    public static AutomationElement WaitForWindow(int pid, int timeoutMs)
    {
        var sw = Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < timeoutMs)
        {
            IntPtr hwnd = FindFrame(pid);
            if (hwnd != IntPtr.Zero) return AutomationElement.FromHandle(hwnd);
            Thread.Sleep(100);
        }
        return null;
    }

    public static void Close(IntPtr hwnd) { PostMessage(hwnd, 0x0010, IntPtr.Zero, IntPtr.Zero); }

    public static uint ForegroundPid()
    {
        uint fg;
        GetWindowThreadProcessId(GetForegroundWindow(), out fg);
        return fg;
    }

    public static string Q(string s) { return s == null ? "(null)" : "\"" + s.Replace("\r", "\\r").Replace("\n", "\\n").Replace("\t", "\\t") + "\""; }

    static bool Ours(object sender)
    {
        try
        {
            var el = sender as AutomationElement;
            return el != null && el.Current.ProcessId == listenPid;
        }
        catch (Exception) { return false; }
    }

    static void Add(string kind, string text)
    {
        lock (Events) Events.Add(string.Format("{0,6} ms  {1,-12} {2}", Clock.ElapsedMilliseconds, kind, text));
    }

    static Thread poller;
    static volatile bool polling;

    /// Watches for new Text elements every 50 ms: in this window the Text
    /// elements a screen reader meets are the lines and the live region's
    /// messages, each message a new element.
    public static void StartLivePolling(AutomationElement window)
    {
        polling = true;
        var seen = new HashSet<string>();
        poller = new Thread(delegate ()
        {
            var cond = new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.Text);
            while (polling)
            {
                try
                {
                    foreach (AutomationElement t in window.FindAll(TreeScope.Descendants, cond))
                    {
                        string id = string.Join(".", Array.ConvertAll(t.GetRuntimeId(), x => x.ToString()));
                        if (seen.Add(id) && LiveOf(t) != " live=off")
                            Add("message", Q(t.Current.Name) + LiveOf(t));
                    }
                }
                catch (Exception) { }
                Thread.Sleep(50);
            }
        });
        poller.IsBackground = true;
        poller.Start();
    }

    public static void StopLivePolling() { polling = false; if (poller != null) poller.Join(2000); }

    public static string LiveOf(AutomationElement t)
    {
        int v = IntProperty(t, AutomationElementIdentifiers.LiveSettingProperty);
        return v < 0 ? " live=(unknown)" : (v == 0 ? " live=off" : (v == 1 ? " live=polite" : " live=assertive"));
    }

    static void OnLiveRegion(object sender, AutomationEventArgs e)
    {
        if (!Ours(sender)) return;
        string name = "(unknown)";
        try { name = Q(((AutomationElement)sender).Current.Name); } catch (Exception) { }
        Add("live-changed", name);
    }

    static void OnFocus(object sender, AutomationFocusChangedEventArgs e)
    {
        if (!Ours(sender)) return;
        string name = "(unknown)";
        try { name = Describe((AutomationElement)sender); } catch (Exception) { }
        Add("focus", name);
    }

    public static int IntProperty(AutomationElement e, AutomationProperty prop)
    {
        try
        {
            object v = e.GetCurrentPropertyValue(prop, true);
            if (v is int) return (int)v;
            if (v is Enum) return Convert.ToInt32(v);
            return -1;
        }
        catch (Exception) { return -1; }
    }

    /// Listens desktop-wide before the window starts, so no startup event
    /// is missed.
    public static void Listen()
    {
        try
        {
            Automation.AddAutomationEventHandler(AutomationElementIdentifiers.LiveRegionChangedEvent,
                AutomationElement.RootElement, TreeScope.Subtree, OnLiveRegion);
        }
        catch (Exception) { }
        Automation.AddAutomationFocusChangedEventHandler(OnFocus);
    }

    public static void ListenFor(int pid) { lock (Events) Events.Clear(); listenPid = pid; }

    public static string[] TakeEvents() { lock (Events) return Events.ToArray(); }

    public static string Patterns(AutomationElement e)
    {
        var names = new List<string>();
        foreach (var p in e.GetSupportedPatterns())
            names.Add(p.ProgrammaticName.Replace("PatternIdentifiers.Pattern", ""));
        return string.Join(",", names.ToArray());
    }

    public static string Describe(AutomationElement e)
    {
        var c = e.Current;
        string accel = string.IsNullOrEmpty(c.AcceleratorKey) ? "" : " accel=" + Q(c.AcceleratorKey);
        string access = string.IsNullOrEmpty(c.AccessKey) ? "" : " access=" + Q(c.AccessKey);
        string focus = c.HasKeyboardFocus ? " FOCUSED" : (c.IsKeyboardFocusable ? " focusable" : "");
        string off = c.IsOffscreen ? " offscreen" : "";
        int lv = IntProperty(e, AutomationElementIdentifiers.LiveSettingProperty);
        string live = lv > 0 ? " live=" + (lv == 1 ? "polite" : "assertive") : "";
        string place = "";
        if (c.ControlType == ControlType.ListItem)
        {
            object p;
            bool selected = e.TryGetCurrentPattern(SelectionItemPattern.Pattern, out p)
                && ((SelectionItemPattern)p).Current.IsSelected;
            place = string.Format(" {0} of {1}{2}", IntProperty(e, AutomationElement.PositionInSetProperty),
                IntProperty(e, AutomationElement.SizeOfSetProperty), selected ? ", selected" : "");
        }
        return string.Format("{0} name={1}{2}{3}{4}{5}{6}{7} patterns=[{8}]",
            c.ControlType.ProgrammaticName.Replace("ControlType.", ""), Q(c.Name),
            place, accel, access, focus, off, live, Patterns(e));
    }

    /// The control view, depth first.
    public static List<string> Tree(AutomationElement root)
    {
        var lines = new List<string>();
        Walk(TreeWalker.ControlViewWalker, root, 0, lines);
        return lines;
    }

    static void Walk(TreeWalker w, AutomationElement e, int depth, List<string> lines)
    {
        if (depth > 14 || lines.Count > 600) return;
        lines.Add(new string(' ', depth * 2) + "- " + Describe(e));
        var child = w.GetFirstChild(e);
        while (child != null)
        {
            Walk(w, child, depth + 1, lines);
            child = w.GetNextSibling(child);
        }
    }

    /// Whether the desktop's control view lists the window `hwnd` (what a
    /// screen reader's object navigation from the desktop meets).
    public static string OnDesktop(IntPtr hwnd)
    {
        try
        {
            var w = TreeWalker.ControlViewWalker;
            var child = w.GetFirstChild(AutomationElement.RootElement);
            while (child != null)
            {
                if (new IntPtr(child.Current.NativeWindowHandle) == hwnd)
                    return "yes, as " + Describe(child);
                child = w.GetNextSibling(child);
            }
            return "no";
        }
        catch (Exception ex) { return "(could not read: " + ex.Message + ")"; }
    }

    /// The list boxes and their options.
    public static List<string> Lists(AutomationElement window)
    {
        var lines = new List<string>();
        var lists = window.FindAll(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.List));
        foreach (AutomationElement list in lists)
        {
            var options = list.FindAll(TreeScope.Children,
                new PropertyCondition(AutomationElement.ControlTypeProperty, ControlType.ListItem));
            int size = options.Count > 0 ? IntProperty(options[0], AutomationElement.SizeOfSetProperty) : -1;
            lines.Add(string.Format("List {0}: {1} options in the tree, the set size {2}", Q(list.Current.Name), options.Count, size));
            foreach (AutomationElement o in options)
                lines.Add("  " + Describe(o));
        }
        return lines;
    }
}
'@
Add-Type -TypeDefinition $source -ReferencedAssemblies UIAutomationClient, UIAutomationTypes, WindowsBase

$report = New-Object System.Collections.Generic.List[string]
function Say([string] $line) { $report.Add($line) }
function Fence([string[]] $lines) { Say '```'; foreach ($l in $lines) { Say $l }; Say '```'; Say '' }

$date = (Get-Date).ToString('dddd, MMMM d, yyyy HH:mm')
Say '# diroxide UI Automation report'
Say ''
Say "Made $date by tools/a11y/uia-report.ps1 on $([Environment]::OSVersion.VersionString)."
Say "Program: $Exe"
Say "Left panel: $Folder"
Say "Right panel: $Other"
Say ''

[DiroUia]::Listen()
$beforePid = [DiroUia]::ForegroundPid()
$arguments = "--gui --background --portable `"$Folder`" `"$Other`""
$procId = [DiroUia]::LaunchInactive($Exe, $arguments, $Folder)
[DiroUia]::ListenFor($procId)
$window = [DiroUia]::WaitForWindow($procId, 30000)
if ($null -eq $window) {
    Say 'Fail: the window did not appear in 30 seconds.'
    $code = [DiroUia]::WaitExit(1000)
    Say "Exit code: $code"
    $report | ForEach-Object { $_ }
    exit 1
}
[DiroUia]::StartLivePolling($window)
$tookForeground = $false
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($sw.ElapsedMilliseconds -lt $SettleMs) {
    if ([DiroUia]::ForegroundPid() -eq [uint32]$procId) { $tookForeground = $true }
    Start-Sleep -Milliseconds 100
}

Say '## The foreground'
Say ''
if ($tookForeground) { Say 'Fail: the window took the foreground at least once.' } else { Say 'Pass: the window never took the foreground.' }
Say ''

Say "## The process's top-level windows"
Say ''
foreach ($hwnd in [DiroUia]::WindowsOf($procId)) {
    $visible = if ([DiroUia]::Visible($hwnd)) { 'visible' } else { 'hidden' }
    Say ("- {0}, class {1}, title {2}; on the desktop's control view: {3}" -f $visible, [DiroUia]::Q([DiroUia]::ClassOf($hwnd)), [DiroUia]::Q([DiroUia]::TitleOf($hwnd)), [DiroUia]::OnDesktop($hwnd))
}
Say ''

Say '## The window, as UI Automation gives it'
Say ''
Fence ([DiroUia]::Tree($window)).ToArray()

Say '## The list boxes and their options'
Say ''
Fence ([DiroUia]::Lists($window)).ToArray()

[DiroUia]::StopLivePolling()
Say '## Events: live regions and focus'
Say ''
$events = [DiroUia]::TakeEvents()
if ($events.Count -eq 0) { Say '(none)'; Say '' } else { Fence $events }

$frame = [DiroUia]::FindFrame($procId)
[DiroUia]::Close($frame)
$code = [DiroUia]::WaitExit(15000)
Say '## Closing'
Say ''
if ($code -eq 0) { Say 'Pass: WM_CLOSE ran the quit rule and the program ended with 0.' } elseif ($code -lt 0) { Say 'Fail: the program did not end within 15 seconds of WM_CLOSE; the report ended it.' } else { Say "Fail: the program ended with $code." }
Say ''
if ($code -lt 0) {
    # The report's own child, ended so no window is left behind.
    Stop-Process -Id $procId -ErrorAction SilentlyContinue
}

if ($Out) {
    $report | Out-File -LiteralPath $Out -Encoding utf8
    "Wrote $Out"
} else {
    $report | ForEach-Object { $_ }
}
