set -e
trap 'echo E' ERR
false
echo unreachable
