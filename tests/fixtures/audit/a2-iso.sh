#!/bin/bash
# minimal isolations for a2 findings
T=$(mktemp -d); cd "$T" || exit 1
echo "--1 r_input_output creates file--"
printf 'xy' <> brandnew; echo "rc=$?"; cat brandnew; echo "size=$(wc -c < brandnew)"
echo "--2 input dup bad fd--"
echo badfd <&7; echo "rc=$?"
echo "--3 move output word >&5- --"
exec 5>/dev/null
echo moved >&5-; echo "rc=$?"
echo "--4 move input word <&5- --"
exec 5</dev/null
read -u 5 x <&5-; echo "rc=$?"
echo "--5 empty redirect target--"
echo oops > ""; echo "rc=$?"
echo oops2 > "$unsetv"; echo "rc=$?"
echo oops3 >& "$unsetv"; echo "rc=$?"
echo "--6 ambiguous multiword--"
v="a b"
echo multi > $v; echo "rc=$?"
echo "--7 exec var fd then read -u--"
echo hello > f9
exec {vr}< f9
echo "vr=$vr"
read -u $vr line
echo "read=[$line]"
eval "exec $vr<&-"
echo "--8 >&word with digits+letters--"
echo x >&1a; echo "rc=$?"
echo "--9 varassign redirect then echo var--"
{vv}> fout
echo "vv-after-nullcmd=[$vv]"
cat fout 2>/dev/null; echo "rc=$?"
echo DONE
