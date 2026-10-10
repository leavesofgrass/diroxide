#!/usr/bin/env bash
# Project guard for every Claude session and agent working in D:/diroxide.
# Copied from diroxide's guard (Saturday, October 10, 2026) with the folder
# allow-list changed; the logic is unchanged.
#
# The owner's rules after Claude deleted most of their user profile on
# Saturday, September 26, 2026:
#   1. Claude may work only in D:/diroxide and D:/diroxide-planning (plus the
#      wiki and the project's memory folder, for records).
#   2. Nothing may ever be written to drive C, or anywhere else outside them.
#   3. Reading elsewhere (D:/worker-5.4.0 for research) is allowed; writing
#      or deleting there is not.
#   4. Deletes happen only inside them, only for this project's own work,
#      and only after listing exactly what will be removed.
#
# Decisions:
#   Write or Edit to a file outside the allowed folders -> deny
#   delete command that names anything outside them     -> deny
#   delete command inside them                          -> allow (think first)
#   delete in Bash                                      -> ask the owner
#   everything else                                     -> allow

input=$(cat)
sq="'"
tool=$(printf '%s' "$input" | grep -o '"tool_name" *: *"[^"]*"' | head -1 | sed -E 's/.*"([^"]*)"$/\1/')

emit() {
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"%s","permissionDecisionReason":"%s"}}\n' "$1" "$2"
  exit 0
}

# Look only at what the tool was asked to do. Claude Code's own fields
# (working folder, transcript, scratchpad) always point at drive C, so drop
# them, then keep only what follows "tool_input".
harness='description|transcript_path|cwd|session_id|hook_event_name|permission_mode|agent_id|agent_type|scratchpad_dir|tool_use_id|prompt_id|effort|level'
body=$(printf '%s' "$input" | sed -E "s#\"($harness)\" *: *\"([^\"\\\\]|\\\\.)*\"##g; s/^.*\"tool_input\" *: *//")

# Tools that only carry text (agent briefs, messages, lookups) cannot run
# or delete anything themselves; the commands their agents run are checked.
case "$tool" in
  Agent|Task|SendMessage|ToolSearch|Skill|TaskStop|AskUserQuestion) exit 0 ;;
esac

# Writes: judge only the destination path.
case "$tool" in
  Write|Edit|NotebookEdit|MultiEdit)
    fp=$(printf '%s' "$body" | grep -oE '"(file_path|notebook_path)" *: *"([^"\\]|\\.)*"' | head -1 | sed -E 's/^"[a-z_]+" *: *"//; s/"$//; s#[\\]+#/#g')
    case "$fp" in
      [dD]:/diroxide/*|/[dD]/diroxide/*) ;;
      # The planning repository (the owner's decision, September 28, 2026).
      [dD]:/diroxide-planning/*|/[dD]/diroxide-planning/*) ;;
      # The owner's Obsidian wiki, for records and lessons only.
      [dD]:/star/wiki/*|/[dD]/star/wiki/*) ;;
      # Claude's own diroxide memory files (the owner, October 1, 2026).
      [cC]:/Users/jon/.claude/projects/D--diroxide/memory/*.md|/[cC]/Users/jon/.claude/projects/D--diroxide/memory/*.md) ;;
      *) emit deny "BLOCKED: writing is allowed only inside D:/diroxide and D:/diroxide-planning. Never write to drive C." ;;
    esac
    printf '%s' "$fp" | grep -q '[.][.]' && emit deny "BLOCKED: a parent-folder step in a write path."
    exit 0 ;;
esac

# Remove the two allowed folders, then see whether any other path is left.
rest=$(printf '%s' "$body" | sed -E \
  -e 's#[dD]:([\\]+|/)+(diroxide-planning|diroxide)([\\/][^[:space:]"'"$sq"';&|<>]*)?##g' \
  -e 's#/[dD]/(diroxide-planning|diroxide)(/[^[:space:]"'"$sq"';&|<>]*)?##g')

outside=""
has() { printf '%s' "$rest" | grep -Eq -- "$1"; }
has '(^|[^A-Za-z])[A-Za-z]:'                          && outside="a drive path outside D:/diroxide and D:/diroxide-planning"
has '(^|[^A-Za-z0-9_.-])/[A-Za-z](/|[^A-Za-z0-9_]|$)' && outside="a Git Bash drive path"
has '(^|[[:space:]"=(:])~'                            && outside="the home folder"
has '[$][{]?(HOME|USERPROFILE|APPDATA|LOCALAPPDATA|TEMP|TMP|PROGRAMDATA|env[:])' && outside="an environment-variable path"
has '%(USERPROFILE|APPDATA|LOCALAPPDATA|HOMEPATH|TEMP|PROGRAMDATA)%' && outside="an environment-variable path"
has '[\\][\\][\\][\\][A-Za-z0-9?.]'                    && outside="a network or device path"
printf '%s' "$body" | grep -Eq '(^|[[:space:]"/\\])[.][.]([[:space:]"/\\]|$)' && outside="a parent-folder step"
printf '%s' "$body" | grep -Eq "[\$][$sq]"            && outside="shell escape quoting in a path"
printf '%s' "$body" | grep -Eq '[\\]x[0-9A-Fa-f]{2}|[\\]u[fF]0[0-9A-Fa-f]{2}' && outside="escaped characters in a path"
printf '%s' "$body" | LC_ALL=C grep -q "$(printf '\357\200')" && outside="hidden characters in a path"

# --- Where a delete word counts (draft, Wave 8a, W8a-q) ---------------------
# Before, a delete word anywhere in the request (in a Python script fed
# through a heredoc, a commit message, a comment) together with any
# outside-looking text anywhere (four backslashes in a script's string)
# was read as a delete outside the project. Now a delete word counts only
# at a command position: the start of the command, or after ; && || | &
# ( ) { } a newline, $( or a backtick, once prefixes such as sudo, env,
# xargs and find's -exec are taken off. Text that is not run is left out
# first: the bodies of heredocs that feed a program other than a shell,
# quoted text that is not a script, and comments. Quoted text after -c,
# -Command, /c, eval, iex or Invoke-Expression is a script and is read as
# commands; so is $( ) inside double quotes, a heredoc on a line that
# names a shell (bash, sh, zsh, pwsh, powershell, cmd), and a heredoc
# with no end line. The checks for outside paths, escapes and hidden
# characters still read the whole request, so a real delete next to an
# outside path anywhere in the request is still denied.

# The command, decoded from its JSON string. Fails when the tool has none.
command_text() {
  printf '%s' "$body" | LC_ALL=C awk '
    BEGIN { RS = "\001" }
    {
      if (!match($0, /"command"[ \t]*:[ \t]*"/)) exit 1
      s = substr($0, RSTART + RLENGTH); n = length(s); out = ""
      for (k = 1; k <= n; k++) {
        c = substr(s, k, 1)
        if (c == "\\") {
          k++; d = substr(s, k, 1)
          if (d == "n" || d == "r") out = out "\n"
          else if (d == "t") out = out "\t"
          else if (d == "u") out = out "\\u"
          else out = out d
        } else if (c == "\"") break
        else out = out c
      }
      printf "%s", out
    }'
}

# The simple commands of a command line, one per line, with what is not
# run left out (see above). The variable bash is 1 when a backslash
# escapes the next character and a backtick runs a command (Bash), 0 for
# PowerShell, where the backtick escapes.
split_prog='
function script_word(w) {
  return w ~ /^(-c|-Command|-command|-C|\/c|\/C|\/k|\/K|eval|iex|Invoke-Expression|-ScriptBlock)$/
}
function shell_line(t) {
  return t ~ /(^|[^A-Za-z0-9_.-])(bash|sh|zsh|dash|ksh|pwsh|powershell|cmd)(\.exe)?([^A-Za-z0-9_.-]|$)/
}
# Command substitutions inside double-quoted text, as commands.
function subst(t,   i, n, c, depth, start, res) {
  res = ""; n = length(t)
  for (i = 1; i <= n; i++) {
    if (substr(t, i, 2) == "$(") {
      depth = 1; start = i + 2; i += 2
      while (i <= n && depth > 0) {
        c = substr(t, i, 1)
        if (c == "(") depth++
        else if (c == ")") depth--
        i++
      }
      res = res "\n" cmds_of(substr(t, start, i - start - 1)) "\n"
      i--
    }
  }
  return res
}
function cmds_of(t,   n, i, c, q, j, e, out, word, lastword, line, body, found, m, delim, ln, hdn, hd, hstrip, hshell, strip) {
  n = length(t); out = ""; word = ""; lastword = ""; line = ""; hdn = 0
  for (i = 1; i <= n; i++) {
    c = substr(t, i, 1)
    if (c == "\\" && bash) {
      word = word substr(t, i + 1, 1); out = out c substr(t, i + 1, 1); line = line c substr(t, i + 1, 1); i++
      continue
    }
    if (c == "`" && !bash) {
      word = word substr(t, i + 1, 1); out = out c substr(t, i + 1, 1); line = line c substr(t, i + 1, 1); i++
      continue
    }
    if (c == " " || c == "\t") {
      if (word != "") lastword = word
      word = ""; out = out c; line = line c
      continue
    }
    if (c == "#" && word == "") {
      while (i < n && substr(t, i + 1, 1) != "\n") i++
      continue
    }
    # A PowerShell here-string, @" or @" at the end of a line.
    if (c == "@" && (substr(t, i + 1, 1) == "\047" || substr(t, i + 1, 1) == "\"") && substr(t, i + 2, 1) == "\n") {
      q = substr(t, i + 1, 1)
      e = index(substr(t, i + 3), "\n" q "@")
      if (e == 0) e = n - i - 1
      body = substr(t, i + 3, e - 1)
      if (script_word(lastword)) out = out "\n" cmds_of(body) "\n"
      else { out = out "Q"; if (q == "\"") out = out subst(body) }
      i = i + 2 + e + 1
      continue
    }
    if (c == "\"" || c == "\047") {
      body = ""; j = i + 1
      while (j <= n) {
        e = substr(t, j, 1)
        if (c == "\"" && ((e == "\\" && bash) || (e == "`" && !bash))) { body = body e substr(t, j + 1, 1); j += 2; continue }
        if (e == c) break
        body = body e; j++
      }
      if (word == "" && script_word(lastword)) out = out "\n" cmds_of(body) "\n"
      else { out = out "Q"; word = word "Q"; line = line "Q"; if (c == "\"") out = out subst(body) }
      i = j
      continue
    }
    # A heredoc: its body starts on the next line.
    if (c == "<" && substr(t, i, 2) == "<<" && substr(t, i + 2, 1) != "<") {
      j = i + 2; strip = 0
      if (substr(t, j, 1) == "-") { strip = 1; j++ }
      while (substr(t, j, 1) == " " || substr(t, j, 1) == "\t") j++
      delim = ""
      while (j <= n && substr(t, j, 1) !~ /[ \t\n;&|<>()]/) { delim = delim substr(t, j, 1); j++ }
      gsub(/["\047\\]/, "", delim)
      if (delim != "") { hdn++; hd[hdn] = delim; hstrip[hdn] = strip }
      out = out " "; i = j - 1
      continue
    }
    if (c == "\n") {
      for (m = 1; m <= hdn; m++) hshell[m] = shell_line(line)
      out = out "\n"; word = ""; lastword = ""; line = ""
      for (m = 1; m <= hdn; m++) {
        body = ""; found = 0
        while (i < n) {
          e = index(substr(t, i + 1), "\n")
          if (e == 0) { ln = substr(t, i + 1); i = n } else { ln = substr(t, i + 1, e - 1); i = i + e }
          q = ln
          if (hstrip[m]) sub(/^\t+/, "", q)
          if (q == hd[m]) { found = 1; break }
          body = body ln "\n"
        }
        if (hshell[m] || !found) out = out cmds_of(body) "\n"
      }
      hdn = 0
      continue
    }
    if (c == "$" && substr(t, i + 1, 1) == "(") {
      out = out "\n"; word = ""; lastword = ""; line = line c; i++
      continue
    }
    if (c ~ /[;&|(){}]/ || (c == "`" && bash)) {
      out = out "\n"; word = ""; lastword = ""; line = line c
      continue
    }
    word = word c; out = out c; line = line c
  }
  # A heredoc on the last line has no body to leave out.
  return out
}
BEGIN { RS = "\001" }
{ printf "%s\n", cmds_of($0) }
'

# Each simple command with its prefixes taken off, so the program it runs
# comes first on its line.
simple_commands() {
  printf '%s' "$ctext" | LC_ALL=C awk -v bash="$1" "$split_prog" |
    sed -E 's/(^|[[:space:]])-(exec|execdir|ok|okdir)([[:space:]]|$)/\n/g' |
    sed -E ':a
s/^[[:space:]]*(sudo|env|command|exec|time|nohup|then|do|else|elif|if|while|until|!|&|[.]|[A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*)([[:space:]]+|$)//
ta
s/^[[:space:]]*xargs([[:space:]]+-[^[:space:]]+)*([[:space:]]+|$)//
ta
s/^[[:space:]]+//'
}

verbs='(rm|rmdir|rd|del|erase|Remove-Item|ri|unlink|shred|Clear-Content|Clear-RecycleBin|Format-Volume|diskpart|cargo clean|git clean|git rm|git worktree remove|git worktree prune|git branch -[dD]|git reset --hard|git checkout --|git restore|git stash (drop|clear)|docker (rm|rmi)|docker (volume|image|system|container|builder) (rm|prune)|robocopy)'
# A program may be named with its folder (/usr/bin/rm).
lead='^([^[:space:]]*[/\\])?'

if ctext=$(command_text); then
  if [ "$tool" = "Bash" ]; then cmds=$(simple_commands 1); else cmds=$(simple_commands 0); fi
  has_verb() { printf '%s\n' "$cmds" | grep -Eqi "$lead$verbs(\.exe)?([^A-Za-z-]|$)"; }
  bash_rm() { printf '%s\n' "$cmds" | grep -Eq "$lead(rm|rmdir|unlink|shred)(\.exe)?([[:space:]]|$)|(^|[[:space:]])-delete([^A-Za-z]|$)"; }
else
  # Tools without a command (lookups, MCP tools): as before, a delete
  # word anywhere.
  has_verb() { printf '%s' "$body" | grep -Eqi "(^|[^A-Za-z])$verbs([^A-Za-z]|$)"; }
  bash_rm() { return 1; }
fi

# File deletes through Bash are a last resort that needs the owner's approval.
# Git Bash rewrites paths before Windows sees them (that is how "C" plus a
# hidden character became C: on September 26). Prefer PowerShell's
# Remove-Item -LiteralPath with a full literal path. Deletes outside the
# project are denied further down whichever shell is used.
if [ "$tool" = "Bash" ] && bash_rm; then
  printf '%s' "$body" | grep -qi 'recup_dir' && emit deny "BLOCKED: D:/recovery/recup_dir holds files recovered by PhotoRec. Claude never deletes there."
  [ -n "$outside" ] && emit deny "BLOCKED: a delete that names $outside. Deletes outside D:/diroxide and D:/diroxide-planning are never allowed."
  emit ask "A file delete in Bash is a last resort and needs the owner's approval (Git Bash rewrites paths). Prefer PowerShell Remove-Item -LiteralPath. Approve only if the exact target was listed first."
fi

# Docker's --rm flag removes the finished container, not a file (the owner's
# approval, Wednesday, September 30, 2026). It is never at a command
# position, so it is not read as the rm delete word; docker rm, docker
# volume rm and the rest stay delete words.
if has_verb; then
  # PhotoRec's recovered files: irreplaceable, never deleted or cleaned by Claude.
  printf '%s' "$body" | grep -qi 'recup_dir' && emit deny "BLOCKED: D:/recovery/recup_dir holds files recovered by PhotoRec. Claude never deletes there."
  [ -n "$outside" ] && emit deny "BLOCKED: a delete that names $outside. Deletes outside D:/diroxide and D:/diroxide-planning are never allowed."
  # Deletes inside the project run without a prompt, so automation keeps
  # working. Claude must still list the exact target first and delete only
  # what the project's own work created and no longer needs.
fi

# Escape sequences and hidden characters in any command that moves,
# renames, copies or writes files: ask the owner (global rule 1a). On
# September 26 an escaped name was translated by Git Bash into C:.
esc=""
printf '%s' "$body" | grep -Eq "[\$][$sq]" && esc=1
printf '%s' "$body" | grep -Eq '[\\]x[0-9A-Fa-f]{2}|[\\]u[fF]0[0-9A-Fa-f]{2}' && esc=1
printf '%s' "$body" | LC_ALL=C grep -q "$(printf '\357\200')" && esc=1
movers='(^|[^A-Za-z-])(mv|cp|Move-Item|Copy-Item|Rename-Item|ren|move|copy|xcopy|robocopy|Set-Content|Out-File|Add-Content|New-Item|tee|ln|install)([^A-Za-z]|$)'
if [ -n "$esc" ] && printf '%s' "$body" | grep -Eqi "$movers"; then
  emit ask "This command moves, copies or writes files and contains an escape sequence or hidden character (global rule 1a). Names like that can be translated into other paths, as on September 26. Approve only if the exact resolved paths were listed first."
fi

# Other commands that touch outside paths run without a prompt (the owner
# does not want to approve every step). Claude must still stay within the
# task and never write or delete outside without the owner's go-ahead.
exit 0
