set -e
until false; do break; done; echo "REACHED1"
i=0; until [ $((++i)) -ge 2 ]; do continue; done; echo "REACHED2"
