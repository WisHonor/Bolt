#!/bin/sh
# T23 probes: repositories planted by ANOTHER account (root). Run from the host:
#   bash redteam/attacks2/planted-root.sh
# Root's steps use `docker exec -u root` (same no-network container, capabilities dropped).
set -u
R() { MSYS_NO_PATHCONV=1 docker exec -u root -w /tmp bolt-redteam sh -c "$1"; }
A() { MSYS_NO_PATHCONV=1 docker exec -w /tmp bolt-redteam sh -c "export BOLT_USER_NAME=V BOLT_USER_EMAIL=v@x BOLT_CONFIG_DIR=/tmp/pv-cfg; mkdir -p /tmp/pv-cfg; $1"; }
B=/tmp/t/release/bolt
R "rm -rf /tmp/.bolt /tmp/pr; mkdir -p /tmp/pr && cd /tmp && BOLT_USER_NAME=Root BOLT_USER_EMAIL=r@x $B start >/dev/null 2>&1; ls -ld /tmp/.bolt"
A "rm -rf /tmp/pv /tmp/pv2 /tmp/pv-cfg/*; mkdir -p /tmp/pv /tmp/pv2; cd /tmp/pv && echo secret > s.txt
   echo '--- history below root-owned /tmp/.bolt:'; $B history 2>&1
   echo '--- save below it (no own project):'; $B save x 2>&1
   echo '--- start below it:'; $B start 2>&1 | head -1; $B save mine >/dev/null 2>&1 && $B history 2>&1 | head -2"
echo '--- root plants an EMPTY .bolt (no format) and a .bolt symlink above the victim:'
R "rm -rf /tmp/.bolt; mkdir /tmp/.bolt; ln -sfn /etc /tmp/pr/.bolt; mkdir -p /tmp/pr/v; chown 1000 /tmp/pr/v"
A "cd /tmp/pv2 && $B history 2>&1; cd /tmp/pr/v && $B history 2>&1; $B start 2>&1 | head -1"
echo '--- victim trusts /tmp/pr/v only; does that trust /tmp/pr (the planted link)?'
R "rm -rf /tmp/.bolt; rm -rf /tmp/pr/v/.bolt"
A "printf \"safe-folders = ['/tmp/pr/v']\n\" > /tmp/pv-cfg/config.toml; mkdir -p /tmp/pr/v2 2>&1; cd /tmp/pr/v && $B history 2>&1"
echo '--- safe-folders entry written with trailing slash, dot-dot and a symlink to /tmp:'
R "cd /tmp && BOLT_USER_NAME=Root BOLT_USER_EMAIL=r@x $B start >/dev/null 2>&1; ls -ld /tmp/.bolt | cut -c1-30"
A "ln -sfn /tmp /tmp/pv-cfg/tmplink; for e in /tmp/ /tmp/pv/.. /tmp/pv-cfg/tmplink; do printf \"safe-folders = ['\$e']\n\" > /tmp/pv-cfg/config.toml; cd /tmp/pv2 && printf '%s -> ' \$e; $B history 2>&1 | head -1; done"
R "rm -rf /tmp/.bolt /tmp/pr"
A "rm -rf /tmp/pv /tmp/pv2 /tmp/pv-cfg"
