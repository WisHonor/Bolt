#!/bin/sh
# What one hostile 1 MiB .boltignore costs `bolt save` (time, memory), and what nesting it
# costs, since the scan holds one compiled matcher per folder level on its path.
#   sh redteam/attacks2/boltignore-cost.sh [bolt]
set -u
BOLT=${1:-/tmp/t/release/bolt}
export BOLT_USER_NAME=T BOLT_USER_EMAIL=t@example.com BOLT_CONFIG_DIR=/tmp/bc-cfg
mkdir -p "$BOLT_CONFIG_DIR"
M="python3 -I /work/redteam/attacks2/measure.py 300"
gen() { # shape -> ~1 MiB of patterns on stdout
  python3 -I - "$1" <<'EOF'
import sys
shape, out, i = sys.argv[1], [], 0
while sum(map(len, out)) < (1 << 20) - 300:
    if shape == "stars":   out.append("*a*b*c*d*e*f*g*h*i*j*k*l*m*n*o*p*q*r*s*t*u*v*w*x*y*z*%d\n" % i)
    elif shape == "deep":  out.append("**/" * 60 + "x%d\n" % i)
    elif shape == "class": out.append("[a-zA-Z0-9_]" * 20 + "%d\n" % i)
    else:                  out.append("literal-name-%d.txt\n" % i)
    i += 1
sys.stdout.write("".join(out))
EOF
}
for shape in literal stars deep class; do
  d=$(mktemp -d); cd "$d" || exit 1
  "$BOLT" start >/dev/null; echo hi > a.txt
  gen $shape > .boltignore
  printf '%-8s %s patterns, ' $shape "$(wc -l < .boltignore)"
  $M "$d" "$BOLT" save x
  cd / && rm -rf "$d"
done
