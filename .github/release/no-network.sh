#!/usr/bin/env bash
# Prints what a program imports from the system, and fails if any of it reaches a network
# or starts another program.
# Usage: no-network.sh <program>
# Exit 0: nothing found. 1: something found (each named). 2: the imports couldn't be read.
set -uo pipefail
set -f

# What the program may not import. A trailing or inner * matches anything.
win_libs="ws2_32 wsock32 mswsock winhttp wininet urlmon dnsapi iphlpapi httpapi websocket webio wldap32 mpr netapi32"
win_calls="CreateProcess* ShellExecute* WinExec"
unix_calls="socket connect bind listen accept accept4 send sendto sendmsg sendmmsg recv recvfrom recvmsg recvmmsg
  getaddrinfo getnameinfo gethostbyname* gethostbyaddr* res_*query res_*search
  fork vfork exec* posix_spawn* system popen"
mac_frameworks="CFNetwork Network SystemConfiguration"

# "a b* c" -> "^(a|b.*|c)$"
regex() {
    local r
    r=$(echo $1 | tr ' ' '|')
    echo "^(${r//\*/.*})\$"
}

cant() { echo "no-network: $1" >&2; exit 2; }

prog=${1:-}
[ -f "$prog" ] || cant "no program at '$prog'"

case "$(uname -s)" in
MINGW* | MSYS* | CYGWIN*)
    vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
    [ -f "$vswhere" ] || cant "vswhere not found"
    dumpbin=$("$vswhere" -latest -products '*' -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe' | tr -d '\r' | head -n 1)
    [ -n "$dumpbin" ] || cant "dumpbin not found"
    # Dash switches and a Windows path, so Git Bash's path rewriting can't get in the way.
    # Each line: "<library> <function>", or "<library> #<ordinal>".
    imports=$("$(cygpath -u "$dumpbin")" -nologo -imports "$(cygpath -w "$prog")" | tr -d '\r' | awk '
        /Section contains the following (delay load )?imports/ { on = 1; next }
        /^  Summary/ { on = 0 }
        on && /^    [^ ]+\.[dD][lL][lL]$/ { lib = $1; next }
        on && lib != "" && $1 == "Ordinal" { print lib, "#" $2; next }
        on && lib != "" && NF == 2 && $1 ~ /^[0-9A-Fa-f]+$/ && $2 != "Characteristics" { print lib, $2 }
    ')
    [ -n "$imports" ] || cant "no imports read from $prog"
    lib_re=$(regex "$win_libs")
    call_re=$(regex "$win_calls")
    bad=$(echo "$imports" | awk -v libs="$lib_re" -v calls="$call_re" '
        { lib = tolower($1); sub(/\.dll$/, "", lib) }
        lib ~ libs || $2 ~ calls')
    ;;
Linux)
    command -v nm > /dev/null || cant "nm not found"
    # Undefined symbols only, versions cut off: "socket@GLIBC_2.2.5" -> "socket".
    imports=$(nm -D --undefined-only "$prog" | awk '{ print $2 }' | sed 's/@.*//') || cant "nm can't read $prog"
    [ -n "$imports" ] || cant "no imports read from $prog"
    bad=$(echo "$imports" | grep -E "$(regex "$unix_calls")")
    ;;
Darwin)
    command -v nm > /dev/null && command -v otool > /dev/null || cant "nm or otool not found"
    syms=$(nm -u "$prog" | sed 's/^_//; s/\$.*//') || cant "nm can't read $prog"
    libs=$(otool -L "$prog" | tail -n +2 | awk '{ print $1 }') || cant "otool can't read $prog"
    [ -n "$syms" ] || cant "no imports read from $prog"
    imports=$(printf '%s\n%s' "$syms" "$libs")
    fw=$(echo $mac_frameworks | tr ' ' '|')
    bad=$( (echo "$syms" | grep -E "$(regex "$unix_calls")"; echo "$libs" | grep -E "/($fw)\.framework/") )
    ;;
*) cant "unknown system $(uname -s)" ;;
esac

echo "$imports"
if [ -n "$bad" ]; then
    echo "no-network: $prog imports what reaches a network or starts a program:" >&2
    echo "$bad" | sed 's/^/  /' >&2
    exit 1
fi
echo "no-network: $prog imports nothing that reaches a network or starts a program"
