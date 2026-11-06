# Git

This guide covers the git plugin: what the title line and the Git column say inside a repository, the status list, staging and committing, diffs, the log and who changed a line, branches and stashes, fetch, pull and push, clone and new repositories, and the trust rule that keeps a repository you did not make from running programs when you only look at it.

Git support is the git plugin (its id is `git`, its name "Git"), an optional component. Get it from Manage optional components. It runs the `git` program on your computer, version 2.30 or newer, always with a list of arguments and never through a shell; diroxide itself knows nothing about git. Without git the title line says "git not found", and the commands are shown with the reason they cannot run.

What this guide says about the spoken lines comes from their design and from the tests that check each sentence at 40 cells. Nothing here had been heard on a screen reader when it was written; if your experience differs, that is a finding.

## The title line

In a folder inside a repository, the title line says the branch and its state after the folder and the panel, then "(Git)":

- "main, 2 ahead, 3 changed here": the branch, where it stands against its upstream, and how many paths changed in this folder and the folders below it.
- "main, no upstream, 1 changed here": a branch that tracks nothing.
- "main, up to date, no changes here".
- "Detached at a1b2c3d, 3 changed": no branch is checked out.
- "Merge in progress, 2 conflicts": a merge, rebase, cherry-pick, revert or bisect is under way.
- "main, no commits yet, 2 changed": a new repository.
- "git refuses it: dubious ownership": git does not trust the folder's owner (another user's folder, or a drive with no owners). If it is yours, run `git config --global --add safe.directory` with its path, in a terminal; the commands say the same.

"Here" means this folder and below, not the whole repository; the status list counts the whole repository. When the words would pass 33 characters, "here" is left off, so the branch and its state stay inside one Braille line.

The words are taken again each time the folder is listed: a reload, a change the folder's watch sees, or the end of a job. A commit made in a terminal shows after the next reload.

## The Git column

When the panel shows the Git column, each row says its state in words, never by a color or a sign alone:

- "Modified", "Staged", "Staged, modified", "Added", "Added, modified".
- "Deleted", "Deleted, staged", "Renamed", "Renamed, modified", "Type changed".
- "Conflict, both modified", and the other conflict words: "both added", "both deleted", "added by us", "deleted by them".
- "Untracked", "Ignored", "Clean".
- "Submodule, new commits", with ", modified" or ", untracked content" when they apply.
- A folder: "3 changed inside", "Untracked", "Ignored" or "Clean".

Outside a repository the column says "Not in a repository".

## The trust rule

A repository you downloaded, copied from a stick or unpacked from an archive brings its own configuration, and plain `git status` runs some programs that configuration names: a file monitor (`core.fsmonitor`) and filter drivers among them. Showing the title line and the column means running git status on every folder you enter, so the plugin reads the repository's own configuration first. Reading it runs nothing.

- If nothing in it runs a program, nothing is asked and status runs.
- If something does, the title line says "Not trusted: its config runs a program" and asks once, starting on No: "Trust this repository?", with what was found ("core.fsmonitor runs ./watch.sh"). The column says "not checked" and never asks.
- Your answer is kept for that repository and that configuration. If the configuration changes, it asks again.
- Even in a trusted repository, the plugin's own status never runs the file monitor, and never takes git's optional locks.
- Commands you start (stage, commit, switch, pull) run as git does in a terminal, the repository's hooks included, but not in a repository you have not trusted.
- To answer again, use Git: trust this repository.

What counts: the file monitor as a program, an editor, pager, askpass or ssh command, a hooks folder outside the repository, an external diff, text converters, filter and merge drivers, an alias that starts with `!`, a credential helper that is a command or a path, a gpg program, remote upload and receive programs, trailer commands, and any include of another file. Only the repository's own configuration counts; your own global and system configuration are yours.

## The commands

The commands are under the Plugins menu and in the palette by name; they have no keys of their own. Bind any of them in `keymap.toml` or the key editor (see [Customizing keys](customizing-keys.md)).

- **Git: status** lists the repository's changed files, each row the state first: "Modified: hello.txt", "Renamed: b.txt, from a.txt". The list's title says the count and the branch: "3 changes, main, 2 ahead". Enter shows a file's changes. Alt+S stages and Alt+U unstages the marked rows.
- **Git: stage** and **Git: unstage** take the marked files, or the rows of the status list. Unstaging keeps the file's changes.
- **Git: stage all changes here** stages every change in this folder and below, new files included.
- **Git: add to .gitignore** adds the marked files to the repository's `.gitignore`, anchored to their place ("/build.log", "/target/"). diroxide writes the file, and asks before it replaces an existing `.gitignore`.
- **Git: discard changes** puts the marked files back as they are staged. It lists them and starts on No: "Discard changes to 3 files?" What was staged stays staged, untracked files are never touched, and the changes go to a stash: "Discarded 3 files, kept in stash 0". Git: pop stash on that stash brings them back.
- **Git: commit** opens a form: the staged files in words first ("2 files staged on main. Modified: hello.txt."), then the message, then Amend the last commit (off) and Sign-off (off). The message goes to git whole, never on a command line. "Committed 2 files on main: a1b2c3d". An empty message commits nothing.

## Diffs, the log and who changed a line

- **Git: show the changes** opens diroxide's compare screens, the verdict first, as for any two files (see [Compare](compare.md)). It compares the staged text with the file when the file has changes not staged, the last commit with the staged text when it has only staged ones, and nothing with the file when it is untracked. In a conflict it compares ours with theirs. The two sides are called "HEAD", "index", "working tree", "ours" and "theirs".
- **Git: compare with the last commit** compares the last commit with the file as it is.
- A binary file is said in words: "Binary file changed: logo.png, 120 KB to 130 KB".
- **Git: log of this folder** and **Git: log of this file** list commits, each row the date and subject first and the commit last: "2026-10-09, Fix the parser, a1b2c3d". Enter on a commit lists its files; Enter on a file compares it with the commit before.
- **Git: who changed a line** asks for the line number, then says "Line 42: Ada Example, Friday, October 9, 2026: Fix the parser, a1b2c3d". The weekday is computed from the commit's own date.

## Branches and stashes

- **Git: branches** lists the local branches: "main, current, 2 ahead", "topic, no upstream". Enter switches. When your changes would be overwritten, git refuses and the plugin says which file: "Cannot switch: hello.txt has local changes". Nothing is ever forced.
- **Git: new branch** asks for a name and switches to the new branch; **Git: rename branch** renames the branch on the row, or the current one.
- **Git: stashes** lists the stashes, the newest first: "Stash 0: half done, on main, 2026-11-06". Enter shows a stash's changes as a document.
- **Git: stash changes** asks for a message and puts your changes away.
- **Git: pop stash** brings back the stash on the row, or the newest.
- **Git: drop stash** removes the stash on the row, after a question that starts on No. If the stashes changed since the list was shown, it says so and does nothing.

## Fetch, pull and push

- **Git: fetch** gets the remote's new commits: "Fetched: main is 3 behind origin/main".
- **Git: pull** is fast-forward only: "Pulled 3 commits into main". When both sides have new commits it stops: "Pull stopped: not a fast-forward", and the next step says to merge or rebase in a terminal.
- **Git: push** asks first: "Push 2 commits to origin/main?" It is never forced. When the remote has new work: "Push refused: remote has new work". A branch with no upstream is pushed to `origin` and set to track it.
- **Git: replace remote history** pushes with a lease (`--force-with-lease --force-if-includes`), after two questions that name the branch and start on No: "Replace history of origin/main?", then "Really replace origin/main?" It is refused if the remote changed since your last fetch.

git never asks for a password here. A remote that needs one says "Sign-in needed: git cannot ask here", with the next step: load your key with `ssh-add`, or sign in with your credential manager. The plugin never asks for, sees or keeps a password.

Progress comes from git's own progress lines, in words, at the progress interval: "Cloning: receiving objects, 45 percent".

## New repositories

- **Git: clone into this panel** asks for the address and, if you like, a folder name (plain letters, digits, `-`, `_` and `.`; empty takes the address's own). The clone is a job with its progress. A clone that stops partway hands its half-made folder to diroxide's delete, which lists it and asks before it goes to the trash.
- **Git: make this folder a repository** asks for the first branch ("main"), a `.gitignore` template (None, Rust, Python, Node, System files) and whether to commit the files here now, with the message. Inside another repository's folder it asks first, starting on No.

## Not in this alpha

- The GitHub half (new repositories on GitHub, gists, pull requests, issues, runs and releases) comes in a later alpha.
- Who changed a line asks for the line number; it does not yet read the viewer's cursor line.
- Merging, rebasing and resolving conflicts are left to a terminal; the status list and the title line say when one is under way.
- A repository whose names are not valid text (possible on Linux) is listed with stand-ins, and those files cannot be staged or compared from diroxide.
- The plugin has been checked with git 2.47 and 2.54; git 2.30, the oldest it accepts, was not run.

## See also

- [Compare](compare.md), the screens a diff uses
- [Plugins](plugins.md), to turn a plugin on or off
- [Optional components](components.md), to get the plugin
- [Customizing keys](customizing-keys.md), to give a command a key
- [Privacy](privacy.md)
- [Known limits](known-limits.md)
- [The documentation index](README.md)
