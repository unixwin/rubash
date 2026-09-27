set -- 'a\\b' 'x\\y'
IFS=''
v=${1-U},${2-U}
echo "assign=[$v]"
echo "arg=[${1-U},${2-U}]"
u=\\z
echo "literal-assign=[$u]"
v2="${1-U}"
echo "quoted-assign=[$v2]"
set -fCu
v3=${1-U}
echo "after-setfCu=[$v3]"
