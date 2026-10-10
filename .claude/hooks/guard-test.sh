# Tests for the diroxide guard, adapted from textweaver's suite. Run from Git Bash:
#   bash D:/diroxide/.claude/hooks/guard-test.sh
# Tests for the DRAFT guard in this folder (Wave 8a, W8a-q). The first 36
# cases are the suite in the wiki (claude/files/diroxide-guard/
# guard-test.sh), unchanged except for g=, which points at the draft. Then
# the memory-folder write rule (October 1, 2026), then the new cases.
# Run from Git Bash: bash D:/diroxide-planning/research/guard-fix-draft/guard-test.sh
g=D:/diroxide/.claude/hooks/guard.sh
pass=0; fail=0
t() {
  # A case that takes more than 60 seconds counts as wrong (got=allow).
  out=$(printf '%s' "$2" | timeout 60 bash $g 2>&1)
  got=$(printf '%s' "$out" | grep -o '"permissionDecision":"[a-z]*"' | sed 's/.*:"//;s/"//'); [ -z "$got" ] && got=allow
  if [ "$got" = "$1" ]; then s=ok; pass=$((pass+1)); else s=WRONG; fail=$((fail+1)); fi
  printf '%-5s want=%-5s got=%-5s %s\n' "$s" "$1" "$got" "$(printf '%s' "$2" | sed -E 's/.*"tool_input"://' | cut -c1-110)"
}
H='{"session_id":"s1","transcript_path":"C:\\Users\\x\\t.jsonl","cwd":"D:\\diroxide","permission_mode":"bypassPermissions","hook_event_name":"PreToolUse",'
T='},"tool_use_id":"toolu_1","scratchpad_dir":"C:\\Users\\x\\AppData\\Local\\Temp\\claude\\s","prompt_id":"p"}'
# deny: deletes reaching outside
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf \"C\"$'"'"'\\xef\\x80\\xba'"'"'"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf C:"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf ~/Downloads"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf /c/Users/x/Desktop"'"$T"
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"Remove-Item -Recurse $env:USERPROFILE\\Desktop"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf ../star"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf D:\\star\\wiki"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf D:/diroxide/../star"'"$T"
# deny: writes outside
t deny  "$H"'"tool_name":"Write","tool_input":{"file_path":"C:\\Users\\x\\.claude\\CLAUDE.md","content":"x"'"$T"
t deny  "$H"'"tool_name":"Edit","tool_input":{"file_path":"D:\\star\\notes.md","old_string":"a","new_string":"b"'"$T"
t deny  "$H"'"tool_name":"Write","tool_input":{"file_path":"D:\\diroxide\\..\\star\\x.md","content":"x"'"$T"
# ask: deletes inside, and anything that touches outside
t ask   "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf target/w3e"'"$T"
t ask   "$H"'"tool_name":"Bash","tool_input":{"command":"find target -name x -delete"'"$T"
t allow "$H"'"tool_name":"PowerShell","tool_input":{"command":"Remove-Item -LiteralPath D:\\diroxide\\target\\w3e -Recurse"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"git rm --cached x && cargo build --release"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"git worktree remove .claude/worktrees/agent-x"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"cargo clean"'"$T"
t ask   "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf /d/diroxide/target/w3e"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"ls /c/Users/x/Downloads"'"$T"
t allow "$H"'"tool_name":"Read","tool_input":{"file_path":"C:\\Users\\x\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\History"'"$T"
t allow "$H"'"tool_name":"Grep","tool_input":{"pattern":"x","path":"D:\\star"'"$T"
# allow: normal work inside the two folders
t deny  "$H"'"tool_name":"Write","tool_input":{"file_path":"D:\\recovery\\README.md","content":"C:\\Users was hit; rm -rf ~ is bad"'"$T"
t allow "$H"'"tool_name":"Write","tool_input":{"file_path":"D:\\diroxide\\docs\\x.md","content":"hello"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"cargo test --workspace && git status && curl https://example.com"'"$T"
t allow "$H"'"tool_name":"Read","tool_input":{"file_path":"D:\\diroxide\\Cargo.toml"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"cd /d/diroxide && git log --oneline -3"'"$T"
t allow "$H"'"tool_name":"PowerShell","tool_input":{"command":"Get-ChildItem D:\\recovery"'"$T"
# Rule 1a: escapes and hidden characters in moves, copies and writes.
t ask   "$H"'"tool_name":"Bash","tool_input":{"command":"mv \"C\"$'"'"'\\xef\\x80\\xba'"'"' old"'"$T"
t ask   "$H"'"tool_name":"PowerShell","tool_input":{"command":"Move-Item -LiteralPath \"a\\uf03ab\" b"'"$T"
t ask   "$H"'"tool_name":"Bash","tool_input":{"command":"cp x \"$(printf '"'"'\\x43'"'"')\""'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"mv target/a target/b"'"$T"
t allow "$H"'"tool_name":"PowerShell","tool_input":{"command":"Get-ChildItem | Where-Object Name -match '"'"'\\x41'"'"'"'"$T"
# Docker's container-removal flag is not a delete (approved Wednesday, September 30, 2026)
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"docker compose run --rm -T dev bash -c \"cargo test > /work/x.log\"; tail C:/Users/x/out.txt"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"docker run --rm img && rm -rf C:/x"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"docker rm C:/x"'"$T"
t allow "$H"'"tool_name":"PowerShell","tool_input":{"command":"docker volume rm tw-target-x"'"$T"

# The memory-folder write rule (the owner, October 1, 2026): only .md
# files in diroxide's own memory folder.
t allow "$H"'"tool_name":"Write","tool_input":{"file_path":"C:\\Users\\jon\\.claude\\projects\\D--diroxide\\memory\\notes.md","content":"x"'"$T"
t deny  "$H"'"tool_name":"Write","tool_input":{"file_path":"C:\\Users\\jon\\.claude\\projects\\D--diroxide\\memory\\run.ps1","content":"x"'"$T"
t deny  "$H"'"tool_name":"Write","tool_input":{"file_path":"C:\\Users\\jon\\.claude\\settings.json","content":"x"'"$T"

# --- New in the draft (Wave 8a): delete words only where a command starts.
# The false alarm: a Python heredoc that rewrites a planning file, with
# delete words, D:\recovery, git show, and four backslashes in its text.
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"cd D:/diroxide-planning && git show HEAD:history/tasks.md > /dev/null; python3 - <<'"'"'EOF'"'"'\nimport re\np = r\"D:\\\\diroxide-planning\\\\history\\\\tasks.md\"\ns = open(p, encoding=\"utf-8\").read()\n# rm -rf old notes; del x; Remove-Item C:\\\\Users\nold = \"D:\\\\recovery\\\\\\\\x and \\\\\\\\server\\\\share\"\ns = s.replace(old, \"\")\nopen(p, \"w\", encoding=\"utf-8\").write(s)\nEOF"'"$T"
# A delete word inside quoted text that is not a script.
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"git commit -m \"Remove the rm -rf note about C:/Users\""'"$T"
t allow "$H"'"tool_name":"PowerShell","tool_input":{"command":"git commit -m @'"'"'\nDocs: rm and Remove-Item are never used on C:\\\\Users\n'"'"'@"'"$T"
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"grep -rn \"rm -rf\" docs C:/notes.txt"'"$T"
# A comment is not run.
t allow "$H"'"tool_name":"Bash","tool_input":{"command":"ls C:/x # never rm -rf C:/x"'"$T"
# Real deletes outside the project are still denied, wherever they start.
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"python3 - <<'"'"'EOF'"'"'\nprint(1)\nEOF\nrm -rf C:/Users/x"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"bash <<'"'"'EOF'"'"'\nrm -rf C:/Users/x\nEOF"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"cat <<EOF | sh\nrm -rf ~/Downloads\nEOF"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"cat <<EOF\nrm -rf C:/x"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"bash -c \"rm -rf ~/Downloads\""'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"sh -c '"'"'rm -rf /c/Users/x'"'"'"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"eval \"rm -rf C:/x\""'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"echo \"$(rm -rf C:/x)\""'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"echo $(rm -rf C:/x)"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"echo `rm -rf C:/x`"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"(rm -rf C:/x)"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"sudo rm -rf /c/x"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"X=1 /usr/bin/rm -rf /c/x"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"ls; rm -rf C:"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"ls || rm -rf C:"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"echo hi\n  rm -rf C:"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"find /c/Users/x -name y | xargs -0 rm -f"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"find /c/Users/x -name y -exec rm {} \\;"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"find /c/Users/x -name y -delete"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"cmd /c \"rd /s /q C:\\\\x\""'"$T"
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"powershell -NoProfile -Command \"& { Remove-Item -Recurse C:\\\\x }\""'"$T"
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"Get-ChildItem C:\\\\x | Remove-Item -Recurse"'"$T"
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"Get-ChildItem C:\\\\x | ForEach-Object { Remove-Item $_.FullName }"'"$T"
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"iex \"Remove-Item C:\\\\x\""'"$T"
# Escaped names and hidden characters in a delete are still denied.
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf $'"'"'C\\uf03a'"'"'"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf target/C\\xef\\x80\\xba"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"rm -rf \"C'"$(printf '\357\200\272')"'\""'"$T"
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"Remove-Item -LiteralPath \"C'"$(printf '\357\200\272')"'\" -Recurse"'"$T"
# The C: cases, in every shape.
t deny  "$H"'"tool_name":"PowerShell","tool_input":{"command":"Remove-Item -Recurse -Force C:"'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"cd D:/diroxide && rm -rf \"C:\""'"$T"
t deny  "$H"'"tool_name":"Bash","tool_input":{"command":"git clean -fdx C:/"'"$T"
# Deletes inside the project behave as before.
t ask   "$H"'"tool_name":"Bash","tool_input":{"command":"cd D:/diroxide && rm -rf target/w8a-q/scratch"'"$T"
t allow "$H"'"tool_name":"PowerShell","tool_input":{"command":"powershell -File .claude/tools/safe-remove.ps1 -Root D:\\\\diroxide\\\\target\\\\w8a-q -Name scratch"'"$T"
echo "passed $pass, failed $fail"
